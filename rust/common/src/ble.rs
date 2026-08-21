//! BLE transport: GATT peripheral via BlueZ (bluer), mirroring
//! python/common/ble_transport.py and the ESP32 sketches' BLE contract:
//!
//! - Service  d1a51b00-0001-...  advertised under the sensor name
//! - Data     d1a51b00-0002-...  READ | NOTIFY, JSON payload (same as REST/WS)
//! - Auth     d1a51b00-0003-...  WRITE, client sends the plain API key
//! - Command  d1a51b00-0004-...  WRITE, app -> node (actuator nodes only)
//!
//! A client connects, writes the API key to the auth characteristic and
//! then reads or subscribes to the data characteristic. Unauthenticated
//! reads return "{}" and no notifications are sent; on disconnect the
//! authentication is cleared.
//!
//! If BlueZ leaves the advertisement off air (Raspberry Pi, see
//! mgmt_advertiser.rs) the node drives the kernel's mgmt socket directly,
//! which requires root.

use crate::ble_framing::frame_payload;
use crate::mgmt_advertiser::{kernel_advertising_active, MgmtAdvertiser};
use crate::wifi::key_matches;
use crate::Payload;
use bluer::adv::Advertisement;
use bluer::gatt::{
    local::{
        characteristic_control, Application, Characteristic, CharacteristicControlEvent,
        CharacteristicNotify, CharacteristicNotifyMethod, CharacteristicRead, CharacteristicWrite,
        CharacteristicWriteMethod, Service,
    },
    CharacteristicWriter,
};
use bluer::Address;
use futures::{FutureExt, StreamExt};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

pub const SERVICE_UUID: &str = "d1a51b00-0001-4a7e-9b3c-0a1b2c3d4e5f";
pub const DATA_CHAR_UUID: &str = "d1a51b00-0002-4a7e-9b3c-0a1b2c3d4e5f";
pub const AUTH_CHAR_UUID: &str = "d1a51b00-0003-4a7e-9b3c-0a1b2c3d4e5f";
pub const COMMAND_CHAR_UUID: &str = "d1a51b00-0004-4a7e-9b3c-0a1b2c3d4e5f";

/// Seconds between pushes for pollable sensors, matching the ESP32
/// sketches. (Polling a BME680 faster than 1 Hz lets its gas heater warm
/// the die and drift the reading.)
const NOTIFY_INTERVAL: Duration = Duration::from_secs(1);
const NOTIFICATION_GAP: Duration = Duration::from_millis(10);

/// What this node does over BLE.
pub enum BleRole {
    /// Pollable sensor: the payload is only built (i.e. the sensor only
    /// read) while an authenticated client is connected, once per
    /// `interval` (`NOTIFY_INTERVAL` for most sensors; motion sensors
    /// push faster, like the Python nodes' MOTION_NOTIFY_INTERVAL).
    Poll {
        build_payload: Box<dyn FnMut() -> Option<Payload> + Send>,
        interval: Duration,
    },
    /// Push/actuator/display node: `tick` runs continuously at `interval`
    /// regardless of clients (a local button or event source must keep
    /// working), returning Some(state) whenever a change should be
    /// published. Actuator/display nodes additionally take binary
    /// commands on the command characteristic (gated on auth); pure push
    /// nodes pass None and keep the three-characteristic layout.
    Actuator {
        tick: Box<dyn FnMut() -> Option<Payload> + Send>,
        interval: Duration,
        on_command: Option<Box<dyn Fn(&[u8]) + Send + Sync>>,
    },
}

impl BleRole {
    /// A pollable sensor at the standard 1 Hz cadence.
    pub fn poll(build_payload: Box<dyn FnMut() -> Option<Payload> + Send>) -> Self {
        BleRole::Poll {
            build_payload,
            interval: NOTIFY_INTERVAL,
        }
    }
}

#[derive(Default)]
struct ClientAccess {
    clients: HashSet<Address>,
    authenticated: HashSet<Address>,
}

struct ConnectionUpdate {
    had_clients: bool,
    has_clients: bool,
    cleared_auth: bool,
}

impl ClientAccess {
    fn record(&mut self, address: Address) {
        self.clients.insert(address);
    }

