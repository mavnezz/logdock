//! E-mail over SMTP, as `LOGDOCK_SMTP_URL` names it.

use async_trait::async_trait;
use lettre::message::{Mailbox, header::ContentType};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use logdock_kernel::{Mail, Mailer, PortError};

fn port(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> PortError {
    PortError::new("e-mail", error)
}

/// [`Mailer`] over SMTP; without a URL it is not configured and sends
/// nothing.
pub struct SmtpMailer {
    transport: Option<AsyncSmtpTransport<Tokio1Executor>>,
    from: Mailbox,
}

#[derive(Debug, thiserror::Error)]
pub enum MailConfigError {
    #[error("LOGDOCK_SMTP_URL: {0}")]
    Url(String),
    #[error("LOGDOCK_MAIL_FROM: {0}")]
    From(String),
}

impl SmtpMailer {
    /// `smtp://user:password@host:587` (STARTTLS when offered, `?tls=required`
    /// to insist), `smtps://…:465` (TLS from the start).
    pub fn new(url: Option<&str>, from: &str) -> Result<Self, MailConfigError> {
        let from: Mailbox =
            from.parse().map_err(|e: lettre::address::AddressError| MailConfigError::From(e.to_string()))?;
        let transport = match url.filter(|u| !u.is_empty()) {
            Some(url) => Some(
                AsyncSmtpTransport::<Tokio1Executor>::from_url(url)
                    .map_err(|e| MailConfigError::Url(e.to_string()))?
                    .build(),
            ),
            None => None,
        };
        Ok(Self { transport, from })
    }
}

#[async_trait]
impl Mailer for SmtpMailer {
    fn configured(&self) -> bool {
        self.transport.is_some()
    }

    async fn send(&self, mail: Mail) -> Result<(), PortError> {
        let Some(transport) = &self.transport else { return Err(port("no SMTP server is configured")) };
        for to in &mail.to {
            let to: Mailbox = to.parse().map_err(port)?;
            let message = Message::builder()
                .from(self.from.clone())
                .to(to)
                .subject(&mail.subject)
                .header(ContentType::TEXT_PLAIN)
                .body(mail.text.clone())
                .map_err(port)?;
            transport.send(message).await.map_err(port)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn is_configured_by_its_url() {
        let none = SmtpMailer::new(None, "Logdock <logdock@example.com>").unwrap();
        assert!(!none.configured());
        let mail = Mail { to: vec!["ops@example.com".into()], subject: "s".into(), text: "t".into() };
        assert!(none.send(mail).await.is_err());
        let some = SmtpMailer::new(Some("smtp://user:secret@127.0.0.1:2525"), "logdock@example.com").unwrap();
        assert!(some.configured());
        assert!(SmtpMailer::new(Some("nonsense://"), "logdock@example.com").is_err());
        assert!(SmtpMailer::new(None, "not an address").is_err());
    }

    /// Against a stand-in SMTP server that accepts everything.
    #[tokio::test]
    async fn sends_through_smtp() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port_number = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut lines = BufReader::new(read).lines();
            write.write_all(b"220 test ESMTP\r\n").await.unwrap();
            let mut received = String::new();
            let mut in_data = false;
            while let Ok(Some(line)) = lines.next_line().await {
                if in_data {
                    if line == "." {
                        in_data = false;
                        write.write_all(b"250 queued\r\n").await.unwrap();
                    } else {
                        received.push_str(&line);
                        received.push('\n');
                    }
                    continue;
                }
                let reply: &[u8] =
                    match line.split_whitespace().next().unwrap_or("").to_ascii_uppercase().as_str() {
                        "EHLO" | "HELO" => b"250 test\r\n",
                        "DATA" => {
                            in_data = true;
                            b"354 go on\r\n"
                        }
                        "QUIT" => {
                            write.write_all(b"221 bye\r\n").await.unwrap();
                            break;
                        }
                        _ => b"250 ok\r\n",
                    };
                write.write_all(reply).await.unwrap();
            }
            received
        });
        let mailer =
            SmtpMailer::new(Some(&format!("smtp://127.0.0.1:{port_number}")), "logdock@example.com").unwrap();
        let mail = Mail {
            to: vec!["ops@example.com".into()],
            subject: "New issue".into(),
            text: "Something broke".into(),
        };
        mailer.send(mail).await.unwrap();
        drop(mailer);
        let received =
            tokio::time::timeout(std::time::Duration::from_secs(5), server).await.unwrap().unwrap();
        assert!(received.contains("Subject: New issue"), "{received}");
        assert!(received.contains("Something broke"));
    }
}
