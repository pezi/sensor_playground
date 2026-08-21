//! Relay hardware abstraction, authoritative state and command parsers.

use common::Payload;
use serde_json::json;

#[cfg(any(target_os = "linux", test))]
pub const CMD_SET: u8 = 0x01;
#[cfg(any(target_os = "linux", test))]
pub const CMD_TOGGLE: u8 = 0x02;
#[cfg(any(target_os = "linux", test))]
pub const CMD_ALL: u8 = 0x03;
#[cfg(target_os = "linux")]
pub const I2C_COMMAND_CHANNEL_CONTROL: u8 = 0x10;

pub trait RelayBank: Send {
    fn count(&self) -> usize;
    fn write(&mut self, states: &[bool]) -> Result<(), String>;
    fn close(&mut self) {}
}

/// Owns the state reported to clients. New state is committed only after the
/// board accepted it, so a failed hardware write is never advertised.
pub struct RelayController {
    bank: Box<dyn RelayBank>,
    states: Vec<bool>,
    pending: bool,
}

impl RelayController {
    pub fn new(mut bank: Box<dyn RelayBank>) -> Result<Self, String> {
        if !(1..=8).contains(&bank.count()) {
            return Err(format!(
                "relay board must have 1..8 channels (got {})",
                bank.count()
            ));
        }
        let states = vec![false; bank.count()];
        bank.write(&states)
            .map_err(|err| format!("switching all relays off at startup: {err}"))?;
        Ok(Self {
            bank,
            states,
            pending: true,
        })
    }

    pub fn count(&self) -> usize {
        self.states.len()
    }

    fn apply(&mut self, next: Vec<bool>) -> Result<(), String> {
        self.bank.write(&next)?;
        self.states = next;
        self.pending = true;
        Ok(())
    }

    pub fn set(&mut self, channel: usize, on: bool) -> Result<(), String> {
        if channel >= self.states.len() {
            return Err(format!(
                "channel {channel} is outside board width {}",
                self.states.len()
            ));
        }
        let mut next = self.states.clone();
        next[channel] = on;
        self.apply(next)
    }

    pub fn toggle(&mut self, channel: usize) -> Result<(), String> {
        if channel >= self.states.len() {
            return Err(format!(
                "channel {channel} is outside board width {}",
                self.states.len()
            ));
        }
        let mut next = self.states.clone();
        next[channel] = !next[channel];
        self.apply(next)
    }

    pub fn set_all(&mut self, on: bool) -> Result<(), String> {
        self.apply(vec![on; self.states.len()])
    }

    pub fn payload(&self) -> Payload {
        let mut payload = Payload::new();
        payload.insert("channels".into(), json!(self.states.len()));
        payload.insert("relay".into(), json!(self.states));
        payload
    }

    pub fn take_pending(&mut self) -> Option<Payload> {
        if !std::mem::take(&mut self.pending) {
            return None;
        }
        Some(self.payload())
    }

    pub fn close(&mut self) {
        self.bank.close();
    }
}

pub struct EmulatedRelayBank {
    count: usize,
}

impl EmulatedRelayBank {
    pub fn new(count: usize) -> Result<Self, String> {
        if !(1..=8).contains(&count) {
            return Err("channels must be 1..8".into());
        }
        Ok(Self { count })
    }
}

impl RelayBank for EmulatedRelayBank {
    fn count(&self) -> usize {
        self.count
    }

    fn write(&mut self, states: &[bool]) -> Result<(), String> {
        let shown = states
            .iter()
            .enumerate()
            .map(|(channel, on)| format!("{}:{}", channel + 1, if *on { "on" } else { "off" }))
            .collect::<Vec<_>>()
            .join(", ");
        println!("[emulation] relays {shown}");
        Ok(())
    }
}

#[cfg(any(target_os = "linux", test))]
pub fn relay_mask(states: &[bool]) -> u8 {
    states.iter().enumerate().fold(
        0,
        |mask, (channel, on)| {
            if *on {
                mask | (1 << channel)
            } else {
                mask
            }
        },
    )
}

#[cfg(target_os = "linux")]
pub struct I2cRelayBank {
    device: common::i2c::I2CDevice,
    count: usize,
}

#[cfg(target_os = "linux")]
impl I2cRelayBank {
    pub fn open(bus: u8, address: u16, count: usize) -> Result<Self, String> {
        if !(1..=8).contains(&count) {
            return Err("channels must be 1..8 for the I2C relay board".into());
        }
        let device = common::i2c::I2CDevice::open(bus, address)
            .map_err(|err| format!("opening /dev/i2c-{bus} address 0x{address:02x}: {err}"))?;
        Ok(Self { device, count })
    }
}

#[cfg(target_os = "linux")]
impl RelayBank for I2cRelayBank {
    fn count(&self) -> usize {
        self.count
    }

    fn write(&mut self, states: &[bool]) -> Result<(), String> {
        if states.len() != self.count {
            return Err(format!(
                "got {} relay states for {} I2C channels",
                states.len(),
                self.count
            ));
        }
        self.device
            .write_reg(I2C_COMMAND_CHANNEL_CONTROL, relay_mask(states))
            .map_err(|err| format!("writing the relay I2C bitmask: {err}"))
    }
}

