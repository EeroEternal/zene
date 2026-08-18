#!/usr/bin/env bash
# GCE startup script: install Caddy, create zene user and directories.
set -euo pipefail

export DEBIAN_FRONTEND=noninteractive

if ! id -u zene >/dev/null 2>&1; then
  useradd --system --create-home --home-dir /var/lib/zene-cloud --shell /usr/sbin/nologin zene
fi

mkdir -p /opt/zene-cloud/bin /opt/zene-cloud/web /var/lib/zene-cloud/workspaces /etc/zene-cloud
chown -R zene:zene /opt/zene-cloud /var/lib/zene-cloud

if [[ ! -f /etc/zene-cloud.env ]]; then
  WORKER_TOKEN="$(openssl rand -hex 32)"
  cat >/etc/zene-cloud.env <<EOF
ZENE_CLOUD_BIND=127.0.0.1:8788
ZENE_CLOUD_DATABASE_URL=sqlite:/var/lib/zene-cloud/zene-cloud.db
ZENE_CLOUD_WORKER_TOKEN=${WORKER_TOKEN}
ZENE_CLOUD_WEB_DIR=/opt/zene-cloud/web
ZENE_CLOUD_WORKSPACE_ROOT=/var/lib/zene-cloud/workspaces
ZENE_CLOUD_API_URL=http://127.0.0.1:8788
ZENE_CLOUD_PUBLIC_BASE_URL=https://zene.run
ZENE_CLOUD_GITHUB_MODE=live
ZENE_CLOUD_PUSH_PR=false
ZENE_CLOUD_ACP_YOLO=true
ZENE_BIN=/opt/zene-cloud/bin/zene
EOF
  chmod 600 /etc/zene-cloud.env
  chown root:zene /etc/zene-cloud.env
fi

if ! command -v caddy >/dev/null 2>&1; then
  apt-get update -y
  apt-get install -y debian-keyring debian-archive-keyring apt-transport-https curl ca-certificates
  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' \
    | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' \
    | tee /etc/apt/sources.list.d/caddy-stable.list >/dev/null
  apt-get update -y
  apt-get install -y caddy git
fi

if ! command -v redis-server >/dev/null 2>&1; then
  apt-get update -y
  apt-get install -y redis-server
fi
if [[ -f /etc/redis/redis.conf ]]; then
  sed -i 's/^bind 127\.0\.0\.1 .*/bind 127.0.0.1 -::1/' /etc/redis/redis.conf 2>/dev/null || true
  sed -i 's/^# bind 127\.0\.0\.1 .*/bind 127.0.0.1 -::1/' /etc/redis/redis.conf 2>/dev/null || true
fi
systemctl enable redis-server 2>/dev/null || true
systemctl start redis-server 2>/dev/null || true

# Deploy user for CI SSH (authorized_keys added separately).
if ! id -u deploy >/dev/null 2>&1; then
  useradd --create-home --shell /bin/bash deploy
  usermod -aG sudo deploy
  echo 'deploy ALL=(ALL) NOPASSWD:ALL' >/etc/sudoers.d/deploy
  chmod 440 /etc/sudoers.d/deploy
  mkdir -p /home/deploy/.ssh
  chmod 700 /home/deploy/.ssh
  chown -R deploy:deploy /home/deploy/.ssh
fi

touch /var/lib/zene-cloud/.startup-done
