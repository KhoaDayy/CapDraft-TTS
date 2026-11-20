//! `UreqTransport` timeouts follow `requests`' `timeout=`: a slow but flowing body is fine,
//! a server that never answers times out (ADR-022).

#![expect(clippy::unwrap_used, reason = "test")]

use capdraft_core::tts::{Transport, UreqTransport};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

/// Serve one request: wait `head_delay`, send the head, then `body` one byte every `gap`.
fn serve_once(head_delay: Duration, body: &'static [u8], gap: Duration) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = s.read(&mut buf);
        std::thread::sleep(head_delay);
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        if s.write_all(head.as_bytes()).is_err() {
            return;
        }
        for b in body {
            std::thread::sleep(gap);
            if s.write_all(&[*b]).and_then(|()| s.flush()).is_err() {
                return;
            }
        }
    });
    format!("http://{addr}/a.mp3")
}

#[test]
fn slow_flowing_body_outlasts_the_timeout() {
    // 3 s of body with a 1 s timeout: the old global timeout failed this
    let url = serve_once(
        Duration::ZERO,
        b"MP3:abcdefghijklmn",
        Duration::from_millis(170),
    );
    let t = Instant::now();
    let r = UreqTransport.get(&url, Duration::from_secs(1)).unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(r.body, b"MP3:abcdefghijklmn");
    assert!(t.elapsed() > Duration::from_secs(2));
}

#[test]
fn silent_server_times_out() {
    let url = serve_once(Duration::from_secs(5), b"x", Duration::ZERO);
    let t = Instant::now();
    let e = UreqTransport.get(&url, Duration::from_secs(1)).unwrap_err();
    assert!(e.contains("timeout"), "{e}");
    assert!(t.elapsed() < Duration::from_secs(4), "{:?}", t.elapsed());
}
