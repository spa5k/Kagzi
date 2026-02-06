//! Worker implementation for executing Kagzi workflows.
//!
//! # Architecture Overview
//!
//! The worker polls the Kagzi server for workflow tasks, executes them using
//! registered handler functions, and reports results back to the server.
//!
//! # Type Erasure with BoxFuture
//!
//! Workflow handlers have different input/output types, but we need to store
//! them uniformly in a `HashMap`. We use `BoxFuture` for type erasure:
//!
//! ```rust
//! use std::pin::Pin;
//! type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
//! ```
//!
//! This allows us to:
//! 1. Store handlers with different signatures in the same map
//! 2. Pass them across async boundaries
//! 3. Execute them dynamically based on workflow type
//!
//! The `'static` lifetime is required because handlers are stored in the worker
//! and can be called at any time in the future.
//!
//! # Why Arc?
//!
//! - `Arc<Semaphore>`: Shared across concurrent task executions to limit parallelism
//! - `Arc<WorkflowFn>`: Handlers are cloned into each spawned task
//! - `Arc<AtomicU32>`: Poll failure counter shared between main loop and tasks

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use backon::{BackoffBuilder, ExponentialBuilder};
use kagzi_proto::kagzi::telemetry_service_client::TelemetryServiceClient;
use kagzi_proto::kagzi::worker_service_client::WorkerServiceClient;
use kagzi_proto::kagzi::{
    ClaimTaskRequest, CompleteWorkflowRequest, DeregisterRequest, ErrorCode, FailWorkflowRequest,
    HeartbeatRequest, Payload as ProtoPayload, RegisterRequest, SubscribeWorkRequest,
};
use kagzi_proto::kagzi::{
    ReportWorkerEventsRequest, ReportWorkerSnapshotRequest, TelemetryLevel, WorkerTelemetryEvent,
    WorkerTelemetrySnapshot,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::Semaphore;
use tokio::sync::mpsc;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use tonic::Request;
use tonic::transport::Channel;
use tower::ServiceBuilder;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::BoxFuture;
use crate::context::Context;
use crate::errors::{KagziError, WorkflowPaused};
use crate::propagation::inject_context;
use crate::retry::Retry;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn timestamp_from_ms(ms: i64) -> Option<prost_types::Timestamp> {
    if ms <= 0 {
        return None;
    }
    let seconds = ms / 1000;
    let nanos = ((ms % 1000) * 1_000_000) as i32;
    Some(prost_types::Timestamp { seconds, nanos })
}

fn timestamp_now() -> prost_types::Timestamp {
    let now = chrono::Utc::now();
    prost_types::Timestamp {
        seconds: now.timestamp(),
        nanos: now.timestamp_subsec_nanos() as i32,
    }
}

/// Default maximum number of concurrent workflow executions
const DEFAULT_MAX_CONCURRENT_WORKFLOWS: usize = 100;

/// Default heartbeat interval in seconds
const DEFAULT_HEARTBEAT_INTERVAL_SECS: u64 = 10;

/// Long poll timeout for task polling (slightly longer than server hold time)
const POLL_TIMEOUT_SECS: u64 = 65;

/// Default worker telemetry snapshot interval
const DEFAULT_TELEMETRY_INTERVAL_SECS: u64 = 10;

/// Fallback claim tick in seconds (belt-and-suspenders for lost wakeups)
const FALLBACK_CLAIM_TICK_SECS: u64 = 10;

/// Maximum number of claim attempts per wakeup signal (bounded by available permits)
const DRAIN_CLAIM_BUDGET: usize = 100;

/// Workflow handler function type.
///
/// Wraps user-provided workflow functions with type erasure so they can be
/// stored and executed dynamically. The Arc allows sharing across concurrent
/// task executions.
///
/// # Type Parameters
/// - Input is deserialized from `serde_json::Value`
/// - Output is serialized back to `serde_json::Value`
type WorkflowFn = Box<
    dyn Fn(Context, serde_json::Value) -> BoxFuture<'static, anyhow::Result<serde_json::Value>>
        + Send
        + Sync,
>;

/// Builder for configuring and constructing a Worker
pub struct WorkerBuilder {
    addr: String,
    namespace: String,
    max_concurrent: usize,
    default_retry: Option<Retry>,
    hostname: Option<String>,
    version: Option<String>,
    labels: HashMap<String, String>,
    workflows: Vec<(String, Arc<WorkflowFn>)>,
    telemetry_enabled: bool,
    telemetry_interval: Duration,
}

impl WorkerBuilder {
    pub fn new(addr: impl Into<String>) -> Self {
        Self {
            addr: addr.into(),
            namespace: "default".to_string(),
            max_concurrent: DEFAULT_MAX_CONCURRENT_WORKFLOWS,
            default_retry: None,
            hostname: None,
            version: None,
            labels: HashMap::new(),
            workflows: Vec::new(),
            telemetry_enabled: true,
            telemetry_interval: Duration::from_secs(DEFAULT_TELEMETRY_INTERVAL_SECS),
        }
    }

    pub fn namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace = ns.into();
        self
    }

    pub fn max_concurrent(mut self, n: usize) -> Self {
        self.max_concurrent = n;
        self
    }

    /// Simple retry count with default exponential backoff
    pub fn retries(mut self, n: u32) -> Self {
        self.default_retry = Some(Retry::exponential(n));
        self
    }

    /// Full retry configuration
    pub fn retry(mut self, r: Retry) -> Self {
        self.default_retry = Some(r);
        self
    }

    pub fn hostname(mut self, h: impl Into<String>) -> Self {
        self.hostname = Some(h.into());
        self
    }

    pub fn version(mut self, v: impl Into<String>) -> Self {
        self.version = Some(v.into());
        self
    }

    pub fn label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.labels.insert(key.into(), value.into());
        self
    }

    /// Enable/disable worker telemetry reporting to the server.
    pub fn telemetry_enabled(mut self, enabled: bool) -> Self {
        self.telemetry_enabled = enabled;
        self
    }

    /// Set the telemetry snapshot interval (default: 10s).
    pub fn telemetry_interval(mut self, interval: Duration) -> Self {
        self.telemetry_interval = interval;
        self
    }

    pub fn workflows<I, F, Fut, In, Out>(mut self, workflows: I) -> Self
    where
        I: IntoIterator<Item = (&'static str, F)>,
        F: Fn(Context, In) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = anyhow::Result<Out>> + Send + 'static,
        In: DeserializeOwned + Send + 'static,
        Out: Serialize + Send + 'static,
    {
        for (name, handler) in workflows {
            let workflow_name = name.to_string();
            let wrapped =
                move |ctx: Context,
                      input_val: serde_json::Value|
                      -> BoxFuture<'static, anyhow::Result<serde_json::Value>> {
                    let workflow_name = workflow_name.clone();
                    match serde_json::from_value::<In>(input_val) {
                        Ok(input) => {
                            let fut = handler(ctx, input);
                            Box::pin(async move {
                                let output = fut.await?;
                                Ok(serde_json::to_value(output)?)
                            })
                        }
                        Err(e) => Box::pin(async move {
                            Err(anyhow::anyhow!(
                                "Failed to deserialize workflow '{}' input: {e}",
                                workflow_name,
                            ))
                        }),
                    }
                };
            self.workflows
                .push((name.to_string(), Arc::new(Box::new(wrapped))));
        }
        self
    }

    /// Validate the worker configuration before building
    ///
    /// # Errors
    /// Returns `anyhow::Error` if the configuration is invalid
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.max_concurrent == 0 {
            anyhow::bail!("max_concurrent must be greater than 0");
        }

        if let Some(ref retry) = self.default_retry {
            retry
                .validate()
                .map_err(|e| anyhow::anyhow!("Invalid retry configuration: {e}"))?;
        }

        for (name, _) in &self.workflows {
            if name.is_empty() {
                anyhow::bail!("Workflow names cannot be empty");
            }
        }

        Ok(())
    }

    /// Build the worker and connect to the server
    ///
    /// # Errors
    /// Returns an error if:
    /// - No workflows are registered
    /// - Configuration is invalid
    /// - Connection to the server fails
    #[tracing::instrument(skip(self))]
    pub async fn build(self) -> anyhow::Result<Worker> {
        self.validate()?;

        if self.workflows.is_empty() {
            anyhow::bail!("At least one workflow must be registered");
        }

        let channel = Channel::from_shared(self.addr.clone())
            .map_err(|e| anyhow::anyhow!("Invalid server address: {e}"))?
            .connect()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to connect to worker service: {e}"))?;

        let channel = ServiceBuilder::new()
            .timeout(Duration::from_secs(POLL_TIMEOUT_SECS))
            .service(channel);

        let client = WorkerServiceClient::new(channel.clone());
        let telemetry_client = TelemetryServiceClient::new(channel);

        // Build workflow map and collect types
        let mut workflow_map = HashMap::new();
        let mut workflow_types = Vec::new();
        for (name, handler) in self.workflows {
            workflow_types.push(name.clone());
            workflow_map.insert(name, handler);
        }

        Ok(Worker {
            client,
            telemetry_client,
            namespace: self.namespace,
            max_concurrent: self.max_concurrent,
            hostname: self.hostname,
            version: self.version,
            labels: self.labels,
            default_retry: self.default_retry,
            workflows: workflow_map,
            workflow_types,
            telemetry_enabled: self.telemetry_enabled,
            telemetry_interval: self.telemetry_interval,
            worker_id: None,
            heartbeat_interval: Duration::from_secs(DEFAULT_HEARTBEAT_INTERVAL_SECS),
            semaphore: Arc::new(Semaphore::new(self.max_concurrent)),
            wakeup_tx: None,
            shutdown: CancellationToken::new(),
            consecutive_poll_failures: Arc::new(AtomicU32::new(0)),
            telemetry_state: Arc::new(WorkerTelemetryState::default()),
            telemetry_event_tx: None,
        })
    }
}

