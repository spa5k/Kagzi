use std::collections::HashMap;

use kagzi_proto::kagzi::queue_service_client::QueueServiceClient;
use kagzi_proto::kagzi::{
    CreateQueueRequest, GetQueueRequest, ListQueuesRequest, PageRequest, UpdateQueueRequest,
};
use tests::common::TestHarness;

#[tokio::test]
async fn queue_registry_create_get_list_update() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let mut client = QueueServiceClient::connect(harness.server_url.clone()).await?;

    let mut labels = HashMap::new();
    labels.insert("team".to_string(), "payments".to_string());
    labels.insert("lane".to_string(), "realtime".to_string());

    let extra_json = br#"{"owner":"payments","tier":1}"#.to_vec();

    let created = client
        .create_queue(CreateQueueRequest {
            namespace: "default".to_string(),
            task_queue: "high_priority".to_string(),
            display_name: Some("High Priority".to_string()),
            description: Some("Latency-sensitive workflows".to_string()),
            labels: labels.clone(),
            extra_json: extra_json.clone(),
            enabled: Some(true),
        })
        .await?
        .into_inner()
        .queue
        .expect("queue must be present");

    assert_eq!(created.namespace, "default");
    assert_eq!(created.task_queue, "high_priority");
    assert_eq!(created.display_name.as_deref(), Some("High Priority"));
    assert_eq!(
        created.description.as_deref(),
        Some("Latency-sensitive workflows")
    );
    assert_eq!(
        created.labels.get("team").map(String::as_str),
        Some("payments")
    );
    assert_eq!(
        created.labels.get("lane").map(String::as_str),
        Some("realtime")
    );
    assert!(!created.extra_json.is_empty());
    assert!(created.enabled);

    let fetched = client
        .get_queue(GetQueueRequest {
            namespace: "default".to_string(),
            task_queue: "high_priority".to_string(),
        })
        .await?
        .into_inner()
        .queue
        .expect("queue must be present");

    assert_eq!(fetched.task_queue, "high_priority");
    assert_eq!(fetched.display_name.as_deref(), Some("High Priority"));

    let list = client
        .list_queues(ListQueuesRequest {
            namespace: "default".to_string(),
            page: Some(PageRequest {
                page_size: 200,
                page_token: "".to_string(),
                include_total_count: false,
            }),
        })
        .await?
        .into_inner();

    assert!(
        list.queues.iter().any(|q| q.task_queue == "high_priority"),
        "expected list_queues to include created queue"
    );

    let mut new_labels = HashMap::new();
    new_labels.insert("team".to_string(), "payments".to_string());
    new_labels.insert("lane".to_string(), "cpu".to_string());

    let updated = client
        .update_queue(UpdateQueueRequest {
            namespace: "default".to_string(),
            task_queue: "high_priority".to_string(),
            display_name: Some("High Priority (v2)".to_string()),
            description: Some("Updated description".to_string()),
            enabled: Some(false),
            labels: new_labels,
            extra_json: Some(br#"{"owner":"payments","tier":2}"#.to_vec()),
        })
        .await?
        .into_inner()
        .queue
        .expect("queue must be present");

    assert_eq!(updated.display_name.as_deref(), Some("High Priority (v2)"));
    assert_eq!(updated.description.as_deref(), Some("Updated description"));
    assert_eq!(updated.labels.get("lane").map(String::as_str), Some("cpu"));
    assert!(!updated.enabled);

    Ok(())
}

#[tokio::test]
async fn queue_registry_rejects_invalid_name() -> anyhow::Result<()> {
    let harness = TestHarness::new().await;
    let mut client = QueueServiceClient::connect(harness.server_url.clone()).await?;

    let err = client
        .create_queue(CreateQueueRequest {
            namespace: "default".to_string(),
            task_queue: "bad name".to_string(),
            display_name: None,
            description: None,
            labels: Default::default(),
            extra_json: Vec::new(),
            enabled: None,
        })
        .await
        .unwrap_err();

    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    Ok(())
}
