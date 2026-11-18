//! `logdockd health`: Docker's health check. The image has no curl, so the
//! binary asks its own `/health/live`.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(3);

/// Whether the server listening on `listen` answers `/health/live` with 200.
/// A server bound to all addresses is asked on loopback.
pub fn check(listen: SocketAddr) -> Result<(), String> {
    let address = if listen.ip().is_unspecified() {
        SocketAddr::from((Ipv4Addr::LOCALHOST, listen.port()))
    } else {
        listen
    };
    let mut stream = TcpStream::connect_timeout(&address, TIMEOUT).map_err(|e| format!("{address}: {e}"))?;
    stream.set_read_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(TIMEOUT)).map_err(|e| e.to_string())?;
    stream
        .write_all(b"GET /health/live HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .map_err(|e| e.to_string())?;
    let mut answer = Vec::new();
    let _ = stream.take(4096).read_to_end(&mut answer);
    let status_line = String::from_utf8_lossy(&answer).lines().next().unwrap_or_default().to_owned();
    if status_line.split_whitespace().nth(1) == Some("200") {
        Ok(())
    } else {
        Err(format!("unexpected answer: {status_line:?}"))
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;

    fn answer_once(reply: &'static [u8]) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0u8; 512];
                let _ = stream.read(&mut request);
                let _ = stream.write_all(reply);
            }
        });
        address
    }

    #[test]
    fn passes_on_200_only() {
        assert!(check(answer_once(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")).is_ok());
        let failed = check(answer_once(b"HTTP/1.1 503 Service Unavailable\r\n\r\n")).unwrap_err();
        assert!(failed.contains("503"), "{failed}");
    }

    #[test]
    fn fails_when_nothing_listens() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        assert!(check(address).is_err());
    }
}