    fn authenticate(&mut self, address: Address, authenticated: bool) {
        self.record(address);
        if authenticated {
            self.authenticated.insert(address);
        } else {
            self.authenticated.remove(&address);
        }
    }

    fn retain_connected(&mut self, connected: &HashSet<Address>) -> ConnectionUpdate {
        let had_clients = !self.clients.is_empty();
        let authenticated_before = self.authenticated.len();
        self.clients.retain(|address| connected.contains(address));
        self.authenticated
            .retain(|address| connected.contains(address));
        ConnectionUpdate {
            had_clients,
            has_clients: !self.clients.is_empty(),
            cleared_auth: self.authenticated.len() < authenticated_before,
        }
    }
}

struct BleState {
    access: AsyncMutex<ClientAccess>,
    last_payload: AsyncMutex<Vec<u8>>,
    notifiers: AsyncMutex<HashMap<Address, CharacteristicWriter>>,
}

impl BleState {
    async fn record_client(&self, address: Address) {
        self.access.lock().await.record(address);
    }

    async fn set_authenticated(&self, address: Address, authenticated: bool) {
        self.access
            .lock()
            .await
            .authenticate(address, authenticated);
    }

    async fn is_authenticated(&self, address: Address) -> bool {
        self.access.lock().await.authenticated.contains(&address)
    }

    async fn authenticated_clients(&self) -> HashSet<Address> {
        self.access.lock().await.authenticated.clone()
    }

    async fn tracked_clients(&self) -> HashSet<Address> {
        self.access.lock().await.clients.clone()
    }

    async fn retain_connected(&self, connected: &HashSet<Address>) -> ConnectionUpdate {
        let update = self.access.lock().await.retain_connected(connected);
        self.notifiers
            .lock()
            .await
            .retain(|address, _| connected.contains(address));
        update
    }
}

/// Run the BLE transport until SIGINT/SIGTERM. Blocks the calling thread.
pub fn run_ble(name: &'static str, api_key: String, role: BleRole) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(serve(name, api_key, role))
}

