'use strict';
/*
 * Shared config.json loading for the Node.js Sensor Playground nodes —
 * same behavior as python/common/wifi_transport.load_config.
 */

const fs = require('fs');
const os = require('os');
const path = require('path');

/** Load config.json from the node's own directory; exit(1) when missing. */
function loadConfig(nodeDir) {
  const configPath = path.join(nodeDir, 'config.json');
  if (!fs.existsSync(configPath)) {
    console.log('Error: config.json not found.\nCopy config.example.json to config.json and edit it.');
    process.exit(1);
  }
  const config = JSON.parse(fs.readFileSync(configPath, 'utf8'));
  if (!config.api_key) {
    console.log('Error: config.json: api_key is required');
    process.exit(1);
  }
  return config;
}

/** The configured hostname, or the system hostname when empty. */
function hostnameOr(h) {
  return h || os.hostname();
}

module.exports = { loadConfig, hostnameOr };
