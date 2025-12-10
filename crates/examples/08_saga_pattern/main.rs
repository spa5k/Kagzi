use kagzi::WorkflowContext;
use serde::{Deserialize, Serialize};
use std::{env, time::Duration};
use tokio::time::sleep;

#[path = "../common.rs"]
mod common;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct TripRequest {
    destination: String,
    fail_car: bool,
    fail_hotel: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct TripResult {
    flight_id: Option<String>,
    hotel_id: Option<String>,
    car_id: Option<String>,
    status: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    common::init_tracing()?;
    let args: Vec<String> = env::args().collect();
    let variant = args.get(1).map(|s| s.as_str()).unwrap_or("saga");

    let server = env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());
    let queue = env::var("KAGZI_TASK_QUEUE").unwrap_or_else(|_| "saga".into());

    match variant {
        "saga" => run_saga(&server, &queue, false, true).await?,
        "partial" => run_saga(&server, &queue, true, false).await?,
        _ => {
            eprintln!("Usage: cargo run -p kagzi --example 08_saga_pattern -- [saga|partial]");
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn run_saga(
    server: &str,
    queue: &str,
    fail_hotel: bool,
    fail_car: bool,
) -> anyhow::Result<()> {
    let mut worker = common::build_worker(server, queue).await?;
    worker.register("trip_booking", trip_workflow);

    let mut client = common::connect_client(server).await?;
    let run = client
        .workflow(
            "trip_booking",
            queue,
            TripRequest {
                destination: "SFO".into(),
                fail_car,
                fail_hotel,
            },
        )
        .await?;

    tracing::info!(%run, fail_car, fail_hotel, "Started saga workflow");
    tokio::spawn(async move { worker.run().await });
    tokio::time::sleep(Duration::from_secs(10)).await;
    Ok(())
}

async fn trip_workflow(
    mut ctx: WorkflowContext,
    request: TripRequest,
) -> anyhow::Result<TripResult> {
    // Bookings
    let flight = ctx
        .run_with_input("book_flight", &request, book_flight(request.clone()))
        .await?;
    let hotel = ctx
        .run_with_input("book_hotel", &request, book_hotel(request.clone()))
        .await?;

    match ctx
        .run_with_input("book_car", &request, book_car(request.clone()))
        .await
    {
        Ok(car) => Ok(TripResult {
            flight_id: Some(flight),
            hotel_id: Some(hotel),
            car_id: Some(car),
            status: "confirmed".into(),
        }),
        Err(err) => {
            tracing::warn!(error = ?err, "Car booking failed, running compensation");
            ctx.run_with_input("cancel_hotel", &hotel, cancel_hotel(hotel.clone()))
                .await?;
            ctx.run_with_input("cancel_flight", &flight, cancel_flight(flight.clone()))
                .await?;
            Ok(TripResult {
                flight_id: Some(flight),
                hotel_id: Some(hotel),
                car_id: None,
                status: "compensated".into(),
            })
        }
    }
}

async fn book_flight(req: TripRequest) -> anyhow::Result<String> {
    sleep(Duration::from_millis(300)).await;
    Ok(format!("flight-{}", req.destination))
}

async fn book_hotel(req: TripRequest) -> anyhow::Result<String> {
    sleep(Duration::from_millis(300)).await;
    if req.fail_hotel {
        anyhow::bail!("hotel service unavailable")
    }
    Ok(format!("hotel-{}", req.destination))
}

async fn book_car(req: TripRequest) -> anyhow::Result<String> {
    sleep(Duration::from_millis(300)).await;
    if req.fail_car {
        anyhow::bail!("car rental sold out")
    }
    Ok(format!("car-{}", req.destination))
}

async fn cancel_hotel(hotel_id: String) -> anyhow::Result<String> {
    sleep(Duration::from_millis(150)).await;
    tracing::info!(%hotel_id, "cancelled hotel");
    Ok(format!("cancelled-{hotel_id}"))
}

async fn cancel_flight(flight_id: String) -> anyhow::Result<String> {
    sleep(Duration::from_millis(150)).await;
    tracing::info!(%flight_id, "cancelled flight");
    Ok(format!("cancelled-{flight_id}"))
}
