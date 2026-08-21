// Command parsing for the display node: the JSON commands the app pushes
// over the WebSocket, a port of the Python node's handler.
//
// The Python node also accepts the same actions as chunked binary writes
// over BLE; this port has no BLE transport, so that framing lives only in
// the Rust port.
package main

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
)

// -- WebSocket commands ---------------------------------------------------

// handleJSONCommand executes one JSON command and returns its ACK/NACK.
// The app matches the reply to its command by id, so an id is required
// even for a command that fails validation.
func handleJSONCommand(display Display, message []byte) map[string]any {
	var command map[string]any
	if err := json.Unmarshal(message, &command); err != nil {
		fmt.Println("Ignoring malformed command")
		return map[string]any{"id": nil, "ok": false, "error": "malformed JSON"}
	}

	rawID, present := command["id"]
	// JSON numbers decode as float64; accept integers only, like the
	// Python node's isinstance(int) check.
	id, isNumber := rawID.(float64)
	if !present || !isNumber || id != float64(int(id)) {
		return map[string]any{"id": nil, "ok": false, "error": "missing command id"}
	}
	commandID := int(id)

	if clear, _ := command["clear"].(bool); clear {
		fmt.Println("Command: clear")
		if err := display.Clear(); err != nil {
			return map[string]any{"id": commandID, "ok": false, "error": err.Error()}
		}
		return map[string]any{"id": commandID, "ok": true}
	}

	encoded, present := command["image"].(string)
	if !present {
		return map[string]any{"id": commandID, "ok": false, "error": "unknown command"}
	}
	data, err := base64.StdEncoding.DecodeString(encoded)
	if err != nil {
		fmt.Println("Ignoring command with invalid base64 image")
		return map[string]any{"id": commandID, "ok": false, "error": "invalid base64"}
	}
	if len(data) != frameSize {
		fmt.Printf("Ignoring image with %d bytes (need %d)\n", len(data), frameSize)
		return map[string]any{
			"id":    commandID,
			"ok":    false,
			"error": fmt.Sprintf("image has %d bytes; need %d", len(data), frameSize),
		}
	}
	fmt.Println("Command: image")
	if err := display.ShowBitmap(data); err != nil {
		return map[string]any{"id": commandID, "ok": false, "error": err.Error()}
	}
	return map[string]any{"id": commandID, "ok": true}
}
