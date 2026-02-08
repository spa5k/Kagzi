use std::env;

use kagzi::{Context, Worker};
use serde::{Deserialize, Serialize};

#[path = "common.rs"]
mod common;

#[derive(Debug, Serialize, Deserialize)]
struct HelloInput {
    name: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server = env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());
    let namespace = env::var("KAGZI_NAMESPACE").unwrap_or_else(|_| "default".into());

    let mut worker = Worker::new(&server)
        .namespace(&namespace)
        .retry(common::default_retry())
        .workflows([("hello_workflow", hello_workflow)])
        .build()
        .await?;

    println!(
        "Worker hub starting; waiting for incoming runs: server={}, namespace={}",
        server, namespace
    );
    println!("Note: See other examples for sleep and polling workflows");
    worker.run().await?;
    Ok(())
}

async fn hello_workflow(_ctx: Context, input: HelloInput) -> anyhow::Result<String> {
    Ok(format!("Hello, {}!", input.name))
}
