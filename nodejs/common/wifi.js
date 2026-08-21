'use strict';
/*
 * Shared Wi-Fi transport for the Node.js Sensor Playground nodes:
 * UDP discovery on 9133 + HTTPS REST on 9132.
 * Mirrors python/common/wifi_transport.py.
 */

const crypto = require('crypto');
const dgram = require('dgram');
const fs = require('fs');
const https = require('https');

const HTTPS_PORT = 9132;
const WS_PORT = 9132; // same port, different protocol per node kind
const UDP_PORT = 9133;
const SCAN_KEYWORD = 'SENSOR_TESTER';

/** Local network IP of the default-route interface (no packet is sent). */
function getLocalIp() {
  return new Promise((resolve, reject) => {
    const sock = dgram.createSocket('udp4');
    sock.on('error', reject);
    sock.connect(80, '8.8.8.8', () => {
      const { address } = sock.address();
      sock.close();
      resolve(address);
    });
  });
}

/*
 * Constant-time API-key comparison. Both sides are hashed first so the
 * comparison length is fixed (timingSafeEqual throws on length mismatch).
 */
function keyMatches(supplied, apiKey) {
  const a = crypto.createHash('sha256').update(supplied || '', 'utf8').digest();
  const b = crypto.createHash('sha256').update(apiKey, 'utf8').digest();
  return crypto.timingSafeEqual(a, b);
}

/*
 * Answer SENSOR_TESTER broadcasts with the node identity plus, for
 * pollable nodes, short-key readings from readDiscovery (may be null for
 * push/display nodes; may be async). Individual failures (e.g. an I2C
 * hiccup) never kill the listener.
 */
function startDiscoveryListener(name, hostname, port, readDiscovery) {
  const sock = dgram.createSocket({ type: 'udp4', reuseAddr: true });

  sock.on('message', async (msg, rinfo) => {
    if (msg.toString('utf8').trim() !== SCAN_KEYWORD) return;
    try {
      const response = { type: name, host: hostname, ip: await getLocalIp(), port };
      if (readDiscovery) {
        Object.assign(response, (await readDiscovery()) || {});
      }
      sock.send(JSON.stringify(response), rinfo.port, rinfo.address);
    } catch (err) {
      console.log(`Discovery reply failed: ${err.message}`);
    }
  });

  sock.bind(UDP_PORT, () => {
    console.log(`UDP discovery listening on port ${UDP_PORT}`);
  });
  return sock;
}

/*
 * Serve read() (may be async; null result -> 503) on GET / over HTTPS
 * with X-Api-Key auth.
 */
function runRestServer(name, apiKey, hostname, read, sslCert, sslKey) {
  const options = {
    cert: fs.readFileSync(sslCert),
    key: fs.readFileSync(sslKey),
  };

  const server = https.createServer(options, async (req, res) => {
    const path = req.url.split('?')[0];
    if (path !== '/') {
      res.writeHead(404).end();
      return;
    }
    if (req.method !== 'GET') {
      res.writeHead(405).end();
      return;
    }
    if (!keyMatches(req.headers['x-api-key'], apiKey)) {
      res.writeHead(401).end();
      return;
    }

    let data = null;
    try {
      data = await read();
    } catch (err) {
      console.log(`Sensor read failed: ${err.message}`);
    }
    if (data === null || data === undefined) {
      res.writeHead(503, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: 'sensor_read_failed' }));
      return;
    }
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({ sensor: name, host: hostname, ...data }));
  });

  server.listen(HTTPS_PORT, () => {
    console.log(`HTTPS server on port ${HTTPS_PORT}`);
  });
  return server;
}

module.exports = {
  HTTPS_PORT,
  WS_PORT,
  UDP_PORT,
  SCAN_KEYWORD,
  getLocalIp,
  keyMatches,
  startDiscoveryListener,
  runRestServer,
};