#[derive(Default)]
struct WorkerTelemetryState {
    subscribed: std::sync::atomic::AtomicBool,
    subscription_state: RwLock<String>,
    last_subscribe_ok_at_ms: std::sync::atomic::AtomicI64,
    last_wakeup_at_ms: std::sync::atomic::AtomicI64,
    last_error: Mutex<String>,
    last_error_at_ms: std::sync::atomic::AtomicI64,

    claim_attempts_delta: std::sync::atomic::AtomicU64,
    claim_success_delta: std::sync::atomic::AtomicU64,
    claim_no_task_delta: std::sync::atomic::AtomicU64,
    claim_error_delta: std::sync::atomic::AtomicU64,
    claim_latency_ms: Mutex<Vec<u32>>,
}

pub struct Worker {
    pub(crate) client: WorkerServiceClient<tower::timeout::Timeout<Channel>>,
    pub(crate) telemetry_client: TelemetryServiceClient<tower::timeout::Timeout<Channel>>,
    namespace: String,
    max_concurrent: usize,
    hostname: Option<String>,
    version: Option<String>,
    labels: HashMap<String, String>,
    default_retry: Option<Retry>,
    /// Workflow handlers by type name.
    /// Arc is used to clone handlers into each spawned task for concurrent execution.
    workflows: HashMap<String, Arc<WorkflowFn>>,
    workflow_types: Vec<String>,
    telemetry_enabled: bool,
    telemetry_interval: Duration,
    worker_id: Option<Uuid>,
    heartbeat_interval: Duration,
    /// Semaphore limits concurrent workflow executions.
    /// Arc allows cloning permits into spawned tasks.
    semaphore: Arc<Semaphore>,
    wakeup_tx: Option<mpsc::Sender<()>>,
    shutdown: CancellationToken,
    /// Counter for exponential backoff on poll failures.
    /// Arc allows atomic updates from both main loop and spawned tasks.
    consecutive_poll_failures: Arc<AtomicU32>,
    telemetry_state: Arc<WorkerTelemetryState>,
    telemetry_event_tx: Option<mpsc::Sender<WorkerTelemetryEvent>>,
}

