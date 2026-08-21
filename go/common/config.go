// Shared config.json loading for the Go Sensor Playground nodes — same
// schema conventions and behavior as python/common/wifi_transport.load_config.
package common

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
)

// LoadJSONConfig reads config.json from the working directory, falling back
// to the executable's directory (the Python nodes read it from the script's
// folder; a compiled binary is normally deployed next to its config), and
// unmarshals it into v. A missing file prints the standard hint and exits.
func LoadJSONConfig(v any) error {
	path := "config.json"
	if _, err := os.Stat(path); err != nil {
		if exe, exeErr := os.Executable(); exeErr == nil {
			alt := filepath.Join(filepath.Dir(exe), "config.json")
			if _, altErr := os.Stat(alt); altErr == nil {
				path = alt
			}
		}
	}

	data, err := os.ReadFile(path)
	if err != nil {
		fmt.Println("Error: config.json not found.\nCopy config.example.json to config.json and edit it.")
		os.Exit(1)
	}
	if err := json.Unmarshal(data, v); err != nil {
		return fmt.Errorf("parsing %s: %w", path, err)
	}
	return nil
}

// HostnameOr returns h, or the system hostname when h is empty.
func HostnameOr(h string) string {
	if h != "" {
		return h
	}
	if sys, err := os.Hostname(); err == nil {
		return sys
	}
	return ""
}
