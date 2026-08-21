// Shared Wi-Fi transport for the Go Sensor Playground nodes:
// UDP discovery on 9133 + HTTPS REST on 9132.
// Mirrors python/common/wifi_transport.py.
package common

import (
	"context"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"strings"
	"syscall"

	"golang.org/x/sys/unix"
)

const (
	HTTPSPort   = 9132
	WSPort      = 9132 // same port, different protocol per node kind
	UDPPort     = 9133
	ScanKeyword = "SENSOR_TESTER"
)

// GetLocalIP determines the local network IP address (default-route
// interface; no packet is actually sent).
func GetLocalIP() (string, error) {
	conn, err := net.Dial("udp", "8.8.8.8:80")
	if err != nil {
		return "", err
	}
	defer conn.Close()
	return conn.LocalAddr().(*net.UDPAddr).IP.String(), nil
}

// KeyMatches compares API keys in constant time. Both sides are hashed
// first so the comparison length is fixed (ConstantTimeCompare would
// otherwise leak the length).
func KeyMatches(supplied, apiKey string) bool {
	a := sha256.Sum256([]byte(supplied))
	b := sha256.Sum256([]byte(apiKey))
	return subtle.ConstantTimeCompare(a[:], b[:]) == 1
}

// RunDiscoveryListener answers SENSOR_TESTER broadcasts with the node
// identity plus, for pollable nodes, short-key readings from
// readDiscovery (may be nil for push/display nodes). Runs forever;
// individual failures (e.g. an I2C hiccup) never kill the loop.
func RunDiscoveryListener(name, hostname string, port int, readDiscovery func() map[string]any) error {
	lc := net.ListenConfig{
		Control: func(network, address string, c syscall.RawConn) error {
			var soErr error
			err := c.Control(func(fd uintptr) {
				soErr = unix.SetsockoptInt(int(fd), unix.SOL_SOCKET, unix.SO_REUSEADDR, 1)
			})
			if err != nil {
				return err
			}
			return soErr
		},
	}
	pc, err := lc.ListenPacket(context.Background(), "udp4", fmt.Sprintf(":%d", UDPPort))
	if err != nil {
		return err
	}
	fmt.Printf("UDP discovery listening on port %d\n", UDPPort)

	buf := make([]byte, 1024)
	for {
		n, addr, err := pc.ReadFrom(buf)
		if err != nil {
			continue
		}
		if strings.TrimSpace(string(buf[:n])) != ScanKeyword {
			continue
		}

		ip, err := GetLocalIP()
		if err != nil {
			fmt.Printf("Discovery reply failed: %v\n", err)
			continue
		}
		reply := map[string]any{"type": name, "host": hostname, "ip": ip, "port": port}
		if readDiscovery != nil {
			for k, v := range readDiscovery() {
				reply[k] = v
			}
		}
		payload, err := json.Marshal(reply)
		if err != nil {
			fmt.Printf("Discovery reply failed: %v\n", err)
			continue
		}
		if _, err := pc.WriteTo(payload, addr); err != nil {
			fmt.Printf("Discovery reply failed: %v\n", err)
		}
	}
}

// RunRESTServer serves read() on GET / over HTTPS with X-Api-Key auth.
// read() returning nil yields a 503 {"error":"sensor_read_failed"}.
// Blocks forever.
func RunRESTServer(name, apiKey, hostname string, read func() map[string]any, sslCert, sslKey string) error {
	mux := http.NewServeMux()
	mux.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/" {
			http.NotFound(w, r)
			return
		}
		if r.Method != http.MethodGet {
			w.WriteHeader(http.StatusMethodNotAllowed)
			return
		}
		if !KeyMatches(r.Header.Get("X-Api-Key"), apiKey) {
			w.WriteHeader(http.StatusUnauthorized)
			return
		}

		data := read()
		w.Header().Set("Content-Type", "application/json")
		if data == nil {
			w.WriteHeader(http.StatusServiceUnavailable)
			json.NewEncoder(w).Encode(map[string]string{"error": "sensor_read_failed"})
			return
		}
		response := map[string]any{"sensor": name, "host": hostname}
		for k, v := range data {
			response[k] = v
		}
		json.NewEncoder(w).Encode(response)
	})

	fmt.Printf("HTTPS server on port %d\n", HTTPSPort)
	server := &http.Server{Addr: fmt.Sprintf(":%d", HTTPSPort), Handler: mux}
	return server.ListenAndServeTLS(sslCert, sslKey)
}
