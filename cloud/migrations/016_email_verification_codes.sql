CREATE TABLE IF NOT EXISTS email_verification_codes (
    id TEXT PRIMARY KEY NOT NULL,
    email TEXT NOT NULL,
    code_hash TEXT NOT NULL,
    purpose TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    consumed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_email_verification_codes_email
    ON email_verification_codes(email);
