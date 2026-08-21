#!/usr/bin/env node
'use strict';
/* Sensor Playground Grove relay node — 1 / 2 / 4 channels (Node.js). */

const { loadConfig, hostnameOr } = require('../common/config');
const { WS_PORT, startDiscoveryListener } = require('../common/wifi');
const { startWsPushServer } = require('../common/ws_server');
const { RelayController, makeRelayBank, handleJsonCommand } = require('./relay');

const PUBLISH_INTERVAL_MS = 20;

function startStateLoop(relay, publish) {
  return setInterval(() => {
    const payload = relay.takePending();
    if (payload) {
      console.log(`relay: ${JSON.stringify(payload.relay)}`);
      publish(payload);
    }
  }, PUBLISH_INTERVAL_MS);
}

async function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const sensorName = config.sensor_name || 'RELAY';

  console.log(config.emulation
    ? 'Emulation mode: switching virtual relays without hardware'
    : 'Initializing relay node...');
  const bank = await makeRelayBank(config);
  const relay = await RelayController.create(bank);
  console.log(`Relay board: ${relay.count} channel(s)`);

  let stopping = false;
  const shutdown = async () => {
    if (stopping) return;
    stopping = true;
    try {
      await relay.setAll(false);
    } catch (err) {
      console.log(`Relay shutdown failed: ${err.message}`);
    }
    try {
      await relay.close();
    } catch (err) {
      console.log(`Relay close failed: ${err.message}`);
    }
    console.log('Stopped.');
    process.exit(0);
  };
  process.on('SIGINT', shutdown);
  process.on('SIGTERM', shutdown);

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const { broadcast } = startWsPushServer(
    config.api_key,
    (send) => send(relay.payload()),
    (message) => handleJsonCommand(relay, message)
  );
  startDiscoveryListener(sensorName, hostname, WS_PORT, () => ({ channels: relay.count }));
  startStateLoop(relay, broadcast);
}

if (require.main === module) {
  main().catch((err) => {
    console.log(`Error: ${err.message}`);
    process.exit(1);
  });
}

module.exports = { main, startStateLoop };
