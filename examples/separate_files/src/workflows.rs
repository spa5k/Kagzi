use std::time::Duration;

use kagzi::Context;

use crate::types::{SendWelcomeEmailInput, User, WorkflowOutput};

// Mock functions for demonstration
pub async fn fetch_user_from_db(user_id: &str) -> anyhow::Result<User> {
    // In a real app, this would query your database
    Ok(User {
        id: user_id.to_string(),
        email: format!("user-{}@example.com", user_id),
        name: format!("User {}", user_id),
        welcome_email_sent: false,
    })
}

pub async fn send_welcome_email_to_user(email: &str) -> anyhow::Result<()> {
    // Simulate sending an email
    println!("📧 Sending welcome email to: {}", email);
    tokio::time::sleep(Duration::from_millis(500)).await;
    println!("✅ Email sent successfully to: {}", email);
    Ok(())
}

pub async fn mark_welcome_email_sent(user_id: &str) -> anyhow::Result<()> {
    // In a real app, this would update your database
    println!("💾 Marked welcome email as sent for user: {}", user_id);
    Ok(())
}

// The main workflow function
pub async fn send_welcome_email(
    mut ctx: Context,
    input: SendWelcomeEmailInput,
) -> anyhow::Result<WorkflowOutput> {
    println!(
        "🚀 Starting welcome email workflow for user: {}",
        input.user_id
    );

    // Step 1: Fetch user (memoized - runs once)
    let user = ctx
        .step("fetch-user")
        .run(|| fetch_user_from_db(&input.user_id))
        .await?;

    println!("👤 Fetched user: {}", user.name);

    // Step 2: Send email (runs only if first step succeeded)
    ctx.step("send-email")
        .run(|| send_welcome_email_to_user(&user.email))
        .await?;

    // Step 3: Update user record
    ctx.step("mark-welcome-sent")
        .run(|| mark_welcome_email_sent(&input.user_id))
        .await?;

    println!(
        "✅ Workflow completed successfully for user: {}",
        input.user_id
    );
    Ok(WorkflowOutput { email_sent: true })
}
