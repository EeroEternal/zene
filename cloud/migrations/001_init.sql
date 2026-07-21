CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY NOT NULL,
    email TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS organizations (
    id TEXT PRIMARY KEY NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS organization_members (
    organization_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    role TEXT NOT NULL,
    joined_at TEXT NOT NULL,
    PRIMARY KEY (organization_id, user_id)
);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS repositories (
    id TEXT PRIMARY KEY NOT NULL,
    organization_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    owner TEXT NOT NULL,
    name TEXT NOT NULL,
    default_branch TEXT NOT NULL,
    clone_url TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS runs (
    id TEXT PRIMARY KEY NOT NULL,
    organization_id TEXT NOT NULL,
    repository_id TEXT NOT NULL,
    requested_by TEXT NOT NULL,
    status TEXT NOT NULL,
    status_version INTEGER NOT NULL,
    title TEXT NOT NULL,
    prompt TEXT NOT NULL,
    base_ref TEXT NOT NULL,
    base_sha TEXT,
    head_branch TEXT NOT NULL,
    head_sha TEXT,
    model TEXT NOT NULL,
    permission_mode TEXT NOT NULL,
    created_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT
);

CREATE TABLE IF NOT EXISTS run_attempts (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    generation INTEGER NOT NULL,
    worker_id TEXT,
    status TEXT NOT NULL,
    lease_expires_at TEXT,
    failure_code TEXT,
    started_at TEXT,
    finished_at TEXT
);

CREATE TABLE IF NOT EXISTS run_messages (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL,
    author_id TEXT,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    client_message_id TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS run_events (
    run_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    attempt_generation INTEGER NOT NULL DEFAULT 0,
    source_event_id TEXT,
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (run_id, seq)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_run_events_source
    ON run_events(run_id, attempt_generation, source_event_id)
    WHERE source_event_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS worker_tokens (
    id TEXT PRIMARY KEY NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS idempotency_records (
    scope TEXT NOT NULL,
    key TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    response_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    PRIMARY KEY (scope, key)
);