async fn serve(name: &'static str, api_key: String, role: BleRole) -> Result<(), String> {
    let session = bluer::Session::new().await.map_err(|e| e.to_string())?;
    let adapter = session.default_adapter().await.map_err(|e| e.to_string())?;
    adapter.set_powered(true).await.map_err(|e| e.to_string())?;

    let state = Arc::new(BleState {
        access: AsyncMutex::new(ClientAccess::default()),
        last_payload: AsyncMutex::new(b"{}".to_vec()),
        notifiers: AsyncMutex::new(HashMap::new()),
    });

    let (mut tick, interval, tick_gated, on_command) = match role {
        BleRole::Poll {
            build_payload,
            interval,
        } => (build_payload, interval, true, None),
        BleRole::Actuator {
            tick,
            interval,
            on_command,
        } => (tick, interval, false, on_command.map(Arc::new)),
    };

    let service_uuid = Uuid::parse_str(SERVICE_UUID).unwrap();
    let (notify_control, notify_handle) = characteristic_control();

    // GATT application ------------------------------------------------------
    let read_state = state.clone();
    let write_state = state.clone();
    let write_key = api_key.clone();

    let mut characteristics = vec![
        Characteristic {
            uuid: Uuid::parse_str(DATA_CHAR_UUID).unwrap(),
            read: Some(CharacteristicRead {
                read: true,
                fun: Box::new(move |req| {
                    let state = read_state.clone();
                    let address = req.device_address;
                    async move {
                        state.record_client(address).await;
                        // Serve the cached payload so slow sensor reads stay
                        // out of the callback.
                        if state.is_authenticated(address).await {
                            Ok(state.last_payload.lock().await.clone())
                        } else {
                            Ok(b"{}".to_vec())
                        }
                    }
                    .boxed()
                }),
                ..Default::default()
            }),
            notify: Some(CharacteristicNotify {
                notify: true,
                // The IO API includes the subscribing central's address, so
                // notifications can be gated per authenticated client.
                method: CharacteristicNotifyMethod::Io,
                ..Default::default()
            }),
            control_handle: notify_handle,
            ..Default::default()
        },
        Characteristic {
            uuid: Uuid::parse_str(AUTH_CHAR_UUID).unwrap(),
            write: Some(CharacteristicWrite {
                write: true,
                method: CharacteristicWriteMethod::Fun(Box::new(move |value, req| {
                    let state = write_state.clone();
                    let api_key = write_key.clone();
                    let address = req.device_address;
                    async move {
                        // Some clients append a trailing NUL to string writes.
                        let key = String::from_utf8_lossy(&value);
                        let key = key.trim_end_matches('\0');
                        let ok = key_matches(key, &api_key);
                        state.set_authenticated(address, ok).await;
                        println!(
                            "{}: {address}",
                            if ok {
                                "BLE client authenticated"
                            } else {
                                "BLE auth rejected"
                            }
                        );
                        Ok(())
                    }
                    .boxed()
                })),
                ..Default::default()
            }),
            ..Default::default()
        },
    ];

    // The command characteristic only exists on actuator nodes, so sensor
    // nodes keep the three-characteristic layout they always advertised.
    if let Some(on_command) = on_command {
        let command_state = state.clone();
        characteristics.push(Characteristic {
            uuid: Uuid::parse_str(COMMAND_CHAR_UUID).unwrap(),
            write: Some(CharacteristicWrite {
                write: true,
                write_without_response: true,
                method: CharacteristicWriteMethod::Fun(Box::new(move |value, req| {
                    let state = command_state.clone();
                    let on_command = on_command.clone();
                    let address = req.device_address;
                    async move {
                        state.record_client(address).await;
                        // Commands are gated like reads and notifications.
                        if state.is_authenticated(address).await {
                            on_command(&value);
                        }
                        Ok(())
                    }
                    .boxed()
                })),
                ..Default::default()
            }),
            ..Default::default()
        });
    }

    let app = Application {
        services: vec![Service {
            uuid: service_uuid,
            primary: true,
            characteristics,
            ..Default::default()
        }],
        ..Default::default()
    };
    let app_handle = adapter
        .serve_gatt_application(app)
        .await
        .map_err(|e| format!("registering GATT application: {e}"))?;

    // Advertising, with the kernel mgmt fallback ----------------------------
    let make_advertisement = |name: &str| Advertisement {
        advertisement_type: bluer::adv::Type::Peripheral,
        service_uuids: [service_uuid].into_iter().collect(),
        discoverable: Some(true),
        local_name: Some(name.to_string()),
        ..Default::default()
    };
    let mut adv_handle = None;
    let mut mgmt_adv = None;
    match adapter.advertise(make_advertisement(name)).await {
        Ok(handle) => adv_handle = Some(handle),
        Err(bluez_err) => match MgmtAdvertiser::start(name, SERVICE_UUID) {
            Ok(adv) => {
                println!("BlueZ advertising failed, advertising via the kernel instead");
                mgmt_adv = Some(adv);
            }
            Err(fallback_err) => {
                return Err(format!(
                    "BLE advertising could not be started: BlueZ said '{bluez_err}' \
                     and the kernel fallback failed: {fallback_err}"
                ));
            }
        },
    }
    enforce_on_air(name, &mut mgmt_adv);
    println!("BLE advertising as {name} (service {SERVICE_UUID})");

    // Tick loop + disconnect watch + signal handling ------------------------
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|e| e.to_string())?;
    let mut ticker = tokio::time::interval(interval);
    let mut connected_check = tokio::time::interval(Duration::from_secs(1));
    let mut message_id: u8 = 0;
    futures::pin_mut!(notify_control);

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = sigterm.recv() => break,
            event = notify_control.next() => {
                match event {
                    Some(CharacteristicControlEvent::Notify(notifier)) => {
                        let address = notifier.device_address();
                        state.record_client(address).await;
                        state.notifiers.lock().await.insert(address, notifier);
                    }
                    Some(_) => {}
                    None => break,
                }
            }
            _ = connected_check.tick() => {
                // Clear authentication when the client disconnects (like the
                // ESP32); re-arm the advertisement so the next client can
                // still find this node.
                let update = refresh_gatt_clients(&adapter, &state).await;
                if update.cleared_auth {
                    println!("BLE client disconnected (auth cleared)");
                }
                if update.had_clients && !update.has_clients {
                    // Connecting stopped the advertisement and bluetoothd
                    // does not re-arm it.
                    if adv_handle.is_some() {
                        adv_handle = None; // drop the stale registration first
                        match adapter.advertise(make_advertisement(name)).await {
                            Ok(handle) => adv_handle = Some(handle),
                            Err(err) => println!("BLE re-advertising failed: {err}"),
                        }
                    }
                    enforce_on_air(name, &mut mgmt_adv);
                }
            }
            _ = ticker.tick() => {
                let mut authenticated = state.authenticated_clients().await;
                if tick_gated && authenticated.is_empty() {
                    continue;
                }
                let payload: Option<Payload> = tokio::task::block_in_place(&mut tick);
                let Some(payload) = payload else { continue };
                let encoded = serde_json::to_vec(&Value::Object(payload)).unwrap();
                *state.last_payload.lock().await = encoded.clone();

                // Authentication can change while the sensor read runs.
                authenticated = state.authenticated_clients().await;
                if authenticated.is_empty() {
                    continue;
                }
                message_id = message_id.wrapping_add(1);
                let mut notifiers = state.notifiers.lock().await;
                let mut stale = Vec::new();
                for (address, notifier) in notifiers.iter() {
                    if !authenticated.contains(address) {
                        continue;
                    }
                    let mut failed = false;
                    for packet in frame_payload(&encoded, message_id) {
                        if notifier.send(&packet).await.is_err() {
                            failed = true;
                            break;
                        }
                        tokio::time::sleep(NOTIFICATION_GAP).await;
                    }
                    if failed || notifier.is_closed().unwrap_or(true) {
                        stale.push(*address);
                    }
                }
                for address in stale {
                    notifiers.remove(&address);
                }
            }
        }
    }

    // Release the advertising instance, otherwise it stays registered and
    // keeps advertising a node that is no longer running.
    drop(adv_handle);
    if let Some(adv) = mgmt_adv.take() {
        adv.stop();
    }
    drop(app_handle);
    println!("BLE transport stopped");
    Ok(())
}

