use async_trait::async_trait;

use crate::PortError;

/// A plain-text e-mail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mail {
    pub to: Vec<String>,
    pub subject: String,
    pub text: String,
}

/// Sends e-mail. Shared by the contexts that notify (issues, monitors,
/// projects' test mail).
#[async_trait]
pub trait Mailer: Send + Sync {
    /// Whether sending is set up at all.
    fn configured(&self) -> bool;

    async fn send(&self, mail: Mail) -> Result<(), PortError>;
}
