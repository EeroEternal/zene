#!/usr/bin/env bash
# Run on the VM (or via SSH from CI) after artifacts are staged under STAGE_DIR.
# Usage: sudo STAGE_DIR=/tmp/zene-cloud-stage bash install-remote.sh
set -euo pipefail

STAGE_DIR="${STAGE_DIR:-/tmp/zene-cloud-stage}"
OPT_DIR="/opt/zene-cloud"
UNIT_SRC="${UNIT_SRC:-$STAGE_DIR/systemd}"
CADDY_SRC="${CADDY_SRC:-$STAGE_DIR/Caddyfile}"

if [[ ! -d "$STAGE_DIR/bin" ]]; then
  echo "Missing $STAGE_DIR/bin (expected zene-cloud-api, zene-cloud-worker, zene-inference-gateway, zene)" >&2
  exit 1
fi

install -d -m 755 "$OPT_DIR/bin" "$OPT_DIR/web"
install -m 755 "$STAGE_DIR/bin/zene-cloud-api" "$OPT_DIR/bin/zene-cloud-api"
install -m 755 "$STAGE_DIR/bin/zene-cloud-worker" "$OPT_DIR/bin/zene-cloud-worker"
if [[ -f "$STAGE_DIR/bin/zene-inference-gateway" ]]; then
  install -m 755 "$STAGE_DIR/bin/zene-inference-gateway" "$OPT_DIR/bin/zene-inference-gateway"
fi
if [[ -f "$STAGE_DIR/bin/zene" ]]; then
  install -m 755 "$STAGE_DIR/bin/zene" "$OPT_DIR/bin/zene"
fi
if [[ -f "$STAGE_DIR/bin/cellz" ]]; then
  install -m 755 "$STAGE_DIR/bin/cellz" "$OPT_DIR/bin/cellz"
fi

if [[ -d "$STAGE_DIR/web" ]]; then
  rm -rf "$OPT_DIR/web"
  mkdir -p "$OPT_DIR/web"
  cp -a "$STAGE_DIR/web/." "$OPT_DIR/web/"
fi

if [[ -d "$UNIT_SRC" ]]; then
  install -m 644 "$UNIT_SRC/zene-cloud-api.service" /etc/systemd/system/zene-cloud-api.service
  install -m 644 "$UNIT_SRC/zene-cloud-worker.service" /etc/systemd/system/zene-cloud-worker.service
  if [[ -f "$UNIT_SRC/zene-inference-gateway.service" ]]; then
    install -m 644 "$UNIT_SRC/zene-inference-gateway.service" /etc/systemd/system/zene-inference-gateway.service
  fi
  if [[ -f "$UNIT_SRC/cellz.service" ]]; then
    install -m 644 "$UNIT_SRC/cellz.service" /etc/systemd/system/cellz.service
  fi
fi

if [[ -f "$CADDY_SRC" ]]; then
  install -m 644 "$CADDY_SRC" /etc/caddy/Caddyfile
fi

chown -R zene:zene "$OPT_DIR" /var/lib/zene-cloud
mkdir -p /var/lib/zene-cloud/workspaces /var/lib/zene-cloud/cells
chown -R zene:zene /var/lib/zene-cloud

systemctl daemon-reload
systemctl enable zene-cloud-api zene-cloud-worker caddy
if [[ -f /etc/systemd/system/cellz.service ]]; then
  systemctl enable cellz
  systemctl restart cellz
fi
if [[ -f /etc/systemd/system/zene-inference-gateway.service ]]; then
  systemctl enable zene-inference-gateway
  systemctl restart zene-inference-gateway
fi
systemctl restart zene-cloud-api
systemctl restart zene-cloud-worker
systemctl reload caddy || systemctl restart caddy

systemctl --no-pager --full status zene-cloud-api zene-cloud-worker caddy || true
if [[ -f /etc/systemd/system/zene-inference-gateway.service ]]; then
  systemctl --no-pager --full status zene-inference-gateway || true
fi
echo "Install complete."
