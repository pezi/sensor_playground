//! Shared building blocks for the Rust Sensor Playground nodes, mirroring
//! python/common/: config loading, the Wi-Fi transport (UDP discovery +
//! HTTPS REST), the WebSocket push server, and — on Linux — I2C access and
//! the BLE GATT transport with the Raspberry Pi advertising workaround.

pub mod config;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub mod ble_framing;
pub mod wifi;
pub mod ws;

#[cfg(target_os = "linux")]
pub mod ble;
#[cfg(target_os = "linux")]
pub mod i2c;
#[cfg(target_os = "linux")]
pub mod mgmt_advertiser;

/// JSON payload: one reading or state message.
pub type Payload = serde_json::Map<String, serde_json::Value>;

/// Round like the Python nodes' round(x, 1).
pub fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// Round like the Python nodes' round(x, 2).
pub fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Uniform random in [lo, hi) for the emulation modes.
pub fn uniform(lo: f64, hi: f64) -> f64 {
    lo + fastrand::f64() * (hi - lo)
}

/// Seconds since the epoch, for the emulation modes' slow sines.
pub fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
