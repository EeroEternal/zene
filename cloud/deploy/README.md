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
# from repo root — cloud workspace + CLI + cellz (from crates.io)
(cd cloud && cargo build --release -p zene-cloud-api -p zene-cloud-worker --locked)
cargo build --release -p zene-cli --locked
cargo install cellz --version 0.1.0 --root /tmp/cellz-bin
(cd cloud/apps/web && npm ci && npm run build)

STAGE=/tmp/zene-cloud-stage
rm -rf "$STAGE" && mkdir -p "$STAGE/bin" "$STAGE/web" "$STAGE/systemd"
cp cloud/target/release/zene-cloud-api cloud/target/release/zene-cloud-worker "$STAGE/bin/"
cp target/release/zene target/release/zene-inference-gateway "$STAGE/bin/"
cp /tmp/cellz-bin/bin/cellz "$STAGE/bin/"
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

## Worker event outbox durability

The worker stores unsent runtime events under the configured worker workspace
root in `.event-outbox/<run-id>/`. The outbox is crash-safe for process restarts
on the same durable filesystem: entries are fsynced before publication, use an
atomic hard-link commit, and are removed only after a successful API response.

The outbox is **not** a cross-VM durable store by itself. If worker replacement
can land on a different VM, configure the workspace/outbox root on a durable
shared POSIX volume, or add a separate database/object-backed event spool before
relying on replacement replay across instances. Network filesystems must support
POSIX advisory locks for the current locking strategy.

Operational requirements:

- preserve the outbox root across worker process restarts;
- do not delete `.event-outbox` while a run is active;
- retain completed-run directories until the API has persisted/acknowledged all
  events, then clean them with an explicit retention policy;
- monitor outbox event count/bytes and investigate the 10,000-event or 128 MiB
  backpressure limit before it is reached;
- treat non-retryable event POST failures as retained events requiring operator
  or policy-driven follow-up, not as safe-to-delete data.
