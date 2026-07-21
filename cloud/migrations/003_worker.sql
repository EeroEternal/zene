-- Worker inbox / delivery cursor for follow-up prompts.
ALTER TABLE run_messages ADD COLUMN delivered_to_worker INTEGER NOT NULL DEFAULT 0;

-- Optional clone credential cache (Phase 0: usually empty / mock).
CREATE TABLE IF NOT EXISTS run_clone_credentials (
    run_id TEXT PRIMARY KEY NOT NULL,
    clone_url TEXT NOT NULL,
    token_enc TEXT,
    username TEXT,
    mock INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL
);
