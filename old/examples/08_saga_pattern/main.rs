use std::env;
use std::time::Duration;

use kagzi::{Context, Kagzi, Worker};
use serde::{Deserialize, Serialize};
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
    let args: Vec<String> = env::args().collect();
    let variant = args.get(1).map(|s| s.as_str()).unwrap_or("saga");

    let server = env::var("KAGZI_SERVER_URL").unwrap_or_else(|_| "http://localhost:50051".into());
    let namespace = env::var("KAGZI_NAMESPACE").unwrap_or_else(|_| "saga".into());

    match variant {
        "saga" => run_saga(&server, &namespace, false, true).await?,
        "partial" => run_saga(&server, &namespace, true, false).await?,
        _ => {
            eprintln!("Usage: cargo run -p examples --example 08_saga_pattern -- [saga|partial]");
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn run_saga(
    server: &str,
    namespace: &str,
    fail_hotel: bool,
    fail_car: bool,
) -> anyhow::Result<()> {
    println!("🔄 Saga Pattern Example - demonstrates compensating transactions\n");

    let mut worker = Worker::new(server)
        .namespace(namespace)
        .workflows([("trip_booking", trip_workflow)])
        .build()
        .await?;

    println!("👷 Worker started");

    let client = Kagzi::connect(server).await?;
    let input = TripRequest {
        destination: "SFO".into(),
        fail_car,
        fail_hotel,
    };
    let run = client
        .start("trip_booking")
        .namespace(namespace)
        .input(&input)?
        .send()
        .await?;

    println!(
        "🚀 Started saga workflow: run={}, fail_car={}, fail_hotel={}",
        run.id, fail_car, fail_hotel
    );
    tokio::spawn(async move { worker.run().await });
    tokio::time::sleep(Duration::from_secs(10)).await;
    println!("✅ Example complete\n");
    Ok(())
}

async fn trip_workflow(mut ctx: Context, request: TripRequest) -> anyhow::Result<TripResult> {
    // Bookings
    let flight = ctx
        .step("book_flight")
        .run(|| book_flight(request.clone()))
        .await?;
    let hotel = ctx
        .step("book_hotel")
        .run(|| book_hotel(request.clone()))
        .await?;

    match ctx.step("book_car").run(|| book_car(request.clone())).await {
        Ok(car) => Ok(TripResult {
            flight_id: Some(flight),
            hotel_id: Some(hotel),
            car_id: Some(car),
            status: "confirmed".into(),
        }),
        Err(err) => {
            println!("Car booking failed, running compensation: error={:?}", err);
            ctx.step("cancel_hotel")
                .run(|| cancel_hotel(hotel.clone()))
                .await?;
            ctx.step("cancel_flight")
                .run(|| cancel_flight(flight.clone()))
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
    println!("cancelled hotel: {}", hotel_id);
    Ok(format!("cancelled-{hotel_id}"))
}

async fn cancel_flight(flight_id: String) -> anyhow::Result<String> {
    sleep(Duration::from_millis(150)).await;
    println!("cancelled flight: {}", flight_id);
    Ok(format!("cancelled-{flight_id}"))
}
