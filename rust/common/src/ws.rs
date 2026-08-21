//! WebSocket push server (ws:// on port 9132, X-Api-Key handshake header)
//! for event, streaming and display/actuator nodes.
//! Mirrors python/common/wifi_transport.WsPushServer.

use crate::wifi::{key_matches, WS_PORT};
use crate::Payload;
use serde_json::Value;
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::{Message, WebSocket};

type Client = Arc<Mutex<WebSocket<TcpStream>>>;

/// Push server: broadcast() sends a payload to every connected client;
/// on_connect runs once per new client (e.g. to send the current state);
/// on_message handles each incoming text message.
pub struct WsPushServer {
    api_key: Arc<String>,
    clients: Arc<Mutex<Vec<Client>>>,
    on_connect: Arc<dyn Fn(&mut dyn FnMut(&Payload)) + Send + Sync>,
    on_message: Arc<dyn Fn(&str) -> Option<Payload> + Send + Sync>,
}

impl WsPushServer {
    pub fn new(
        api_key: String,
        on_connect: impl Fn(&mut dyn FnMut(&Payload)) + Send + Sync + 'static,
        on_message: impl Fn(&str) + Send + Sync + 'static,
    ) -> Self {
        Self::new_replying(api_key, on_connect, move |text| {
            on_message(text);
            None
        })
    }

    /// Like `new`, but the message handler answers the client that sent the
    /// message: a returned payload is sent back to that one client (the
    /// display nodes' command ACKs), None sends nothing. Same contract as
    /// the Python server's on_message.
    pub fn new_replying(
        api_key: String,
        on_connect: impl Fn(&mut dyn FnMut(&Payload)) + Send + Sync + 'static,
        on_message: impl Fn(&str) -> Option<Payload> + Send + Sync + 'static,
    ) -> Self {
        Self {
            api_key: Arc::new(api_key),
            clients: Arc::new(Mutex::new(Vec::new())),
            on_connect: Arc::new(on_connect),
            on_message: Arc::new(on_message),
        }
    }

    /// Send a JSON payload to all connected clients.
    pub fn broadcast(&self, payload: &Payload) {
        let message = serde_json::to_string(&Value::Object(payload.clone())).unwrap();
        let mut clients = self.clients.lock().unwrap();
        clients.retain(|client| {
            let mut ws = client.lock().unwrap();
            ws.send(Message::Text(message.clone().into())).is_ok()
        });
    }

    /// Accept clients forever. Blocks the calling thread.
    pub fn listen_and_serve(&self) -> std::io::Result<()> {
        let listener = TcpListener::bind(("0.0.0.0", WS_PORT))?;
        println!("WebSocket server on port {WS_PORT}");

        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let api_key = self.api_key.clone();
            let clients = self.clients.clone();
            let on_connect = self.on_connect.clone();
            let on_message = self.on_message.clone();
            std::thread::spawn(move || {
                // Reject WebSocket handshakes without the correct X-Api-Key.
                let auth = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
                    let supplied = req
                        .headers()
                        .get("x-api-key")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default();
                    if key_matches(supplied, &api_key) {
                        Ok(resp)
                    } else {
                        let mut e = ErrorResponse::new(None);
                        *e.status_mut() = tungstenite::http::StatusCode::UNAUTHORIZED;
                        Err(e)
                    }
                };
                let Ok(ws) = tungstenite::accept_hdr(stream, auth) else {
                    return;
                };
                // Short read timeouts let the reader release the lock so
                // broadcast() from another thread can interleave sends.
                let _ = ws
                    .get_ref()
                    .set_read_timeout(Some(Duration::from_millis(50)));
                let client: Client = Arc::new(Mutex::new(ws));
                clients.lock().unwrap().push(client.clone());
                println!("Client connected");

                {
                    let mut send = |payload: &Payload| {
                        let msg = serde_json::to_string(&Value::Object(payload.clone())).unwrap();
                        let _ = client.lock().unwrap().send(Message::Text(msg.into()));
                    };
                    on_connect(&mut send);
                }

                loop {
                    let result = { client.lock().unwrap().read() };
                    match result {
                        Ok(Message::Text(text)) => {
                            if let Some(reply) = on_message(&text) {
                                let msg =
                                    serde_json::to_string(&Value::Object(reply)).unwrap();
                                if client
                                    .lock()
                                    .unwrap()
                                    .send(Message::Text(msg.into()))
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        }
                        Ok(Message::Close(_)) => break,
                        Ok(_) => {}
                        Err(tungstenite::Error::Io(err))
                            if err.kind() == std::io::ErrorKind::WouldBlock
                                || err.kind() == std::io::ErrorKind::TimedOut =>
                        {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }

                clients
                    .lock()
                    .unwrap()
                    .retain(|c| !Arc::ptr_eq(c, &client));
                println!("Client disconnected");
            });
        }
        Ok(())
    }
}
