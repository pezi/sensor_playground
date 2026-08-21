// Shared WebSocket push server for the Go Sensor Playground nodes —
// event, streaming and display/actuator nodes push JSON over ws:// on
// port 9132, authenticated with the X-Api-Key handshake header.
// Mirrors python/common/wifi_transport.WsPushServer.
package common

import (
	"encoding/json"
	"fmt"
	"net/http"
	"sync"

	"github.com/gorilla/websocket"
)

type WsServer struct {
	apiKey    string
	onConnect func(send func(payload any) error)
	onMessage func(message []byte) any
	clients   map[*websocket.Conn]*sync.Mutex // per-conn write lock
	mu        sync.Mutex
	upgrader  websocket.Upgrader
}

// NewWsServer builds a push server. onConnect (optional) runs once per new
// client, e.g. to send the current state; onMessage (optional) handles each
// incoming message.
func NewWsServer(apiKey string, onConnect func(send func(payload any) error), onMessage func(message []byte)) *WsServer {
	var handler func([]byte) any
	if onMessage != nil {
		handler = func(message []byte) any { onMessage(message); return nil }
	}
	return newWsServer(apiKey, onConnect, handler)
}

// NewWsServerReplying builds a push server whose message handler answers
// the client that sent the message: a non-nil return value is sent back to
// that one client (the display nodes' command ACKs), nil sends nothing.
// Same contract as the Python server's on_message.
func NewWsServerReplying(apiKey string, onConnect func(send func(payload any) error), onMessage func(message []byte) any) *WsServer {
	return newWsServer(apiKey, onConnect, onMessage)
}

func newWsServer(apiKey string, onConnect func(send func(payload any) error), onMessage func(message []byte) any) *WsServer {
	return &WsServer{
		apiKey:    apiKey,
		onConnect: onConnect,
		onMessage: onMessage,
		clients:   make(map[*websocket.Conn]*sync.Mutex),
		upgrader:  websocket.Upgrader{CheckOrigin: func(*http.Request) bool { return true }},
	}
}

// Broadcast sends a JSON payload to all connected clients.
func (s *WsServer) Broadcast(payload any) {
	message, err := json.Marshal(payload)
	if err != nil {
		return
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	for conn, lock := range s.clients {
		lock.Lock()
		err := conn.WriteMessage(websocket.TextMessage, message)
		lock.Unlock()
		if err != nil {
			conn.Close()
			delete(s.clients, conn)
		}
	}
}

func (s *WsServer) handle(w http.ResponseWriter, r *http.Request) {
	// Reject WebSocket handshakes without the correct X-Api-Key.
	if !KeyMatches(r.Header.Get("X-Api-Key"), s.apiKey) {
		w.WriteHeader(http.StatusUnauthorized)
		return
	}
	conn, err := s.upgrader.Upgrade(w, r, nil)
	if err != nil {
		return
	}
	lock := &sync.Mutex{}
	s.mu.Lock()
	s.clients[conn] = lock
	s.mu.Unlock()
	fmt.Println("Client connected")

	if s.onConnect != nil {
		s.onConnect(func(payload any) error {
			message, err := json.Marshal(payload)
			if err != nil {
				return err
			}
			lock.Lock()
			defer lock.Unlock()
			return conn.WriteMessage(websocket.TextMessage, message)
		})
	}

	for {
		_, message, err := conn.ReadMessage()
		if err != nil {
			break
		}
		if s.onMessage != nil {
			if reply := s.onMessage(message); reply != nil {
				if encoded, err := json.Marshal(reply); err == nil {
					lock.Lock()
					err = conn.WriteMessage(websocket.TextMessage, encoded)
					lock.Unlock()
					if err != nil {
						break
					}
				}
			}
		}
	}

	s.mu.Lock()
	delete(s.clients, conn)
	s.mu.Unlock()
	conn.Close()
	fmt.Println("Client disconnected")
}

// ListenAndServe serves ws:// on WSPort. Blocks forever.
func (s *WsServer) ListenAndServe() error {
	mux := http.NewServeMux()
	mux.HandleFunc("/", s.handle)
	fmt.Printf("WebSocket server on port %d\n", WSPort)
	server := &http.Server{Addr: fmt.Sprintf(":%d", WSPort), Handler: mux}
	return server.ListenAndServe()
}
