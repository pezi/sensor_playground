//! Sensor Playground Relay Node — Grove SPDT Relay, 1 / 2 / 4 channels.
//!
//! GPIO character-device lines drive 1-/2-channel modules; the Grove
//! 4-channel module is driven directly over I2C with command 0x10 + bitmask.
//! Wi-Fi uses authenticated WebSocket commands/state and UDP discovery. On
//! Linux, BLE uses the same binary command packets as the ESP32/Python nodes.

mod gpio;
mod relay;

use common::{config, wifi, ws::WsPushServer, Payload};
#[cfg(target_os = "linux")]
use relay::handle_ble_command;
use relay::{handle_json_command, EmulatedRelayBank, RelayBank, RelayController};
use serde::Deserialize;
use serde_json::json;
use std::process::exit;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const PUBLISH_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Deserialize)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct Config {
    api_key: String,
    #[serde(default)]
    hostname: String,
    #[serde(default = "default_sensor_name")]
    sensor_name: String,
    #[serde(default = "default_interface")]
    interface: String,
    #[serde(default = "default_gpio_chip")]
    gpio_chip: String,
    #[serde(default = "default_relay_pins")]
    relay_pins: Vec<u32>,
    #[serde(default)]
    relay_active_low: bool,
    #[serde(default = "default_channels")]
    channels: usize,
    #[serde(default = "default_i2c_address")]
    i2c_address: String,
    #[serde(default = "default_i2c_bus")]
    i2c_bus: u8,
    #[serde(default = "default_transport")]
    transport: String,
    #[serde(default)]
    emulation: bool,
}

fn default_sensor_name() -> String {
    "RELAY".into()
}
fn default_interface() -> String {
    "gpio".into()
}
fn default_gpio_chip() -> String {
    "/dev/gpiochip0".into()
}
fn default_relay_pins() -> Vec<u32> {
    vec![5, 6]
}
fn default_channels() -> usize {
    4
}
fn default_i2c_address() -> String {
    "0x11".into()
}
fn default_i2c_bus() -> u8 {
    1
}
fn default_transport() -> String {
    "wifi".into()
}

fn configured_channel_count(cfg: &Config) -> Result<usize, String> {
    let count = if cfg.interface == "i2c" {
        cfg.channels
    } else {
        cfg.relay_pins.len()
    };
    if !(1..=8).contains(&count) {
        return Err("config.json: relay board must have 1..8 channels".into());
    }
    Ok(count)
}

#[cfg(any(target_os = "linux", test))]
fn parse_i2c_address(text: &str) -> Result<u16, String> {
    let digits = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    let address = u16::from_str_radix(digits, 16)
        .map_err(|_| format!("config.json: invalid i2c_address {text:?}"))?;
    if address > 0x7f {
        return Err(format!(
            "config.json: i2c_address {text:?} is outside 0x00..0x7f"
        ));
    }
    Ok(address)
}

