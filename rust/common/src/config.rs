//! config.json loading — same schema conventions and behavior as
//! python/common/wifi_transport.load_config. Each node defines its own
//! serde config struct with defaults and loads it through here.

use serde::de::DeserializeOwned;
use std::path::PathBuf;
use std::process::exit;

/// Read config.json from the working directory, falling back to the
/// executable's directory (the Python nodes read it from the script's
/// folder; a compiled binary is normally deployed next to its config).
/// Missing file or missing api_key exits with the standard hint.
pub fn load_config<T: DeserializeOwned>() -> T {
    let mut path = PathBuf::from("config.json");
    if !path.exists() {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let alt = dir.join("config.json");
                if alt.exists() {
                    path = alt;
                }
            }
        }
    }

    let Ok(data) = std::fs::read_to_string(&path) else {
        println!("Error: config.json not found.\nCopy config.example.json to config.json and edit it.");
        exit(1);
    };

    match serde_json::from_str(&data) {
        Ok(cfg) => cfg,
        Err(err) => {
            println!("Error parsing {}: {err}", path.display());
            exit(1);
        }
    }
}

/// The configured hostname, or the system hostname when empty.
pub fn hostname_or(h: &str) -> String {
    if !h.is_empty() {
        return h.to_string();
    }
    hostname::get()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Exit with the standard message when the api_key is missing.
pub fn require_api_key(api_key: &str) {
    if api_key.is_empty() {
        println!("Error: config.json: api_key is required");
        exit(1);
    }
}
