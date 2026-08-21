'use strict';
/*
 * Shared WebSocket push server for the Node.js Sensor Playground nodes —
 * event, streaming and display/actuator nodes push JSON over ws:// on
 * port 9132, authenticated with the X-Api-Key handshake header.
 * Mirrors python/common/wifi_transport.WsPushServer.
 *
 * Requires the `ws` package (declared by the nodes that use this module).
 */

const http = require('http');

const { keyMatches, WS_PORT } = require('./wifi');

function attachClient(client, clients, onConnect, onMessage) {
  clients.add(client);
  console.log('Client connected');
  client.on('message', (message) => {
    if (!onMessage) return;
    // The handler may be async (the display node awaits the panel).
    // Nodes that only act on commands return nothing; a returned
    // payload is the ACK for this one client.
    Promise.resolve(onMessage(message.toString()))
      .then((reply) => {
        if (reply !== undefined && reply !== null) client.send(JSON.stringify(reply));
      })
      .catch((err) => console.log(`Command failed: ${err.message}`));
  });
  client.on('error', (err) => {
    // `ws` emits protocol and socket failures on the client object. Without
    // this listener Node treats them as uncaught errors and exits the node.
    clients.delete(client);
    console.log(`WebSocket client error: ${err.message}`);
  });
  client.on('close', () => {
    clients.delete(client);
    console.log('Client disconnected');
  });
  if (onConnect) {
    onConnect((payload) => client.send(JSON.stringify(payload)));
  }
}

/*
 * Start a push server. onConnect(send) (optional) runs once per new
 * client, e.g. to send the current state; onMessage(message) (optional)
 * handles each incoming message and may return a payload, which is then
 * sent back to *that* client as its reply (the display nodes' command
 * ACKs — same contract as the Python server's on_message). Returns
 * { broadcast, server }.
 */
function startWsPushServer(apiKey, onConnect, onMessage) {
  // Resolve `ws` from the node's own node_modules (the dependency lives in
  // each node's package.json, not next to this shared folder).
  const requireFromNode = require.main ? require.main.require.bind(require.main) : require;
  let WebSocketServer;
  try {
    ({ WebSocketServer } = requireFromNode('ws'));
  } catch {
    console.log(
      "Error: the ws package is not installed.\n" +
      "Run 'npm install' in this node's folder first."
    );
    process.exit(1);
  }

  const server = http.createServer();
  const wss = new WebSocketServer({ noServer: true });
  const clients = new Set();

  server.on('upgrade', (req, socket, head) => {
    // Reject WebSocket handshakes without the correct X-Api-Key.
    if (!keyMatches(req.headers['x-api-key'], apiKey)) {
      socket.write('HTTP/1.1 401 Unauthorized\r\n\r\n');
      socket.destroy();
      return;
    }
    wss.handleUpgrade(req, socket, head, (client) => {
      attachClient(client, clients, onConnect, onMessage);
    });
  });

  const broadcast = (payload) => {
    const message = JSON.stringify(payload);
    for (const client of clients) {
      try {
        client.send(message);
      } catch {
        clients.delete(client);
      }
    }
  };

  server.listen(WS_PORT, () => {
    console.log(`WebSocket server on port ${WS_PORT}`);
  });
  return { broadcast, server };
}

module.exports = { startWsPushServer, _attachClient: attachClient };
