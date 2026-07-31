#!/bin/bash
#
# Generates a self-signed SSL certificate and writes SERVER_CERT and
# SERVER_KEY into secrets.h in the correct C string format.
#
# Usage:  ./generate_cert.sh
#
# Prerequisites:
#   - openssl and python3 must be installed
#   - secrets.h must already exist (copy from secrets.h.example first)

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SECRETS="$SCRIPT_DIR/secrets.h"
CERT_FILE=$(mktemp)
KEY_FILE=$(mktemp)
trap 'rm -f "$CERT_FILE" "$KEY_FILE"' EXIT

if [ ! -f "$SECRETS" ]; then
  echo "Error: secrets.h not found."
  echo "Copy secrets.h.example to secrets.h first."
  exit 1
fi

echo "Generating self-signed certificate..."
openssl req -x509 -nodes -days 3650 -newkey rsa:2048 \
  -keyout "$KEY_FILE" -out "$CERT_FILE" \
  -subj "/CN=SensorTester" 2>/dev/null

python3 - "$SECRETS" "$CERT_FILE" "$KEY_FILE" << 'PYEOF'
import re, sys

secrets_path, cert_path, key_path = sys.argv[1], sys.argv[2], sys.argv[3]

def pem_to_c(path):
    lines = []
    with open(path) as f:
        for line in f:
            line = line.rstrip('\n')
            lines.append('  "' + line + r'\n' + '"')
    return '\n'.join(lines)

with open(secrets_path) as f:
    content = f.read()

cert_c = 'static const char SERVER_CERT[] =\n' + pem_to_c(cert_path) + ';'
key_c = 'static const char SERVER_KEY[] =\n' + pem_to_c(key_path) + ';'

content = re.sub(
    r'static const char SERVER_CERT\[\][\s\S]*?;',
    lambda m: cert_c,
    content,
    count=1
)
content = re.sub(
    r'static const char SERVER_KEY\[\][\s\S]*?;',
    lambda m: key_c,
    content,
    count=1
)

with open(secrets_path, 'w') as f:
    f.write(content)
PYEOF

echo "Done. SERVER_CERT and SERVER_KEY updated in secrets.h"
