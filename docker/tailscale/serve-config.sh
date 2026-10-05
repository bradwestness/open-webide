#!/bin/sh
# Emit a persistent Serve config for the enrolled node's full MagicDNS name.
set -eu
host=${1:?Usage: sh docker/tailscale/serve-config.sh device.tailnet.ts.net}
case "$host" in
  *[!a-zA-Z0-9.-]*|.*|*..*|*.) echo 'Use the full MagicDNS hostname without a scheme or port.' >&2; exit 1 ;;
  *.ts.net) ;;
  *) echo 'Use the full MagicDNS hostname ending in .ts.net.' >&2; exit 1 ;;
esac
cat <<JSON
{
  "TCP": {"443": {"HTTPS": true}},
  "Web": {
    "$host:443": {
      "Handlers": {
        "/": {"Proxy": "http://127.0.0.1:3000"},
        "/bridge": {"Proxy": "http://127.0.0.1:3001"},
        "/bridge/secret": {"Text": "Not available through the proxy"}
      }
    }
  },
  "AllowFunnel": {"$host:443": false}
}
JSON
