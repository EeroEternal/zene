# Zene Cloud — GCP deploy (zene.run)

Hong Kong GCE VM (`asia-east2-b`) + Caddy + Cloudflare DNS.

## One-time provision

```bash
cd cloud/deploy
./firewall.sh
./create-vm.sh
```

Defaults: project `xinference`, zone `asia-east2-b`, instance `zene-cloud`, machine `e2-standard-2`.

Note the printed static IP, then in Cloudflare for `zene.run`:

1. DNS A `@` → static IP, Proxy **on**
2. Optional CNAME `www` → `zene.run`, Proxy on
3. SSL/TLS mode **Full**

Disable or delete the old Cloudflare Pages project `zene-docs` in the dashboard (marketing site removed).

## First deploy

Build on a Linux amd64 host (or use GitHub Actions `deploy-cloud`):

```bash
# from repo root — cloud workspace + CLI
(cd cloud && cargo build --release -p zene-cloud-api -p zene-cloud-worker --locked)
cargo build --release -p zene-cli --locked
(cd cloud/apps/web && npm ci && npm run build)

STAGE=/tmp/zene-cloud-stage
rm -rf "$STAGE" && mkdir -p "$STAGE/bin" "$STAGE/web" "$STAGE/systemd"
cp cloud/target/release/zene-cloud-api cloud/target/release/zene-cloud-worker "$STAGE/bin/"
cp target/release/zene "$STAGE/bin/"
cp -a cloud/apps/web/dist/. "$STAGE/web/"
cp cloud/deploy/systemd/*.service "$STAGE/systemd/"
cp cloud/deploy/Caddyfile cloud/deploy/install-remote.sh "$STAGE/"

# copy to VM then:
# gcloud compute scp --recurse "$STAGE" deploy@zene-cloud:/tmp/zene-cloud-stage --zone=asia-east2-b
# gcloud compute ssh deploy@zene-cloud --zone=asia-east2-b --command='sudo STAGE_DIR=/tmp/zene-cloud-stage bash /tmp/zene-cloud-stage/install-remote.sh'
```

Edit `/etc/zene-cloud.env` on the VM (see `env.example`). Generate a strong `ZENE_CLOUD_WORKER_TOKEN`. For live GitHub:

```text
Setup / callback URL: https://zene.run/api/v1/github/install/callback
```

Place the App private key at `/etc/zene-cloud/github-app.pem` (`chown zene:zene`, `chmod 600`).

## CI

Workflow: `.github/workflows/deploy-cloud.yml`

Repo secrets:

| Secret | Purpose |
|--------|---------|
| `ZENE_CLOUD_SSH_KEY` | Private key for `deploy` user |
| `ZENE_CLOUD_HOST` | Static IP or `zene.run` (direct origin IP preferred if CF proxy blocks SSH) |
| `ZENE_CLOUD_USER` | `deploy` |

Business secrets stay in `/etc/zene-cloud.env` only.
