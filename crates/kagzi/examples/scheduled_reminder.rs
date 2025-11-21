//! Example: Scheduled Reminders
//!
//! This example demonstrates:
//! - Using sleep_until for scheduled tasks
//! - Multiple timed reminders in a sequence
//! - Durable sleep that survives restarts
//!
//! Run with: cargo run --example scheduled_reminder

use chrono::{DateTime, Duration, Utc};
use kagzi::{Kagzi, WorkflowContext};
use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Serialize, Deserialize)]
struct ReminderInput {
    user_id: String,
    task_name: String,
    reminder_times: Vec<i64>, // Minutes from now
}

#[derive(Debug, Serialize, Deserialize)]
struct ReminderSent {
    sent_at: DateTime<Utc>,
    reminder_number: usize,
    message: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ReminderResult {
    user_id: String,
    task_name: String,
    reminders_sent: Vec<ReminderSent>,
    total_reminders: usize,
}

/// Scheduled reminder workflow
async fn scheduled_reminder_workflow(
    ctx: WorkflowContext,
    input: ReminderInput,
) -> anyhow::Result<ReminderResult> {
    println!("⏰ Setting up reminders for: {}", input.user_id);
    println!("   Task: {}", input.task_name);
    println!("   Number of reminders: {}", input.reminder_times.len());

    let mut reminders_sent = Vec::new();

    // Send initial notification
    ctx.step("send-initial-notification", async {
        println!("  📧 Sending initial notification...");

        // Simulate sending notification
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        println!(
            "     ✓ Initial notification sent for task: {}",
            input.task_name
        );

        Ok::<_, anyhow::Error>(())
    })
    .await?;

    // Schedule and send reminders
    let total_reminder_count = input.reminder_times.len();
    for (idx, minutes_from_now) in input.reminder_times.iter().enumerate() {
        let reminder_number = idx + 1;
        let wake_time = Utc::now() + Duration::minutes(*minutes_from_now);

        println!(
            "\n  ⏰ Scheduling reminder {} for {}",
            reminder_number,
            wake_time.format("%Y-%m-%d %H:%M:%S UTC")
        );

        // Sleep until the scheduled time
        let sleep_step_id = format!("sleep-until-reminder-{}", reminder_number);
        ctx.sleep_until(&sleep_step_id, wake_time).await?;

        println!("  ⏰ Woke up at scheduled time!");

        // Send the reminder
        let send_step_id = format!("send-reminder-{}", reminder_number);
        let task_name_clone = input.task_name.clone();
        let reminder: ReminderSent = ctx
            .step(&send_step_id, async move {
                let message = format!(
                    "Reminder {}/{}: Don't forget about '{}'!",
                    reminder_number, total_reminder_count, task_name_clone
                );

                println!("  📧 {}", message);

                // Simulate sending reminder
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                println!("     ✓ Reminder sent successfully");

                Ok::<_, anyhow::Error>(ReminderSent {
                    sent_at: Utc::now(),
                    reminder_number,
                    message,
                })
            })
            .await?;

        reminders_sent.push(reminder);
    }

    // Send final completion notification
    let task_name_clone = input.task_name.clone();
    ctx.step("send-completion-notification", async move {
        println!("\n  📧 Sending completion notification...");

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        println!("     ✓ All reminders sent for task: {}", task_name_clone);

        Ok::<_, anyhow::Error>(())
    })
    .await?;

    let result = ReminderResult {
        user_id: input.user_id.clone(),
        task_name: input.task_name.clone(),
        reminders_sent,
        total_reminders: input.reminder_times.len(),
    };

    println!(
        "\n✅ All {} reminders completed for user: {}",
        result.total_reminders, result.user_id
    );

    Ok(result)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,kagzi=debug,sqlx=warn")
        .init();

    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://kagzi:kagzi_dev_password@localhost:5432/kagzi".to_string());

    let kagzi = Kagzi::connect(&database_url).await?;

    kagzi
        .register_workflow("scheduled-reminder", scheduled_reminder_workflow)
        .await;

    // For demo purposes, schedule reminders at short intervals
    // In production, these would be hours or days apart
    let reminder_schedule = ReminderInput {
        user_id: "user_123".to_string(),
        task_name: "Complete project proposal".to_string(),
        reminder_times: vec![
            0, // Immediate (for demo)
            0, // 5 seconds later (simulating minutes)
            0, // 10 seconds later (simulating minutes)
        ],
    };

    println!("Starting scheduled reminder workflow...");
    println!("Note: For demo purposes, reminders are scheduled at short intervals (seconds)");
    println!("In production, use realistic intervals (minutes, hours, days)\n");

    let handle = kagzi
        .start_workflow("scheduled-reminder", reminder_schedule)
        .await?;

    println!("Workflow started: {}\n", handle.run_id());

    // Start worker in background
    let worker = kagzi.create_worker();
    tokio::spawn(async move {
        worker.start().await.expect("Worker failed");
    });

    // Wait for result
    println!("Sending reminders...\n");
    let result_value = handle.result().await?;
    let result: ReminderResult = serde_json::from_value(result_value)?;

    println!("\n=== Reminder Summary ===");
    println!("User: {}", result.user_id);
    println!("Task: {}", result.task_name);
    println!("Total Reminders Sent: {}", result.total_reminders);
    println!("\nReminder Details:");
    for reminder in &result.reminders_sent {
        println!(
            "  [{}] {} - {}",
            reminder.sent_at.format("%H:%M:%S"),
            reminder.reminder_number,
            reminder.message
        );
    }

    Ok(())
}
