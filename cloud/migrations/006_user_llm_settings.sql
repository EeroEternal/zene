CREATE TABLE IF NOT EXISTS user_llm_settings (
    user_id TEXT PRIMARY KEY NOT NULL,
    provider_id TEXT NOT NULL DEFAULT 'custom',
    base_url TEXT NOT NULL DEFAULT '',
    api_key TEXT NOT NULL DEFAULT '',
    default_model TEXT NOT NULL DEFAULT '',
    models_json TEXT NOT NULL DEFAULT '[]',
    updated_at TEXT NOT NULL
);
