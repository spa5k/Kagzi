//! Performance benchmarks for parallel execution
//!
//! Measures:
//! - Parallel vs Sequential execution speedup
//! - Overhead of parallel coordination
//! - Memoization cache performance
//! - Scalability with increasing step count

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use kagzi::{Kagzi, WorkflowContext};
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::time::Duration;
use tokio::runtime::Runtime;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct BenchInput {
    count: usize,
}

// Helper to create a test database connection for benchmarks
async fn setup_bench_db() -> Kagzi {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgres@localhost:5432/kagzi_bench".to_string()
    });
    Kagzi::connect(&database_url).await.unwrap()
}

// Benchmark 1: Parallel vs Sequential Execution
fn bench_parallel_vs_sequential(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let kagzi = rt.block_on(setup_bench_db());

    let mut group = c.benchmark_group("parallel_vs_sequential");

    for size in [3, 5, 10, 20].iter() {
        // Sequential workflow
        async fn sequential_workflow(
            ctx: WorkflowContext,
            input: BenchInput,
        ) -> anyhow::Result<String> {
            let mut results = Vec::new();
            for i in 0..input.count {
                let result: String = ctx
                    .step(&format!("seq-step-{}", i), async move {
                        // Simulate 50ms I/O operation
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        Ok::<_, anyhow::Error>(format!("result-{}", i))
                    })
                    .await?;
                results.push(result);
            }
            Ok(format!("Sequential: {} steps", results.len()))
        }

        // Parallel workflow
        async fn parallel_workflow(
            ctx: WorkflowContext,
            input: BenchInput,
        ) -> anyhow::Result<String> {
            let steps: Vec<(String, Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>>)> = (0..input.count)
                .map(|i| {
                    let step_id = format!("par-step-{}", i);
                    let future: Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>> = Box::pin(async move {
                        // Simulate 50ms I/O operation
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        Ok::<_, anyhow::Error>(format!("result-{}", i))
                    });
                    (step_id, future)
                })
                .collect();

            let results = ctx.parallel_vec("bench-parallel-group", steps).await?;
            Ok(format!("Parallel: {} steps", results.len()))
        }

        // Benchmark sequential
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(
            BenchmarkId::new("sequential", size),
            size,
            |b, &size| {
                b.to_async(&rt).iter(|| async {
                    kagzi
                        .register_workflow(
                            &format!("seq-bench-{}", size),
                            sequential_workflow,
                        )
                        .await;

                    let input = BenchInput { count: size };
                    let handle = kagzi
                        .start_workflow(&format!("seq-bench-{}", size), input)
                        .await
                        .unwrap();

                    let worker = kagzi.create_worker();
                    let worker_handle = tokio::spawn(async move {
                        tokio::select! {
                            _ = worker.start() => {},
                            _ = tokio::time::sleep(Duration::from_secs(30)) => {},
                        }
                    });

                    let result = tokio::time::timeout(Duration::from_secs(60), handle.result())
                        .await
                        .unwrap();
                    worker_handle.abort();

                    black_box(result)
                });
            },
        );

        // Benchmark parallel
        group.bench_with_input(
            BenchmarkId::new("parallel", size),
            size,
            |b, &size| {
                b.to_async(&rt).iter(|| async {
                    kagzi
                        .register_workflow(&format!("par-bench-{}", size), parallel_workflow)
                        .await;

                    let input = BenchInput { count: size };
                    let handle = kagzi
                        .start_workflow(&format!("par-bench-{}", size), input)
                        .await
                        .unwrap();

                    let worker = kagzi.create_worker();
                    let worker_handle = tokio::spawn(async move {
                        tokio::select! {
                            _ = worker.start() => {},
                            _ = tokio::time::sleep(Duration::from_secs(30)) => {},
                        }
                    });

                    let result = tokio::time::timeout(Duration::from_secs(60), handle.result())
                        .await
                        .unwrap();
                    worker_handle.abort();

                    black_box(result)
                });
            },
        );
    }

    group.finish();
}

