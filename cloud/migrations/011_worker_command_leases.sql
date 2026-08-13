-- Attempt-aware at-least-once delivery for worker follow-up commands.
-- delivered_to_worker remains the durable acknowledgement bit for compatibility.
ALTER TABLE run_messages ADD COLUMN delivery_attempt INTEGER NOT NULL DEFAULT 0;
ALTER TABLE run_messages ADD COLUMN delivery_claimed_at TEXT;
ALTER TABLE run_messages ADD COLUMN delivery_claim_attempt_id TEXT;

CREATE INDEX IF NOT EXISTS idx_run_messages_worker_delivery
    ON run_messages(run_id, delivered_to_worker, delivery_claimed_at, created_at);
