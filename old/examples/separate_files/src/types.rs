use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct SendWelcomeEmailInput {
    pub user_id: String,
    pub user_email: String,
}

#[derive(Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub email: String,
    pub name: String,
    pub welcome_email_sent: bool,
}

#[derive(Serialize, Deserialize)]
pub struct WorkflowOutput {
    pub email_sent: bool,
}