fn make_relay_bank(cfg: &Config) -> Result<Box<dyn RelayBank>, String> {
    let count = configured_channel_count(cfg)?;
    if cfg.emulation {
        return Ok(Box::new(EmulatedRelayBank::new(count)?));
    }
    #[cfg(target_os = "linux")]
    {
        match cfg.interface.as_str() {
            "gpio" => Ok(Box::new(gpio::GpioRelayBank::open(
                &cfg.gpio_chip,
                &cfg.relay_pins,
                cfg.relay_active_low,
            )?)),
            "i2c" => Ok(Box::new(relay::I2cRelayBank::open(
                cfg.i2c_bus,
                parse_i2c_address(&cfg.i2c_address)?,
                cfg.channels,
            )?)),
            other => Err(format!(
                "interface {other:?} is not supported (use \"gpio\" or \"i2c\"; Arduino-based hats require Python)"
            )),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err("relay GPIO/I2C access requires Linux (use emulation elsewhere)".into())
    }
}

type SharedRelay = Arc<Mutex<RelayController>>;

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

fn install_signal_handlers() {
    unsafe {
        let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
    }
}

fn stop_relays(relay: &SharedRelay) {
    let mut relay = relay.lock().unwrap();
    if let Err(err) = relay.set_all(false) {
        println!("Relay shutdown failed: {err}");
    }
    relay.close();
}

fn main() {
    let cfg: Config = config::load_config();
    config::require_api_key(&cfg.api_key);
    let hostname = config::hostname_or(&cfg.hostname);
    let sensor_name: &'static str = Box::leak(cfg.sensor_name.clone().into_boxed_str());

    println!(
        "{}",
        if cfg.emulation {
            "Emulation mode: switching virtual relays without hardware"
        } else {
            "Initializing relay node..."
        }
    );
    let bank = match make_relay_bank(&cfg) {
        Ok(bank) => bank,
        Err(err) => {
            println!("Error: {err}");
            exit(1);
        }
    };
    let relay = match RelayController::new(bank) {
        Ok(relay) => Arc::new(Mutex::new(relay)),
        Err(err) => {
            println!("Error: {err}");
            exit(1);
        }
    };
    println!("Relay board: {} channel(s)", relay.lock().unwrap().count());

    if cfg.transport == "ble" {
        #[cfg(target_os = "linux")]
        {
            let tick_relay = relay.clone();
            let command_relay = relay.clone();
            let result = common::ble::run_ble(
                sensor_name,
                cfg.api_key,
                common::ble::BleRole::Actuator {
                    tick: Box::new(move || tick_relay.lock().unwrap().take_pending()),
                    interval: PUBLISH_INTERVAL,
                    on_command: Some(Box::new(move |packet| {
                        if let Err(err) =
                            handle_ble_command(&mut command_relay.lock().unwrap(), packet)
                        {
                            println!("BLE command failed: {err}");
                        }
                    })),
                },
            );
            stop_relays(&relay);
            if let Err(err) = result {
                println!("Error: {err}");
                exit(1);
            }
            return;
        }
        #[cfg(not(target_os = "linux"))]
        println!("Warning: BLE transport requires Linux, using wifi.");
    }

    install_signal_handlers();

    let server = Arc::new(WsPushServer::new(
        cfg.api_key,
        {
            let relay = relay.clone();
            move |send: &mut dyn FnMut(&Payload)| send(&relay.lock().unwrap().payload())
        },
        {
            let relay = relay.clone();
            move |message: &str| {
                if let Err(err) = handle_json_command(&mut relay.lock().unwrap(), message) {
                    println!("Ignoring command: {err}");
                }
            }
        },
    ));

    {
        let hostname = hostname.clone();
        let channels = relay.lock().unwrap().count();
        std::thread::spawn(move || {
            let discovery = move || {
                let mut payload = Payload::new();
                payload.insert("channels".into(), json!(channels));
                Some(payload)
            };
            if let Err(err) = wifi::run_discovery_listener(
                sensor_name,
                &hostname,
                wifi::WS_PORT,
                Some(Box::new(discovery)),
            ) {
                println!("UDP discovery failed: {err}");
            }
        });
    }

    {
        let relay = relay.clone();
        let server = server.clone();
        std::thread::spawn(move || loop {
            if STOP.load(Ordering::SeqCst) {
                stop_relays(&relay);
                println!("Stopped.");
                exit(0);
            }
            if let Some(payload) = relay.lock().unwrap().take_pending() {
                println!("relay: {}", payload["relay"]);
                server.broadcast(&payload);
            }
            std::thread::sleep(PUBLISH_INTERVAL);
        });
    }

    if let Err(err) = server.listen_and_serve() {
        stop_relays(&relay);
        println!("Error: {err}");
        exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_for(interface: &str) -> Config {
        Config {
            api_key: "12345678".into(),
            hostname: String::new(),
            sensor_name: default_sensor_name(),
            interface: interface.into(),
            gpio_chip: default_gpio_chip(),
            relay_pins: default_relay_pins(),
            relay_active_low: false,
            channels: default_channels(),
            i2c_address: default_i2c_address(),
            i2c_bus: default_i2c_bus(),
            transport: default_transport(),
            emulation: true,
        }
    }

    #[test]
    fn configuration_uses_pins_or_i2c_width() {
        assert_eq!(configured_channel_count(&config_for("gpio")).unwrap(), 2);
        assert_eq!(configured_channel_count(&config_for("i2c")).unwrap(), 4);
        assert_eq!(parse_i2c_address("0x11").unwrap(), 0x11);
        assert_eq!(parse_i2c_address("12").unwrap(), 0x12);
        assert!(parse_i2c_address("0x80").is_err());
    }
}
