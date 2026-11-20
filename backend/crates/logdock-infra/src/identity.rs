//! Identity's adapters that are no repositories: password hashing, tokens,
//! and the setup code.

use std::sync::Arc;

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash as Phc;
use argon2::password_hash::{Error as HashError, PasswordHasher as _, PasswordVerifier as _};
use async_trait::async_trait;
use logdock_identity::application::ports::{PasswordHasher, Tokens};
use logdock_identity::domain::{PasswordHash, SetupCode, TokenHash};
use logdock_kernel::{PortError, Secret};

use crate::random::{hex, random_bytes, sha256_hex};

/// Argon2id with the library's default parameters (19 MiB, 2 passes, one
/// lane), as OWASP recommends. Hashing runs on the blocking pool, two at a
/// time at most.
pub struct Argon2Hasher {
    /// Verified against for unknown accounts, so they cost as much time.
    dummy: Arc<PasswordHash>,
    running: Arc<tokio::sync::Semaphore>,
}

impl Argon2Hasher {
    pub fn new() -> Result<Self, PortError> {
        Ok(Self {
            dummy: Arc::new(hash(b"logdock dummy password")?),
            running: Arc::new(tokio::sync::Semaphore::new(2)),
        })
    }
}

fn hash(password: &[u8]) -> Result<PasswordHash, PortError> {
    Argon2::default()
        .hash_password(password)
        .map(|phc| PasswordHash::new(phc.to_string()))
        .map_err(|error| PortError::new("password hashing", error))
}

fn verify(password: &[u8], hash: &PasswordHash) -> Result<bool, PortError> {
    let phc = Phc::new(hash.as_str()).map_err(|error| PortError::new("stored password hash", error))?;
    match Argon2::default().verify_password(password, &phc) {
        Ok(()) => Ok(true),
        Err(HashError::PasswordInvalid) => Ok(false),
        Err(error) => Err(PortError::new("password verification", error)),
    }
}

async fn blocking<T: Send + 'static>(
    what: &'static str,
    work: impl FnOnce() -> Result<T, PortError> + Send + 'static,
) -> Result<T, PortError> {
    tokio::task::spawn_blocking(work).await.map_err(|error| PortError::new(what, error))?
}

#[async_trait]
impl PasswordHasher for Argon2Hasher {
    async fn hash(&self, password: &Secret) -> Result<PasswordHash, PortError> {
        let password = password.clone();
        let _permit = self.running.acquire().await.map_err(|e| PortError::new("password hashing", e))?;
        blocking("password hashing", move || hash(password.expose().as_bytes())).await
    }

    async fn verify(&self, password: &Secret, stored: Option<&PasswordHash>) -> Result<bool, PortError> {
        let password = password.clone();
        let (hash, real) = match stored {
            Some(hash) => (Arc::new(hash.clone()), true),
            None => (Arc::clone(&self.dummy), false),
        };
        let _permit = self.running.acquire().await.map_err(|e| PortError::new("password verification", e))?;
        let matches =
            blocking("password verification", move || verify(password.expose().as_bytes(), &hash)).await?;
        Ok(real && matches)
    }
}

/// 256-bit tokens from the operating system's random source, as hex.
#[derive(Debug, Default)]
pub struct RandomTokens;

impl Tokens for RandomTokens {
    fn session_token(&self) -> Secret {
        Secret::new(hex(&random_bytes::<32>()))
    }

    fn csrf_token(&self) -> Secret {
        Secret::new(hex(&random_bytes::<32>()))
    }

    /// `ldt_` (Logdock token, for secret scanners) and 160 random bits.
    fn api_token(&self) -> Secret {
        Secret::new(format!("ldt_{}", hex(&random_bytes::<20>())))
    }

    fn hash(&self, token: &Secret) -> TokenHash {
        TokenHash::new(sha256_hex(token.expose()))
    }
}

/// A new setup code such as `K7QX-2M9P`: 8 characters of an alphabet
/// without the ones people confuse (0/O, 1/I/L).
pub fn new_setup_code() -> SetupCode {
    const ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
    let bytes = random_bytes::<8>();
    let chars: String = bytes.iter().map(|b| ALPHABET[usize::from(*b) % ALPHABET.len()] as char).collect();
    SetupCode::new(Secret::new(format!("{}-{}", &chars[..4], &chars[4..])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hashes_are_salted_argon2id_and_verify() {
        let hasher = Argon2Hasher::new().unwrap();
        let password = Secret::new("correct horse battery");
        let first = hasher.hash(&password).await.unwrap();
        let second = hasher.hash(&password).await.unwrap();
        assert!(first.as_str().starts_with("$argon2id$"));
        assert_ne!(first, second, "salted");
        assert!(hasher.verify(&password, Some(&first)).await.unwrap());
        assert!(!hasher.verify(&Secret::new("wrong"), Some(&first)).await.unwrap());
        assert!(!hasher.verify(&password, None).await.unwrap());
        let broken = PasswordHash::new("not a hash".into());
        assert!(hasher.verify(&password, Some(&broken)).await.is_err());
    }

    #[test]
    fn tokens_are_random_and_hashed() {
        let tokens = RandomTokens;
        let token = tokens.session_token();
        assert_eq!(token.expose().len(), 64);
        assert_ne!(token.expose(), tokens.session_token().expose());
        let api = tokens.api_token();
        assert!(api.expose().starts_with("ldt_") && api.expose().len() == 44, "{}", api.expose());
        assert_eq!(tokens.hash(&token).as_str().len(), 64);
        assert_ne!(tokens.csrf_token().expose(), tokens.csrf_token().expose());
        let code = new_setup_code();
        assert_eq!(code.expose().len(), 9);
        assert!(code.matches(&code.expose().to_lowercase()));
    }
}
