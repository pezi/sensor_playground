//! Wi-Fi transport: UDP discovery on 9133 + HTTPS REST on 9132.
//! Mirrors python/common/wifi_transport.py.
//!
//! The HTTPS side is deliberately hand-rolled on `TcpListener` + rustls
//! (thread per connection): the nodes serve a single GET / route, which
//! does not justify an async framework's dependency tree on a Raspberry Pi.

use crate::Payload;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::Arc;

pub const HTTPS_PORT: u16 = 9132;
pub const WS_PORT: u16 = 9132; // same port, different protocol per node kind
pub const UDP_PORT: u16 = 9133;
pub const SCAN_KEYWORD: &str = "SENSOR_TESTER";

/// A poll callback: the current reading, or None when the read failed.
pub type ReadFn = Box<dyn Fn() -> Option<Payload> + Send + Sync>;

/// Local network IP of the default-route interface (no packet is sent).
fn get_local_ip() -> std::io::Result<String> {
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.connect("8.8.8.8:80")?;
    Ok(sock.local_addr()?.ip().to_string())
}

/// Constant-time API-key comparison. Both sides are hashed first so the
/// comparison length is fixed and the equality check leaks nothing usable.
pub fn key_matches(supplied: &str, api_key: &str) -> bool {
    let a = Sha256::digest(supplied.as_bytes());
    let b = Sha256::digest(api_key.as_bytes());
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Answer SENSOR_TESTER broadcasts with the node identity plus, for
/// pollable nodes, short-key readings from `read_discovery` (None for
/// push/display nodes). Runs forever; individual failures (e.g. an I2C
/// hiccup) never kill the loop.
pub fn run_discovery_listener(
    name: &str,
    hostname: &str,
    port: u16,
    read_discovery: Option<ReadFn>,
) -> std::io::Result<()> {
    let sock = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;
    sock.set_reuse_address(true)?;
    sock.bind(
        &format!("0.0.0.0:{UDP_PORT}")
            .parse::<std::net::SocketAddr>()
            .unwrap()
            .into(),
    )?;
    let sock: UdpSocket = sock.into();
    println!("UDP discovery listening on port {UDP_PORT}");

    let mut buf = [0u8; 1024];
    loop {
        let Ok((n, addr)) = sock.recv_from(&mut buf) else {
            continue;
        };
        let msg = String::from_utf8_lossy(&buf[..n]);
        if msg.trim() != SCAN_KEYWORD {
            continue;
        }

        let ip = match get_local_ip() {
            Ok(ip) => ip,
            Err(err) => {
                println!("Discovery reply failed: {err}");
                continue;
            }
        };
        let mut reply = Payload::new();
        reply.insert("type".into(), json!(name));
        reply.insert("host".into(), json!(hostname));
        reply.insert("ip".into(), json!(ip));
        reply.insert("port".into(), json!(port));
        if let Some(read) = &read_discovery {
            if let Some(data) = read() {
                for (k, v) in data {
                    reply.insert(k, v);
                }
            }
        }
        match serde_json::to_vec(&Value::Object(reply)) {
            Ok(payload) => {
                if let Err(err) = sock.send_to(&payload, addr) {
                    println!("Discovery reply failed: {err}");
                }
            }
            Err(err) => println!("Discovery reply failed: {err}"),
        }
    }
}

fn load_tls_config(cert_path: &str, key_path: &str) -> Result<Arc<rustls::ServerConfig>, String> {
    let certs = rustls_pemfile::certs(&mut std::io::BufReader::new(
        std::fs::File::open(cert_path).map_err(|e| format!("opening {cert_path}: {e}"))?,
    ))
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| format!("parsing {cert_path}: {e}"))?;
    let key = rustls_pemfile::private_key(&mut std::io::BufReader::new(
        std::fs::File::open(key_path).map_err(|e| format!("opening {key_path}: {e}"))?,
    ))
    .map_err(|e| format!("parsing {key_path}: {e}"))?
    .ok_or_else(|| format!("no private key found in {key_path}"))?;

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("TLS config: {e}"))?;
    Ok(Arc::new(config))
}

/// Serve `read()` on GET / over HTTPS with X-Api-Key auth (None -> 503
/// {"error":"sensor_read_failed"}). Blocks forever.
pub fn run_rest_server(
    name: &'static str,
    api_key: String,
    hostname: String,
    read: ReadFn,
    ssl_cert: &str,
    ssl_key: &str,
) -> Result<(), String> {
    let tls_config = load_tls_config(ssl_cert, ssl_key)?;
    let listener = TcpListener::bind(("0.0.0.0", HTTPS_PORT))
        .map_err(|e| format!("binding port {HTTPS_PORT}: {e}"))?;
    println!("HTTPS server on port {HTTPS_PORT}");

    let api_key = Arc::new(api_key);
    let hostname = Arc::new(hostname);
    let read = Arc::new(read);
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let tls_config = tls_config.clone();
        let api_key = api_key.clone();
        let hostname = hostname.clone();
        let read = read.clone();
        std::thread::spawn(move || {
            let _ = handle_client(stream, tls_config, name, &read, &api_key, &hostname);
        });
    }
    Ok(())
}

fn handle_client(
    mut tcp: TcpStream,
    tls_config: Arc<rustls::ServerConfig>,
    name: &str,
    read: &ReadFn,
    api_key: &str,
    hostname: &str,
) -> std::io::Result<()> {
    tcp.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    tcp.set_write_timeout(Some(std::time::Duration::from_secs(5)))?;

    let mut conn = rustls::ServerConnection::new(tls_config)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let mut tls = rustls::Stream::new(&mut conn, &mut tcp);

    // Read the request head (cap 8 KB); the nodes only serve bodyless GETs.
    let mut request = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        if request.len() > 8192 {
            return Ok(());
        }
        let n = tls.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..n]);
    }
    let head = String::from_utf8_lossy(&request);
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    let path = path.split('?').next().unwrap_or_default();

    let supplied_key = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("x-api-key"))
        .map(|(_, value)| value.trim().to_string())
        .unwrap_or_default();

    let (status, body) = if path != "/" {
        ("404 Not Found", None)
    } else if method != "GET" {
        ("405 Method Not Allowed", None)
    } else if !key_matches(&supplied_key, api_key) {
        ("401 Unauthorized", None)
    } else {
        match read() {
            Some(data) => {
                let mut response = Payload::new();
                response.insert("sensor".into(), json!(name));
                response.insert("host".into(), json!(hostname));
                for (k, v) in data {
                    response.insert(k, v);
                }
                (
                    "200 OK",
                    Some(serde_json::to_string(&Value::Object(response)).unwrap()),
                )
            }
            None => (
                "503 Service Unavailable",
                Some(r#"{"error":"sensor_read_failed"}"#.to_string()),
            ),
        }
    };

    let body = body.unwrap_or_default();
    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if !body.is_empty() {
        response.push_str("Content-Type: application/json\r\n");
    }
    response.push_str("\r\n");
    response.push_str(&body);

    tls.write_all(response.as_bytes())?;
    tls.conn.send_close_notify();
    let _ = tls.conn.complete_io(tls.sock);
    Ok(())
}