/// Execute one JSON command pushed over the WebSocket.
pub fn handle_json_command(relay: &mut RelayController, message: &str) -> Result<(), String> {
    let command: serde_json::Value = match serde_json::from_str(message) {
        Ok(command) => command,
        Err(_) => {
            println!("Ignoring malformed command");
            return Ok(());
        }
    };

    if let Some(on) = command.get("all").and_then(|value| value.as_bool()) {
        println!("Command: all {}", if on { "on" } else { "off" });
        return relay.set_all(on);
    }
    let Some(channel) = command
        .get("ch")
        .and_then(|value| value.as_u64())
        .and_then(|value| usize::try_from(value).ok())
    else {
        println!("Ignoring command without an integer channel");
        return Ok(());
    };
    if command.get("toggle") == Some(&json!(true)) {
        println!("Command: toggle channel {channel}");
        return relay.toggle(channel);
    }
    let Some(on) = command.get("on").and_then(|value| value.as_bool()) else {
        println!("Ignoring command without a boolean 'on'");
        return Ok(());
    };
    println!(
        "Command: channel {channel} {}",
        if on { "on" } else { "off" }
    );
    relay.set(channel, on)
}

/// Execute one binary command packet written over BLE.
#[cfg(any(target_os = "linux", test))]
pub fn handle_ble_command(relay: &mut RelayController, packet: &[u8]) -> Result<(), String> {
    match packet {
        [CMD_SET, channel, state, ..] => relay.set(usize::from(*channel), *state != 0),
        [CMD_TOGGLE, channel, ..] => relay.toggle(usize::from(*channel)),
        [CMD_ALL, state, ..] => relay.set_all(*state != 0),
        [] => Err("empty command packet".into()),
        [CMD_SET, ..] => Err("set without a channel and state byte".into()),
        [CMD_TOGGLE, ..] => Err("toggle without a channel byte".into()),
        [CMD_ALL, ..] => Err("all without a state byte".into()),
        [opcode, ..] => Err(format!("unknown opcode 0x{opcode:02x}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    type Writes = Arc<Mutex<Vec<Vec<bool>>>>;
    type Failure = Arc<AtomicBool>;

    struct RecordingBank {
        count: usize,
        writes: Arc<Mutex<Vec<Vec<bool>>>>,
        fail: Arc<AtomicBool>,
    }

    impl RelayBank for RecordingBank {
        fn count(&self) -> usize {
            self.count
        }

        fn write(&mut self, states: &[bool]) -> Result<(), String> {
            if self.fail.load(Ordering::SeqCst) {
                return Err("write failed".into());
            }
            self.writes.lock().unwrap().push(states.to_vec());
            Ok(())
        }
    }

    fn test_relay(count: usize) -> (RelayController, Writes, Failure) {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let fail = Arc::new(AtomicBool::new(false));
        let mut relay = RelayController::new(Box::new(RecordingBank {
            count,
            writes: writes.clone(),
            fail: fail.clone(),
        }))
        .unwrap();
        relay.take_pending();
        (relay, writes, fail)
    }

    #[test]
    fn json_commands_update_and_publish_the_complete_board() {
        let (mut relay, writes, _) = test_relay(2);
        handle_json_command(&mut relay, r#"{"ch":0,"on":true}"#).unwrap();
        handle_json_command(&mut relay, r#"{"ch":1,"toggle":true}"#).unwrap();
        assert_eq!(relay.payload()["relay"], json!([true, true]));
        handle_json_command(&mut relay, r#"{"all":false}"#).unwrap();
        assert_eq!(relay.payload()["relay"], json!([false, false]));
        assert_eq!(writes.lock().unwrap().len(), 4);
        let payload = relay.take_pending().unwrap();
        assert_eq!(payload["channels"], json!(2));
    }

    #[test]
    fn malformed_and_out_of_range_commands_are_ignored() {
        for command in [
            "not json",
            r#"{"ch":2,"on":true}"#,
            r#"{"ch":-1,"on":true}"#,
            r#"{"ch":0.5,"on":true}"#,
            r#"{"ch":true,"on":true}"#,
            r#"{"ch":0,"on":1}"#,
            r#"{"toggle":true}"#,
        ] {
            let (mut relay, writes, _) = test_relay(2);
            let _ = handle_json_command(&mut relay, command);
            assert_eq!(writes.lock().unwrap().len(), 1, "{command}");
            assert!(relay.take_pending().is_none(), "{command}");
        }
    }

    #[test]
    fn failed_hardware_write_is_not_committed() {
        let (mut relay, _, fail) = test_relay(1);
        fail.store(true, Ordering::SeqCst);
        assert!(relay.set(0, true).is_err());
        assert_eq!(relay.payload()["relay"], json!([false]));
        assert!(relay.take_pending().is_none());
    }

    #[test]
    fn ble_commands_and_i2c_mask_match_the_wire_protocol() {
        let (mut relay, _, _) = test_relay(4);
        handle_ble_command(&mut relay, &[CMD_SET, 0, 1]).unwrap();
        handle_ble_command(&mut relay, &[CMD_TOGGLE, 2]).unwrap();
        handle_ble_command(&mut relay, &[CMD_SET, 3, 1]).unwrap();
        assert_eq!(relay_mask(&[true, false, true, true]), 0x0d);
        assert_eq!(relay.payload()["relay"], json!([true, false, true, true]));
        handle_ble_command(&mut relay, &[CMD_ALL, 0]).unwrap();
        assert_eq!(
            relay.payload()["relay"],
            json!([false, false, false, false])
        );
        assert!(handle_ble_command(&mut relay, &[CMD_SET, 0]).is_err());
    }
}