impl Worker {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(addr: &str) -> WorkerBuilder {
        WorkerBuilder::new(addr)
    }

    pub fn worker_id(&self) -> Option<Uuid> {
        self.worker_id
    }

    pub fn is_registered(&self) -> bool {
        self.worker_id.is_some()
    }

    pub fn active_count(&self) -> usize {
        self.max_concurrent - self.semaphore.available_permits()
    }

    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }

    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    fn signal_backend_name(&self) -> String {
        "server".to_string()
    }

    fn make_event(
        worker_id: &str,
        namespace: &str,
        task_queue: &str,
        level: TelemetryLevel,
        event_type: &str,
        message: &str,
        extra: serde_json::Value,
    ) -> WorkerTelemetryEvent {
        WorkerTelemetryEvent {
            worker_id: worker_id.to_string(),
            namespace: namespace.to_string(),
            task_queue: task_queue.to_string(),
            occurred_at: Some(timestamp_now()),
            level: level as i32,
            event_type: event_type.to_string(),
            message: message.to_string(),
            extra_json: serde_json::to_vec(&extra).unwrap_or_default(),
        }
    }

    fn emit_event(
        &self,
        worker_id: &str,
        task_queue: &str,
        level: TelemetryLevel,
        event_type: &str,
        message: &str,
        extra: serde_json::Value,
    ) {
        let Some(tx) = &self.telemetry_event_tx else {
            return;
        };
        let evt = Self::make_event(
            worker_id,
            &self.namespace,
            task_queue,
            level,
            event_type,
            message,
            extra,
        );
        let _ = tx.try_send(evt);
    }

    fn spawn_telemetry_tasks(
        &mut self,
        worker_id: String,
        task_queue: String,
    ) -> Vec<tokio::task::JoinHandle<()>> {
        if !self.telemetry_enabled {
            return Vec::new();
        }

        let (event_tx, mut event_rx) = mpsc::channel::<WorkerTelemetryEvent>(1024);
        self.telemetry_event_tx = Some(event_tx);

        let mut client_events = self.telemetry_client.clone();
        let shutdown_events = self.shutdown.clone();

        let events_handle = tokio::spawn(async move {
            let mut buffer: Vec<WorkerTelemetryEvent> = Vec::with_capacity(256);
            let mut flush_ticker = tokio::time::interval(Duration::from_secs(2));
            flush_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                tokio::select! {
                    _ = shutdown_events.cancelled() => break,
                    _ = flush_ticker.tick() => {
                        if buffer.is_empty() {
                            continue;
                        }
                        let batch = std::mem::take(&mut buffer);
                        let _ = client_events.report_worker_events(Request::new(ReportWorkerEventsRequest {
                            events: batch,
                        })).await;
                    }
                    evt = event_rx.recv() => {
                        let Some(evt) = evt else { break };
                        buffer.push(evt);
                        if buffer.len() >= 250 {
                            let batch = std::mem::take(&mut buffer);
                            let _ = client_events.report_worker_events(Request::new(ReportWorkerEventsRequest {
                                events: batch,
                            })).await;
                        }
                    }
                }
            }

            if !buffer.is_empty() {
                let _ = client_events
                    .report_worker_events(Request::new(ReportWorkerEventsRequest {
                        events: buffer,
                    }))
                    .await;
            }
        });

        let state = self.telemetry_state.clone();
        let mut client_snapshots = self.telemetry_client.clone();
        let shutdown_snapshots = self.shutdown.clone();
        let interval = self.telemetry_interval;
        let namespace = self.namespace.clone();
        let backend = self.signal_backend_name();
        let max_concurrent = self.max_concurrent as u32;
        let semaphore = self.semaphore.clone();
        let worker_id_for_snapshot = worker_id.clone();
        let task_queue_for_snapshot = task_queue.clone();

        let hostname = self.hostname.clone();
        let version = self.version.clone();
        let labels = self.labels.clone();
        let workflow_types = self.workflow_types.clone();
        let extra_json_bytes = serde_json::to_vec(&serde_json::json!({
            "hostname": hostname,
            "pid": std::process::id(),
            "version": version,
            "labels": labels,
            "workflow_types": workflow_types,
        }))
        .unwrap_or_default();

        let snapshots_handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                tokio::select! {
                    _ = shutdown_snapshots.cancelled() => break,
                    _ = ticker.tick() => {
                        let subscribed = state.subscribed.load(Ordering::Relaxed);
                        let subscription_state = state.subscription_state.read().await.clone();
                        let last_subscribe_ok_at = timestamp_from_ms(state.last_subscribe_ok_at_ms.load(Ordering::Relaxed));
                        let last_wakeup_at = timestamp_from_ms(state.last_wakeup_at_ms.load(Ordering::Relaxed));
                        let last_error_at = timestamp_from_ms(state.last_error_at_ms.load(Ordering::Relaxed));
                        let last_error = state.last_error.lock().await.clone();

                        let claim_attempts_delta = state.claim_attempts_delta.swap(0, Ordering::Relaxed);
                        let claim_success_delta = state.claim_success_delta.swap(0, Ordering::Relaxed);
                        let claim_no_task_delta = state.claim_no_task_delta.swap(0, Ordering::Relaxed);
                        let claim_error_delta = state.claim_error_delta.swap(0, Ordering::Relaxed);

                        let mut lat = state.claim_latency_ms.lock().await;
                        let (avg, p95, max) = if lat.is_empty() {
                            (0.0, 0.0, 0.0)
                        } else {
                            let mut vals = lat.clone();
                            vals.sort_unstable();
                            let sum: u64 = vals.iter().map(|v| *v as u64).sum();
                            let avg = sum as f64 / vals.len() as f64;
                            let idx = ((vals.len() as f64) * 0.95).ceil().max(1.0) as usize - 1;
                            let p95 = vals.get(idx).copied().unwrap_or(0) as f64;
                            let max = *vals.last().unwrap_or(&0) as f64;
                            (avg, p95, max)
                        };
                        lat.clear();

                        let in_flight = (max_concurrent as usize)
                            .saturating_sub(semaphore.available_permits()) as u32;

                            let snapshot = WorkerTelemetrySnapshot {
                                worker_id: worker_id_for_snapshot.clone(),
                                namespace: namespace.clone(),
                                task_queue: task_queue_for_snapshot.clone(),
                                observed_at: Some(timestamp_now()),
                                signal_backend: backend.clone(),
                                subscribed,
                                subscription_state,
                                last_subscribe_ok_at,
                                last_wakeup_at,
                                last_error,
                                last_error_at,
                                max_concurrent,
                                in_flight,
                                active_workflows_authoritative: 0,
                                last_claim_at: None,
                                last_claim_result: String::new(),
                                last_claim_error: String::new(),
                                claim_attempts_delta,
                                claim_success_delta,
                                claim_no_task_delta,
                                claim_error_delta,
                                claim_rtt_ms_avg: avg,
                            claim_rtt_ms_p95: p95,
                            claim_rtt_ms_max: max,
                            extra_json: extra_json_bytes.clone(),
                        };

                        let _ = client_snapshots.report_worker_snapshot(Request::new(ReportWorkerSnapshotRequest {
                            snapshot: Some(snapshot),
                        })).await;
                    }
                }
            }
        });

        vec![events_handle, snapshots_handle]
    }

    /// Run the worker main loop
    ///
    /// This method will:
    /// 1. Register the worker with the server
    /// 2. Start a heartbeat task
    /// 3. Subscribe for work wakeups and claim tasks
    /// 4. Handle graceful shutdown
    #[tracing::instrument(skip(self))]
    pub async fn run(&mut self) -> anyhow::Result<()> {
        if self.workflows.is_empty() {
            anyhow::bail!("No workflows registered. Call workflows() before run()");
        }

        // Default queue. (Server will also default if omitted in RegisterRequest.)
        let primary_queue = "default".to_string();

        let resp = self
            .client
            .register(RegisterRequest {
                namespace: self.namespace.clone(),
                task_queue: None,
                workflow_types: self.workflow_types.clone(),
                hostname: self.hostname.clone().unwrap_or_else(|| {
                    hostname::get()
                        .ok()
                        .and_then(|h| h.into_string().ok())
                        .unwrap_or_default()
                }),
                pid: std::process::id() as i32,
                version: self.version.clone().unwrap_or_default(),
                labels: self.labels.clone(),
                queue_concurrency_limit: None,
                workflow_type_concurrency: Vec::new(),
            })
            .await
            .map_err(|e| anyhow::anyhow!(KagziError::from(e)))?
            .into_inner();

        self.worker_id = Some(Uuid::parse_str(&resp.worker_id)?);
        self.heartbeat_interval = Duration::from_secs(resp.heartbeat_interval_secs as u64);

        info!(
            worker_id = %self.worker_id.map(|id| id.to_string()).unwrap_or_else(|| "unregistered".to_string()),
            task_queue = %primary_queue,
            workflow_count = %self.workflow_types.len(),
            "Worker registered"
        );

        let heartbeat_handle = match self.spawn_heartbeat_task() {
            Some(handle) => handle,
            None => {
                warn!("Cannot spawn heartbeat task: worker not registered");
                return Ok(());
            }
        };

        let worker_id = match &self.worker_id {
            Some(id) => id.to_string(),
            None => {
                warn!("Worker not registered, cannot subscribe");
                return Ok(());
            }
        };

        let telemetry_handles =
            self.spawn_telemetry_tasks(worker_id.clone(), primary_queue.clone());
        self.emit_event(
            &worker_id,
            &primary_queue,
            TelemetryLevel::Info,
            "worker_registered",
            "Worker registered",
            serde_json::json!({ "backend": self.signal_backend_name() }),
        );

        let (wakeup_tx, mut wakeups) = mpsc::channel::<()>(256);
        self.wakeup_tx = Some(wakeup_tx.clone());

        // Prime the pump: try a single claim at startup, in case signals were missed.
        let _ = self.drain_claim_and_execute(&primary_queue, 1).await;

        self.spawn_wakeup_task(worker_id.clone(), primary_queue.clone(), wakeup_tx);

        let mut fallback_ticker =
            tokio::time::interval(Duration::from_secs(FALLBACK_CLAIM_TICK_SECS));
        let shutdown = self.shutdown.clone();
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    info!("Worker shutdown signal received");
                    break;
                }
                _ = fallback_ticker.tick() => {
                    let _ = self.drain_claim_and_execute(&primary_queue, 1).await;
                }
                msg = wakeups.recv() => {
                    if msg.is_none() {
                        error!("Wakeup source closed, triggering shutdown");
                        self.shutdown.cancel();
                        break;
                    }
                    let _ = self.drain_claim_and_execute(&primary_queue, DRAIN_CLAIM_BUDGET).await;
                }
            }
        }

        info!(
            active_count = self.active_count(),
            "Draining active workflows..."
        );
        while self.active_count() > 0 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        heartbeat_handle.abort();
        for h in telemetry_handles {
            h.abort();
        }
        if let Some(id) = self.worker_id {
            let _ = self
                .client
                .deregister(DeregisterRequest {
                    worker_id: id.to_string(),
                    drain: false,
                })
                .await;
        }

        info!("Worker deregistered");
        Ok(())
    }

    fn spawn_wakeup_task(&self, worker_id: String, task_queue: String, tx: mpsc::Sender<()>) {
        let telemetry_state = self.telemetry_state.clone();
        let telemetry_event_tx = self.telemetry_event_tx.clone();
        let telemetry_enabled = self.telemetry_enabled;
        let namespace = self.namespace.clone();
        let mut client = self.client.clone();
        let workflow_types = self.workflow_types.clone();

        tokio::spawn(async move {
            let mut backoff = ExponentialBuilder::default()
                .with_min_delay(Duration::from_millis(100))
                .with_max_delay(Duration::from_secs(10))
                .with_jitter()
                .build();

            loop {
                if telemetry_enabled {
                    telemetry_state.subscribed.store(false, Ordering::Relaxed);
                    *telemetry_state.subscription_state.write().await = "connecting".to_string();
                }

                let res = client
                    .subscribe_work(Request::new(SubscribeWorkRequest {
                        namespace: namespace.clone(),
                        worker_id: worker_id.clone(),
                        task_queue: task_queue.clone(),
                        workflow_types: workflow_types.clone(),
                    }))
                    .await;

                let mut stream = match res {
                    Ok(r) => r.into_inner(),
                    Err(e) => {
                        error!(error = %e, "Failed to SubscribeWork, retrying");
                        if telemetry_enabled {
                            *telemetry_state.last_error.lock().await = e.to_string();
                            telemetry_state
                                .last_error_at_ms
                                .store(now_ms(), Ordering::Relaxed);
                            *telemetry_state.subscription_state.write().await = "error".to_string();
                            if let Some(tx_evt) = &telemetry_event_tx {
                                let _ = tx_evt.try_send(Worker::make_event(
                                    &worker_id,
                                    &namespace,
                                    &task_queue,
                                    TelemetryLevel::Error,
                                    "subscribe_error",
                                    "SubscribeWork error",
                                    serde_json::json!({ "error": e.to_string() }),
                                ));
                            }
                        }
                        let d = backoff.next().unwrap_or(Duration::from_secs(10));
                        tokio::time::sleep(d).await;
                        continue;
                    }
                };

                if telemetry_enabled {
                    telemetry_state.subscribed.store(true, Ordering::Relaxed);
                    telemetry_state
                        .last_subscribe_ok_at_ms
                        .store(now_ms(), Ordering::Relaxed);
                    *telemetry_state.subscription_state.write().await = "subscribed".to_string();
                    if let Some(tx_evt) = &telemetry_event_tx {
                        let _ = tx_evt.try_send(Worker::make_event(
                            &worker_id,
                            &namespace,
                            &task_queue,
                            TelemetryLevel::Info,
                            "subscribe_ok",
                            "SubscribeWork established",
                            serde_json::json!({}),
                        ));
                    }
                }

                backoff = ExponentialBuilder::default()
                    .with_min_delay(Duration::from_millis(100))
                    .with_max_delay(Duration::from_secs(10))
                    .with_jitter()
                    .build();

                loop {
                    match stream.message().await {
                        Ok(Some(_)) => {
                            if telemetry_enabled {
                                telemetry_state
                                    .last_wakeup_at_ms
                                    .store(now_ms(), Ordering::Relaxed);
                            }
                            if tx.send(()).await.is_err() {
                                return;
                            }
                        }
                        Ok(None) => {
                            warn!("SubscribeWork closed by server, resubscribing");
                            if telemetry_enabled {
                                telemetry_state.subscribed.store(false, Ordering::Relaxed);
                                *telemetry_state.subscription_state.write().await =
                                    "reconnecting".to_string();
                                if let Some(tx_evt) = &telemetry_event_tx {
                                    let _ = tx_evt.try_send(Worker::make_event(
                                        &worker_id,
                                        &namespace,
                                        &task_queue,
                                        TelemetryLevel::Warn,
                                        "subscribe_closed",
                                        "SubscribeWork stream closed; resubscribing",
                                        serde_json::json!({}),
                                    ));
                                }
                            }
                            break;
                        }
                        Err(e) => {
                            warn!(error = %e, "SubscribeWork error, resubscribing");
                            if telemetry_enabled {
                                *telemetry_state.last_error.lock().await = e.to_string();
                                telemetry_state
                                    .last_error_at_ms
                                    .store(now_ms(), Ordering::Relaxed);
                                telemetry_state.subscribed.store(false, Ordering::Relaxed);
                                *telemetry_state.subscription_state.write().await =
                                    "reconnecting".to_string();
                                if let Some(tx_evt) = &telemetry_event_tx {
                                    let _ = tx_evt.try_send(Worker::make_event(
                                        &worker_id,
                                        &namespace,
                                        &task_queue,
                                        TelemetryLevel::Warn,
                                        "subscribe_error",
                                        "SubscribeWork stream error; resubscribing",
                                        serde_json::json!({ "error": e.to_string() }),
                                    ));
                                }
                            }
                            break;
                        }
                    }
                }
            }
        });
    }

    /// Spawn a background task to send periodic heartbeats to the server
    fn spawn_heartbeat_task(&self) -> Option<tokio::task::JoinHandle<()>> {
        let worker_id = self.worker_id?;
        let mut client = self.client.clone();
        let interval = self.heartbeat_interval;
        let shutdown = self.shutdown.clone();

        Some(tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);

            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = ticker.tick() => {
                        let resp = client.heartbeat(HeartbeatRequest {
                            worker_id: worker_id.to_string(),
                        }).await;

                        match resp {
                            Ok(r) => {
                                let inner = r.into_inner();
                                if inner.should_drain {
                                    info!("Server requested drain");
                                    shutdown.cancel();
                                }
                            }
                            Err(e) => {
                                // If NotFound or FailedPrecondition, the server thinks we're offline
                                // (e.g., due to missed heartbeats during network partition).
                                // Trigger shutdown to prevent double execution when server assigns
                                // our tasks to other workers.
                                if e.code() == tonic::Code::NotFound
                                    || e.code() == tonic::Code::FailedPrecondition
                                {
                                    error!(
                                        error = %e,
                                        "Worker rejected by server (offline), triggering shutdown"
                                    );
                                    shutdown.cancel();
                                } else {
                                    error!(error = %e, "Heartbeat failed");
                                }
                            }
                        }
                    }
                }
            }
        }))
    }

    #[tracing::instrument(skip(self))]
    async fn drain_claim_and_execute(&mut self, task_queue: &str, budget: usize) -> usize {
        let worker_id = match &self.worker_id {
            Some(id) => id.to_string(),
            None => {
                warn!("Worker not registered, skipping claim");
                tokio::time::sleep(Duration::from_secs(1)).await;
                return 0;
            }
        };

        let mut claimed = 0usize;

        for _ in 0..budget {
            let permit = match self.semaphore.clone().try_acquire_owned() {
                Ok(p) => p,
                Err(_) => break, // no permits left
            };

            self.telemetry_state
                .claim_attempts_delta
                .fetch_add(1, Ordering::Relaxed);

            let claim_start = std::time::Instant::now();
            let resp = self
                .client
                .claim_task(Request::new(ClaimTaskRequest {
                    namespace: self.namespace.clone(),
                    worker_id: worker_id.clone(),
                    task_queue: task_queue.to_string(),
                    workflow_types: self.workflow_types.clone(),
                }))
                .await;
            let claim_latency_ms = claim_start.elapsed().as_millis().min(u128::from(u32::MAX));
            self.telemetry_state
                .claim_latency_ms
                .lock()
                .await
                .push(claim_latency_ms as u32);

            match resp {
                Ok(r) => {
                    // Reset failure counter on successful round-trip
                    self.consecutive_poll_failures.store(0, Ordering::Relaxed);
                    let inner = r.into_inner();
                    let Some(result) = inner.result else {
                        drop(permit);
                        break;
                    };

                    match result {
                        kagzi_proto::kagzi::claim_task_response::Result::NoTask(_) => {
                            self.telemetry_state
                                .claim_no_task_delta
                                .fetch_add(1, Ordering::Relaxed);
                            if claimed == 0 && budget > 1 {
                                // Avoid hammering ClaimTask when signals are racing across workers.
                                tokio::time::sleep(Duration::from_millis(25)).await;
                            }
                            drop(permit);
                            break;
                        }
                        kagzi_proto::kagzi::claim_task_response::Result::Task(task) => {
                            self.telemetry_state
                                .claim_success_delta
                                .fetch_add(1, Ordering::Relaxed);
                            claimed += 1;

                            if let Some(handler) = self.workflows.get(&task.workflow_type) {
                                let handler = handler.clone();
                                let client = self.client.clone();
                                let payload = task.input.unwrap_or(ProtoPayload {
                                    data: Vec::new(),
                                    metadata: HashMap::new(),
                                });
                                let input: serde_json::Value =
                                    match serde_json::from_slice(&payload.data) {
                                        Ok(v) => v,
                                        Err(e) => {
                                            warn!(
                                                run_id = %task.run_id,
                                                error = %e,
                                                payload_len = payload.data.len(),
                                                "Failed to deserialize task input, using null"
                                            );
                                            serde_json::Value::Null
                                        }
                                    };
                                let run_id = task.run_id.clone();
                                let default_retry = self.default_retry.clone();
                                let namespace = self.namespace.clone();
                                let wakeup_tx = self.wakeup_tx.clone();

                                tokio::spawn(async move {
                                    let permit = permit;
                                    execute_workflow(
                                        client,
                                        handler,
                                        run_id,
                                        namespace,
                                        input,
                                        default_retry,
                                    )
                                    .await;
                                    drop(permit);
                                    if let Some(tx) = wakeup_tx {
                                        let _ = tx.try_send(());
                                    }
                                });
                            } else {
                                error!(
                                    workflow_type = %task.workflow_type,
                                    "No handler for workflow type"
                                );
                                drop(permit);
                            }
                        }
                    }
                }
                Err(e) => {
                    drop(permit);
                    self.telemetry_state
                        .claim_error_delta
                        .fetch_add(1, Ordering::Relaxed);
                    *self.telemetry_state.last_error.lock().await = e.to_string();
                    self.telemetry_state
                        .last_error_at_ms
                        .store(now_ms(), Ordering::Relaxed);

                    // If NotFound or FailedPrecondition, the server thinks we're offline/draining
                    // or otherwise unauthorized for this queue. Trigger shutdown to prevent
                    // double execution if tasks are reassigned.
                    if e.code() == tonic::Code::NotFound
                        || e.code() == tonic::Code::FailedPrecondition
                    {
                        error!(
                            error = %e,
                            "Claim rejected by server (offline/draining/not registered), triggering shutdown"
                        );
                        self.shutdown.cancel();
                        break;
                    }

                    error!(error = %e, "ClaimTask failed");
                    let _failures = self
                        .consecutive_poll_failures
                        .fetch_add(1, Ordering::Relaxed)
                        + 1;

                    let mut backoff = ExponentialBuilder::default()
                        .with_min_delay(Duration::from_millis(100))
                        .with_max_delay(Duration::from_secs(30))
                        .with_jitter()
                        .build();

                    let backoff_duration = backoff.next().unwrap_or(Duration::from_secs(30));
                    tokio::time::sleep(backoff_duration).await;
                    break;
                }
            }
        }

        claimed
    }
}

