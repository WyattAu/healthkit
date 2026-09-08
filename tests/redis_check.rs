// Integration tests drive RedisCheck against a hermetic fake RESP server
// (no live Redis required); unwrap/expect and panicking asserts are the
// test signal here. A live-server smoke test is `#[ignore]`d at the bottom.
#![cfg(feature = "redis")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use healthkit::{HealthRegistry, HealthStatus, RedisCheck};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::thread;
use std::time::Duration;

/// Fake RESP server: parses complete RESP array commands from the byte
/// stream and replies with `reply` to each — sufficient for `PING` (the
/// client handshake may issue additional commands such as `CLIENT SETINFO`,
/// possibly batched into a single write).
fn spawn_fake_redis(reply: &'static [u8]) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf: Vec<u8> = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                while let Some(consumed) = resp_command_len(&buf) {
                    buf.drain(..consumed);
                    if stream
                        .write_all(reply)
                        .and_then(|_| stream.flush())
                        .is_err()
                    {
                        return;
                    }
                }
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            }
        }
    });
    addr
}

/// Byte length of the first complete RESP array command in `buf`
/// (`*<n>\r\n` followed by n bulk strings `$<len>\r\n<data>\r\n`), or `None`
/// if the buffer holds no complete command yet.
fn resp_command_len(buf: &[u8]) -> Option<usize> {
    if buf.is_empty() || buf[0] != b'*' {
        return None;
    }
    let mut i = 1;
    let mut argc: usize = 0;
    while i < buf.len() && buf[i].is_ascii_digit() {
        argc = argc * 10 + (buf[i] - b'0') as usize;
        i += 1;
    }
    if i + 1 >= buf.len() || buf[i] != b'\r' || buf[i + 1] != b'\n' {
        return None;
    }
    i += 2;
    for _ in 0..argc {
        if i >= buf.len() || buf[i] != b'$' {
            return None;
        }
        i += 1;
        let mut arg_len: usize = 0;
        while i < buf.len() && buf[i].is_ascii_digit() {
            arg_len = arg_len * 10 + (buf[i] - b'0') as usize;
            i += 1;
        }
        if i + 1 >= buf.len() || buf[i] != b'\r' || buf[i + 1] != b'\n' {
            return None;
        }
        i += 2 + arg_len + 2;
        if i > buf.len() {
            return None;
        }
    }
    Some(i)
}

#[tokio::test]
async fn healthy_ping_flows_through_readiness() {
    let addr = spawn_fake_redis(b"+PONG\r\n");
    let registry = HealthRegistry::new();
    let r = registry.clone();
    tokio::task::spawn_blocking(move || {
        RedisCheck::new(format!("redis://{addr}"), Duration::from_secs(5), 500)
            .register(&r, "cache");
    })
    .await
    .unwrap();

    let (status, results) = registry.check_readiness().await.unwrap();
    assert_eq!(status, HealthStatus::Healthy);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "cache");
    assert_eq!(results[0].status, HealthStatus::Healthy);
}

#[tokio::test]
async fn error_reply_folds_into_unhealthy_readiness() {
    let addr = spawn_fake_redis(b"-ERR boom\r\n");
    let registry = HealthRegistry::new();
    let r = registry.clone();
    tokio::task::spawn_blocking(move || {
        RedisCheck::new(format!("redis://{addr}"), Duration::from_secs(5), 500)
            .register(&r, "cache");
    })
    .await
    .unwrap();

    let (status, results) = registry.check_readiness().await.unwrap();
    assert_eq!(status, HealthStatus::Unhealthy);
    assert_eq!(results[0].status, HealthStatus::Unhealthy);
}

#[tokio::test]
async fn connection_refused_folds_into_unhealthy_readiness() {
    // Bind and drop to get an addr that refuses connections.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let registry = HealthRegistry::new();
    let r = registry.clone();
    tokio::task::spawn_blocking(move || {
        RedisCheck::new(format!("redis://{addr}"), Duration::from_secs(5), 500)
            .register(&r, "cache");
    })
    .await
    .unwrap();

    let (status, _) = registry.check_readiness().await.unwrap();
    assert_eq!(status, HealthStatus::Unhealthy);
}

/// Live-server smoke test. Ignored by default:
/// `HEALTHKIT_REDIS_URL=redis://localhost cargo test --features redis -- --ignored`
#[tokio::test]
#[ignore = "requires a live Redis server"]
async fn live_redis_reports_healthy() {
    let url = std::env::var("HEALTHKIT_REDIS_URL").unwrap_or("redis://127.0.0.1".to_string());
    let check = RedisCheck::new(url, Duration::from_secs(2), 500);
    assert_eq!(check.check().await.unwrap(), HealthStatus::Healthy);
}
