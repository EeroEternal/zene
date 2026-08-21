CREATE TABLE IF NOT EXISTS user_llm_providers (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL,
    provider_id TEXT NOT NULL DEFAULT 'custom',
    name TEXT NOT NULL DEFAULT '',
    base_url TEXT NOT NULL DEFAULT '',
    api_key TEXT NOT NULL DEFAULT '',
    default_model TEXT NOT NULL DEFAULT '',
    models_json TEXT NOT NULL DEFAULT '[]',
    is_default INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_user_llm_providers_user
    ON user_llm_providers(user_id);

INSERT INTO user_llm_providers (
    id, user_id, provider_id, name, base_url, api_key, default_model, models_json, is_default, created_at, updated_at
)
SELECT
    lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)), 2) || '-a' || substr(hex(randomblob(2)), 2) || '-' || hex(randomblob(6))),
    user_id,
    provider_id,
    CASE
        WHEN provider_id = 'deepseek' THEN 'DeepSeek'
        WHEN provider_id = 'openai' THEN 'OpenAI'
        WHEN provider_id = 'kimi' THEN 'Kimi'
        WHEN provider_id = 'glm' THEN 'GLM'
        WHEN provider_id = 'qwen' THEN 'Qwen'
        WHEN base_url LIKE '%smartgate%' THEN 'SmartGate'
        ELSE 'Custom'
    END,
    base_url,
    api_key,
    default_model,
    models_json,
    1,
    updated_at,
    updated_at
FROM user_llm_settings
WHERE (base_url != '' OR api_key != '' OR default_model != '');
