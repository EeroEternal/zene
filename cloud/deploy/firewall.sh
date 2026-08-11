#!/usr/bin/env bash
set -euo pipefail

PROJECT="${GCP_PROJECT:-xinference}"
RULE_NAME="${FIREWALL_RULE:-allow-zene-cloud-http}"

gcloud compute firewall-rules describe "$RULE_NAME" --project="$PROJECT" >/dev/null 2>&1 && {
  echo "Firewall rule $RULE_NAME already exists."
  exit 0
}

gcloud compute firewall-rules create "$RULE_NAME" \
  --project="$PROJECT" \
  --direction=INGRESS \
  --priority=1000 \
  --network=default \
  --action=ALLOW \
  --rules=tcp:80,tcp:443 \
  --source-ranges=0.0.0.0/0 \
  --target-tags=zene-cloud \
  --description="HTTP/HTTPS for Zene Cloud (Caddy)"

echo "Created firewall rule $RULE_NAME"