/// Execute a workflow task and report the result to the server
#[tracing::instrument(
    name = "workflow_execution",
    skip(client, handler),
    fields(run_id = %run_id, otel.kind = "client")
)]
async fn execute_workflow(
    mut client: WorkerServiceClient<tower::timeout::Timeout<Channel>>,
    handler: Arc<WorkflowFn>,
    run_id: String,
    namespace: String,
    input: serde_json::Value,
    default_retry: Option<Retry>,
) {
    let namespace_for_requests = namespace.clone();
    let ctx = Context {
        client: client.clone(),
        run_id: run_id.clone(),
        namespace,
        default_retry,
    };

    // Execute workflow in a separate task so panics are captured as JoinError
    // instead of crashing the runtime. JoinError is converted into a failure.
    let task = tokio::spawn(async move { handler(ctx, input).await });

    let result = match task.await {
        Ok(r) => r,
        Err(join_err) => {
            error!(error = %join_err, "Workflow panicked");
            Err(anyhow::anyhow!("workflow panicked: {join_err}"))
        }
    };

    match result {
        Ok(output) => {
            let data = match serde_json::to_vec(&output) {
                Ok(bytes) => bytes,
                Err(e) => {
                    error!(error = %e, "Failed to serialize workflow output");
                    return; // Let workflow retry or fail explicitly
                }
            };

            let mut complete_request = Request::new(CompleteWorkflowRequest {
                run_id,
                namespace: namespace_for_requests.clone(),
                output: Some(ProtoPayload {
                    data,
                    metadata: HashMap::new(),
                }),
            });
            inject_context(complete_request.metadata_mut());

            let _ = client.complete_workflow(complete_request).await;
        }
        Err(e) => {
            if e.downcast_ref::<WorkflowPaused>().is_some() {
                info!("Workflow paused (sleeping)");
                return;
            }

            error!(error = %e, "Workflow failed");

            let kagzi_err = e
                .downcast_ref::<KagziError>()
                .cloned()
                .unwrap_or_else(|| KagziError::new(ErrorCode::Internal, e.to_string()));

            let mut fail_request = Request::new(FailWorkflowRequest {
                run_id,
                namespace: namespace_for_requests.clone(),
                error: Some(kagzi_err.to_detail()),
            });
            inject_context(fail_request.metadata_mut());

            let _ = client.fail_workflow(fail_request).await;
        }
    }
}