// Benchmark 2: Memoization Cache Performance
fn bench_memoization_performance(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let kagzi = rt.block_on(setup_bench_db());

    let mut group = c.benchmark_group("memoization");

    async fn cached_workflow(
        ctx: WorkflowContext,
        input: BenchInput,
    ) -> anyhow::Result<String> {
        // First execution will cache
        let steps: Vec<(String, Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>>)> = (0..input.count)
            .map(|i| {
                let step_id = format!("cached-step-{}", i);
                let future: Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>> = Box::pin(async move {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    Ok::<_, anyhow::Error>(format!("result-{}", i))
                });
                (step_id, future)
            })
            .collect();

        let results = ctx.parallel_vec("memo-bench-group", steps).await?;
        Ok(format!("Cached: {} steps", results.len()))
    }

    for size in [10, 50, 100].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(
            BenchmarkId::new("with_cache", size),
            size,
            |b, &size| {
                b.to_async(&rt).iter(|| async {
                    kagzi
                        .register_workflow(&format!("memo-bench-{}", size), cached_workflow)
                        .await;

                    let input = BenchInput { count: size };

                    // First run - populate cache
                    let handle1 = kagzi
                        .start_workflow(&format!("memo-bench-{}", size), input.clone())
                        .await
                        .unwrap();

                    let worker1 = kagzi.create_worker();
                    let worker_handle1 = tokio::spawn(async move {
                        tokio::select! {
                            _ = worker1.start() => {},
                            _ = tokio::time::sleep(Duration::from_secs(10)) => {},
                        }
                    });

                    let _ = tokio::time::timeout(Duration::from_secs(15), handle1.result()).await;
                    worker_handle1.abort();

                    // Second run - use cache (this is what we benchmark)
                    let handle2 = kagzi
                        .start_workflow(&format!("memo-bench-{}", size), input)
                        .await
                        .unwrap();

                    let worker2 = kagzi.create_worker();
                    let worker_handle2 = tokio::spawn(async move {
                        tokio::select! {
                            _ = worker2.start() => {},
                            _ = tokio::time::sleep(Duration::from_secs(10)) => {},
                        }
                    });

                    let result = tokio::time::timeout(Duration::from_secs(15), handle2.result())
                        .await
                        .unwrap();
                    worker_handle2.abort();

                    black_box(result)
                });
            },
        );
    }

    group.finish();
}

// Benchmark 3: Race() Performance
fn bench_race_performance(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let kagzi = rt.block_on(setup_bench_db());

    let mut group = c.benchmark_group("race");

    async fn race_workflow(ctx: WorkflowContext, input: BenchInput) -> anyhow::Result<String> {
        let steps: Vec<(String, Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>>)> = (0..input.count)
            .map(|i| {
                let step_id = format!("race-step-{}", i);
                let delay = if i == 0 { 10 } else { 100 }; // First one wins
                let future: Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>> = Box::pin(async move {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    Ok::<_, anyhow::Error>(format!("result-{}", i))
                });
                (step_id, future)
            })
            .collect();

        let (winner, result) = ctx.race("race-bench-group", steps).await?;
        Ok(format!("Winner: {}, result: {}", winner, result))
    }

    for size in [3, 5, 10].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.to_async(&rt).iter(|| async {
                kagzi
                    .register_workflow(&format!("race-bench-{}", size), race_workflow)
                    .await;

                let input = BenchInput { count: size };
                let handle = kagzi
                    .start_workflow(&format!("race-bench-{}", size), input)
                    .await
                    .unwrap();

                let worker = kagzi.create_worker();
                let worker_handle = tokio::spawn(async move {
                    tokio::select! {
                        _ = worker.start() => {},
                        _ = tokio::time::sleep(Duration::from_secs(10)) => {},
                    }
                });

                let result = tokio::time::timeout(Duration::from_secs(15), handle.result())
                    .await
                    .unwrap();
                worker_handle.abort();

                black_box(result)
            });
        });
    }

    group.finish();
}

// Benchmark 4: Parallel Coordination Overhead
fn bench_parallel_overhead(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let kagzi = rt.block_on(setup_bench_db());

    let mut group = c.benchmark_group("parallel_overhead");

    async fn minimal_parallel_workflow(
        ctx: WorkflowContext,
        input: BenchInput,
    ) -> anyhow::Result<String> {
        let steps: Vec<(String, Pin<Box<dyn std::future::Future<Output = anyhow::Result<i32>> + Send>>)> = (0..input.count)
            .map(|i| {
                let step_id = format!("minimal-step-{}", i);
                let future: Pin<Box<dyn std::future::Future<Output = anyhow::Result<i32>> + Send>> = Box::pin(async move {
                    // No delay - just return immediately
                    Ok::<_, anyhow::Error>(i as i32)
                });
                (step_id, future)
            })
            .collect();

        let results = ctx
            .parallel_vec("overhead-bench-group", steps)
            .await?;
        Ok(format!("Sum: {}", results.iter().sum::<i32>()))
    }

    for size in [10, 50, 100].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            b.to_async(&rt).iter(|| async {
                kagzi
                    .register_workflow(
                        &format!("overhead-bench-{}", size),
                        minimal_parallel_workflow,
                    )
                    .await;

                let input = BenchInput { count: size };
                let handle = kagzi
                    .start_workflow(&format!("overhead-bench-{}", size), input)
                    .await
                    .unwrap();

                let worker = kagzi.create_worker();
                let worker_handle = tokio::spawn(async move {
                    tokio::select! {
                        _ = worker.start() => {},
                        _ = tokio::time::sleep(Duration::from_secs(10)) => {},
                    }
                });

                let result = tokio::time::timeout(Duration::from_secs(15), handle.result())
                    .await
                    .unwrap();
                worker_handle.abort();

                black_box(result)
            });
        });
    }

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(10)
        .measurement_time(Duration::from_secs(30));
    targets = bench_parallel_vs_sequential, bench_memoization_performance, bench_race_performance, bench_parallel_overhead
}
criterion_main!(benches);
