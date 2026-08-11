CREATE TABLE IF NOT EXISTS oauth_states (
    state TEXT PRIMARY KEY NOT NULL,
    user_id TEXT,
    redirect_to TEXT,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS github_accounts (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL UNIQUE,
    github_user_id TEXT NOT NULL,
    login TEXT NOT NULL,
    access_token_enc TEXT NOT NULL,
    token_type TEXT NOT NULL DEFAULT 'bearer',
    scope TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS github_installations (
    id TEXT PRIMARY KEY NOT NULL,
    organization_id TEXT NOT NULL,
    installation_id TEXT NOT NULL UNIQUE,
    account_login TEXT NOT NULL,
    account_type TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS approval_requests (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL,
    request_key TEXT NOT NULL,
    jsonrpc_id TEXT,
    kind TEXT NOT NULL,
    risk TEXT NOT NULL DEFAULT 'medium',
    payload_json TEXT NOT NULL,
    status TEXT NOT NULL,
    allowed_decisions TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT,
    resolved_by TEXT,
    resolved_at TEXT,
    decision TEXT,
    UNIQUE(run_id, request_key)
);

CREATE TABLE IF NOT EXISTS git_operations (
    id TEXT PRIMARY KEY NOT NULL,
    organization_id TEXT NOT NULL,
    repository_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    operation TEXT NOT NULL,
    expected_head_sha TEXT,
    result_head_sha TEXT,
    approval_id TEXT,
    status TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    provider_request_id TEXT,
    result_json TEXT,
    created_at TEXT NOT NULL,
    finished_at TEXT,
    UNIQUE(repository_id, idempotency_key)
);

CREATE TABLE IF NOT EXISTS pull_requests (
    id TEXT PRIMARY KEY NOT NULL,
    repository_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    provider_number INTEGER,
    url TEXT,
    title TEXT NOT NULL,
    body TEXT,
    base_sha TEXT,
    head_sha TEXT,
    state TEXT NOT NULL,
    draft INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS audit_logs (
    id TEXT PRIMARY KEY NOT NULL,
    organization_id TEXT,
    actor_type TEXT NOT NULL,
    actor_id TEXT,
    action TEXT NOT NULL,
    resource_type TEXT,
    resource_id TEXT,
    metadata_json TEXT,
    created_at TEXT NOT NULL
);

ALTER TABLE repositories ADD COLUMN installation_id TEXT;
ALTER TABLE repositories ADD COLUMN provider_repo_id TEXT;
ALTER TABLE repositories ADD COLUMN private INTEGER NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN clone_status TEXT;
ALTER TABLE runs ADD COLUMN last_error TEXT;
