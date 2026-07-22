#!/usr/bin/env bash
set -euo pipefail

PROJECT="${GCP_PROJECT:-xinference}"
ZONE="${GCP_ZONE:-asia-east2-b}"
INSTANCE="${INSTANCE_NAME:-zene-cloud}"
MACHINE="${MACHINE_TYPE:-e2-standard-2}"
ADDRESS_NAME="${ADDRESS_NAME:-zene-cloud-ip}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

gcloud config set project "$PROJECT"

if ! gcloud compute addresses describe "$ADDRESS_NAME" --region="${ZONE%-*}" --project="$PROJECT" >/dev/null 2>&1; then
  gcloud compute addresses create "$ADDRESS_NAME" \
    --project="$PROJECT" \
    --region="${ZONE%-*}"
fi

STATIC_IP="$(gcloud compute addresses describe "$ADDRESS_NAME" \
  --project="$PROJECT" \
  --region="${ZONE%-*}" \
  --format='get(address)')"

bash "$SCRIPT_DIR/firewall.sh"

if gcloud compute instances describe "$INSTANCE" --zone="$ZONE" --project="$PROJECT" >/dev/null 2>&1; then
  echo "Instance $INSTANCE already exists in $ZONE"
  echo "Static IP: $STATIC_IP"
  exit 0
fi

gcloud compute instances create "$INSTANCE" \
  --project="$PROJECT" \
  --zone="$ZONE" \
  --machine-type="$MACHINE" \
  --image-family=ubuntu-2404-lts-amd64 \
  --image-project=ubuntu-os-cloud \
  --boot-disk-size=50GB \
  --boot-disk-type=pd-balanced \
  --tags=zene-cloud \
  --address="$STATIC_IP" \
  --metadata-from-file=startup-script="$SCRIPT_DIR/startup.sh" \
  --scopes=https://www.googleapis.com/auth/logging.write,https://www.googleapis.com/auth/monitoring.write

echo
echo "Created $INSTANCE"
echo "Static IP: $STATIC_IP"
echo "Point Cloudflare A record for zene.run → $STATIC_IP (proxied)"
echo "Wait ~1–2 min for startup script, then deploy binaries with install-remote.sh / CI"