/// bluetoothd can accept an advertisement registration while the controller
/// never starts transmitting (observed on Raspberry Pi, especially after a
/// connect/disconnect cycle). When running with CAP_NET_ADMIN, verify the
/// kernel state and drive the mgmt socket directly if the ad is not on air.
/// Must only be called while no client is connected — a connection
/// legitimately suspends advertising.
fn enforce_on_air(name: &str, mgmt_adv: &mut Option<MgmtAdvertiser>) {
    match kernel_advertising_active() {
        Ok(true) => {}
        Ok(false) => match mgmt_adv {
            Some(adv) => {
                if let Err(err) = adv.restart() {
                    println!("BLE re-advertising failed: {err}");
                }
            }
            None => match MgmtAdvertiser::start(name, SERVICE_UUID) {
                Ok(adv) => {
                    println!("bluetoothd left advertising off, advertising via the kernel instead");
                    *mgmt_adv = Some(adv);
                }
                Err(err) => println!("BLE kernel advertising fallback failed: {err}"),
            },
        },
        Err(_) => {} // no CAP_NET_ADMIN: cannot verify, trust bluetoothd
    }
}

async fn refresh_gatt_clients(adapter: &bluer::Adapter, state: &BleState) -> ConnectionUpdate {
    let tracked = state.tracked_clients().await;
    let mut connected = HashSet::new();
    for address in tracked {
        if let Ok(device) = adapter.device(address) {
            // Preserve state on a transient D-Bus query failure; a confirmed
            // `false` is required before authentication is cleared.
            if device.is_connected().await.unwrap_or(true) {
                connected.insert(address);
            }
        }
    }
    state.retain_connected(&connected).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(value: &str) -> Address {
        value.parse().unwrap()
    }

    #[test]
    fn authentication_is_per_client_and_disconnect_prunes_only_that_client() {
        let first = address("00:11:22:33:44:55");
        let second = address("AA:BB:CC:DD:EE:FF");
        let mut access = ClientAccess::default();
        access.authenticate(first, true);
        access.authenticate(second, false);

        assert!(access.authenticated.contains(&first));
        assert!(!access.authenticated.contains(&second));

        let update = access.retain_connected(&HashSet::from([second]));
        assert!(update.had_clients);
        assert!(update.has_clients);
        assert!(update.cleared_auth);
        assert!(!access.authenticated.contains(&first));
        assert!(access.clients.contains(&second));
    }
}
