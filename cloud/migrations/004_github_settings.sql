CREATE TABLE IF NOT EXISTS github_provider_config (
    organization_id TEXT PRIMARY KEY NOT NULL,
    mode TEXT NOT NULL DEFAULT 'live',
    client_id TEXT,
    client_secret TEXT,
    app_id TEXT,
    app_private_key TEXT,
    app_slug TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
