use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;
use zene_cloud_domain::{
    CreateLlmProviderRequest, UpdateLlmProviderRequest, UpdateLlmSettingsRequest, UserLlmProvider,
    UserLlmSettings,
};

use crate::{parse_time, Db};

impl Db {
    pub async fn list_user_llm_providers(&self, user_id: Uuid) -> Result<Vec<UserLlmProvider>> {
        let rows: Vec<(
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            i64,
            String,
            String,
        )> = sqlx::query_as(
            "SELECT id, user_id, provider_id, name, base_url, api_key, default_model, models_json, is_default, created_at, updated_at
             FROM user_llm_providers WHERE user_id = ? ORDER BY is_default DESC, created_at ASC",
        )
        .bind(user_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    user_id,
                    provider_id,
                    name,
                    base_url,
                    api_key,
                    default_model,
                    models_json,
                    is_default,
                    created_at,
                    updated_at,
                )| {
                    let models: Vec<String> =
                        serde_json::from_str(&models_json).unwrap_or_default();
                    UserLlmProvider {
                        id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::nil()),
                        user_id: Uuid::parse_str(&user_id).unwrap_or_else(|_| Uuid::nil()),
                        provider_id,
                        name,
                        base_url,
                        api_key,
                        default_model,
                        models,
                        is_default: is_default != 0,
                        created_at: parse_time(&created_at),
                        updated_at: parse_time(&updated_at),
                    }
                },
            )
            .collect())
    }

    pub async fn get_user_llm_provider(
        &self,
        user_id: Uuid,
        provider_id: Uuid,
    ) -> Result<Option<UserLlmProvider>> {
        let row: Option<(
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            i64,
            String,
            String,
        )> = sqlx::query_as(
            "SELECT id, user_id, provider_id, name, base_url, api_key, default_model, models_json, is_default, created_at, updated_at
             FROM user_llm_providers WHERE user_id = ? AND id = ?",
        )
        .bind(user_id.to_string())
        .bind(provider_id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(
            |(
                id,
                user_id,
                provider_id,
                name,
                base_url,
                api_key,
                default_model,
                models_json,
                is_default,
                created_at,
                updated_at,
            )| {
                let models: Vec<String> = serde_json::from_str(&models_json).unwrap_or_default();
                UserLlmProvider {
                    id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::nil()),
                    user_id: Uuid::parse_str(&user_id).unwrap_or_else(|_| Uuid::nil()),
                    provider_id,
                    name,
                    base_url,
                    api_key,
                    default_model,
                    models,
                    is_default: is_default != 0,
                    created_at: parse_time(&created_at),
                    updated_at: parse_time(&updated_at),
                }
            },
        ))
    }

    pub async fn create_user_llm_provider(
        &self,
        user_id: Uuid,
        req: CreateLlmProviderRequest,
    ) -> Result<UserLlmProvider> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let provider_id = req.provider_id.trim().to_string();
        let name = req
            .name
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| match provider_id.as_str() {
                "deepseek" => "DeepSeek".into(),
                "openai" => "OpenAI".into(),
                "kimi" => "Kimi".into(),
                "glm" => "GLM".into(),
                "qwen" => "Qwen".into(),
                _ if req.base_url.contains("smartgate") => "SmartGate".into(),
                _ => "Custom".into(),
            });
        let base_url = req.base_url.trim().to_string();
        let default_model = req.default_model.trim().to_string();
        let mut models: Vec<String> = req
            .models
            .into_iter()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .collect();
        if !default_model.is_empty() && !models.contains(&default_model) {
            models.insert(0, default_model.clone());
        }
        let api_key = req
            .api_key
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let models_json = serde_json::to_string(&models).unwrap_or_else(|_| "[]".into());

        let existing = self.list_user_llm_providers(user_id).await?;
        let is_default = req.is_default || existing.is_empty();

        if is_default {
            sqlx::query("UPDATE user_llm_providers SET is_default = 0 WHERE user_id = ?")
                .bind(user_id.to_string())
                .execute(&self.pool)
                .await?;
        }

        sqlx::query(
            "INSERT INTO user_llm_providers
             (id, user_id, provider_id, name, base_url, api_key, default_model, models_json, is_default, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(&provider_id)
        .bind(&name)
        .bind(&base_url)
        .bind(&api_key)
        .bind(&default_model)
        .bind(&models_json)
        .bind(if is_default { 1 } else { 0 })
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;

        // Also sync legacy user_llm_settings for fallback compatibility
        let _ = self
            .upsert_user_llm_settings(
                user_id,
                UpdateLlmSettingsRequest {
                    provider_id: provider_id.clone(),
                    base_url: base_url.clone(),
                    default_model: default_model.clone(),
                    models: models.clone(),
                    api_key: if api_key.is_empty() {
                        None
                    } else {
                        Some(api_key.clone())
                    },
                },
            )
            .await;

        Ok(UserLlmProvider {
            id,
            user_id,
            provider_id,
            name,
            base_url,
            api_key,
            default_model,
            models,
            is_default,
            created_at: now,
            updated_at: now,
        })
    }

    pub async fn update_user_llm_provider(
        &self,
        user_id: Uuid,
        provider_id: Uuid,
        req: UpdateLlmProviderRequest,
    ) -> Result<UserLlmProvider> {
        let existing = self
            .get_user_llm_provider(user_id, provider_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("provider not found"))?;

        let now = Utc::now();
        let kind = req
            .provider_id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or(existing.provider_id);
        let name = req
            .name
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or(existing.name);
        let base_url = req
            .base_url
            .map(|s| s.trim().to_string())
            .unwrap_or(existing.base_url);
        let default_model = req
            .default_model
            .map(|s| s.trim().to_string())
            .unwrap_or(existing.default_model);

        let mut models = req.models.unwrap_or(existing.models);
        models = models
            .into_iter()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .collect();
        if !default_model.is_empty() && !models.contains(&default_model) {
            models.insert(0, default_model.clone());
        }

        let api_key = req
            .api_key
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or(existing.api_key);

        let is_default = req.is_default.unwrap_or(existing.is_default);
        if is_default && !existing.is_default {
            sqlx::query("UPDATE user_llm_providers SET is_default = 0 WHERE user_id = ?")
                .bind(user_id.to_string())
                .execute(&self.pool)
                .await?;
        }

        let models_json = serde_json::to_string(&models).unwrap_or_else(|_| "[]".into());

        sqlx::query(
            "UPDATE user_llm_providers
             SET provider_id = ?, name = ?, base_url = ?, api_key = ?, default_model = ?,
                 models_json = ?, is_default = ?, updated_at = ?
             WHERE user_id = ? AND id = ?",
        )
        .bind(&kind)
        .bind(&name)
        .bind(&base_url)
        .bind(&api_key)
        .bind(&default_model)
        .bind(&models_json)
        .bind(if is_default { 1 } else { 0 })
        .bind(now.to_rfc3339())
        .bind(user_id.to_string())
        .bind(provider_id.to_string())
        .execute(&self.pool)
        .await?;

        if is_default {
            let _ = self
                .upsert_user_llm_settings(
                    user_id,
                    UpdateLlmSettingsRequest {
                        provider_id: kind.clone(),
                        base_url: base_url.clone(),
                        default_model: default_model.clone(),
                        models: models.clone(),
                        api_key: if api_key.is_empty() {
                            None
                        } else {
                            Some(api_key.clone())
                        },
                    },
                )
                .await;
        }

        Ok(UserLlmProvider {
            id: provider_id,
            user_id,
            provider_id: kind,
            name,
            base_url,
            api_key,
            default_model,
            models,
            is_default,
            created_at: existing.created_at,
            updated_at: now,
        })
    }

    pub async fn delete_user_llm_provider(&self, user_id: Uuid, provider_id: Uuid) -> Result<()> {
        let existing = self.get_user_llm_provider(user_id, provider_id).await?;
        if let Some(p) = existing {
            sqlx::query("DELETE FROM user_llm_providers WHERE user_id = ? AND id = ?")
                .bind(user_id.to_string())
                .bind(provider_id.to_string())
                .execute(&self.pool)
                .await?;

            if p.is_default {
                // Pick another provider as default if available
                let remaining = self.list_user_llm_providers(user_id).await?;
                if let Some(first) = remaining.first() {
                    sqlx::query(
                        "UPDATE user_llm_providers SET is_default = 1 WHERE user_id = ? AND id = ?",
                    )
                    .bind(user_id.to_string())
                    .bind(first.id.to_string())
                    .execute(&self.pool)
                    .await?;
                }
            }
        }
        Ok(())
    }

    pub async fn resolve_user_llm_provider_for_model(
        &self,
        user_id: Uuid,
        model_name: &str,
    ) -> Result<Option<UserLlmProvider>> {
        let providers = self.list_user_llm_providers(user_id).await?;
        if providers.is_empty() {
            // Fallback to legacy user_llm_settings
            let legacy = self.get_user_llm_settings(user_id).await?;
            return Ok(legacy.map(|s| UserLlmProvider {
                id: Uuid::nil(),
                user_id: s.user_id,
                provider_id: s.provider_id,
                name: "Default".into(),
                base_url: s.base_url,
                api_key: s.api_key,
                default_model: s.default_model,
                models: s.models,
                is_default: true,
                created_at: s.updated_at,
                updated_at: s.updated_at,
            }));
        }

        let m = model_name.trim();
        if !m.is_empty() && m != "default" {
            // Match provider whose models list contains the requested model
            if let Some(matched) = providers.iter().find(|p| {
                p.models.iter().any(|model| model.eq_ignore_ascii_case(m))
                    || p.default_model.eq_ignore_ascii_case(m)
            }) {
                return Ok(Some(matched.clone()));
            }
        }

        // Return default provider or the first one
        let default = providers
            .iter()
            .find(|p| p.is_default)
            .or_else(|| providers.first())
            .cloned();
        Ok(default)
    }

    pub async fn get_user_llm_settings(&self, user_id: Uuid) -> Result<Option<UserLlmSettings>> {
        let row: Option<(String, String, String, String, String, String, String)> = sqlx::query_as(
            "SELECT user_id, provider_id, base_url, api_key, default_model, models_json, updated_at
             FROM user_llm_settings WHERE user_id = ?",
        )
        .bind(user_id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(
            |(user_id, provider_id, base_url, api_key, default_model, models_json, updated_at)| {
                let models: Vec<String> = serde_json::from_str(&models_json).unwrap_or_default();
                UserLlmSettings {
                    user_id: Uuid::parse_str(&user_id).unwrap_or_else(|_| Uuid::nil()),
                    provider_id,
                    base_url,
                    api_key,
                    default_model,
                    models,
                    updated_at: parse_time(&updated_at),
                }
            },
        ))
    }

    pub async fn upsert_user_llm_settings(
        &self,
        user_id: Uuid,
        req: UpdateLlmSettingsRequest,
    ) -> Result<UserLlmSettings> {
        let now = Utc::now();
        let existing = self.get_user_llm_settings(user_id).await?;
        let provider_id = req.provider_id.trim().to_string();
        let base_url = req.base_url.trim().to_string();
        let default_model = req.default_model.trim().to_string();
        let models: Vec<String> = req
            .models
            .into_iter()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .collect();
        let api_key = req
            .api_key
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| existing.as_ref().map(|e| e.api_key.clone()))
            .unwrap_or_default();
        let models_json = serde_json::to_string(&models).unwrap_or_else(|_| "[]".into());

        if existing.is_some() {
            sqlx::query(
                "UPDATE user_llm_settings
                 SET provider_id = ?, base_url = ?, api_key = ?, default_model = ?,
                     models_json = ?, updated_at = ?
                 WHERE user_id = ?",
            )
            .bind(&provider_id)
            .bind(&base_url)
            .bind(&api_key)
            .bind(&default_model)
            .bind(&models_json)
            .bind(now.to_rfc3339())
            .bind(user_id.to_string())
            .execute(&self.pool)
            .await?;
        } else {
            sqlx::query(
                "INSERT INTO user_llm_settings
                 (user_id, provider_id, base_url, api_key, default_model, models_json, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(user_id.to_string())
            .bind(&provider_id)
            .bind(&base_url)
            .bind(&api_key)
            .bind(&default_model)
            .bind(&models_json)
            .bind(now.to_rfc3339())
            .execute(&self.pool)
            .await?;
        }

        Ok(UserLlmSettings {
            user_id,
            provider_id,
            base_url,
            api_key,
            default_model,
            models,
            updated_at: now,
        })
    }
}
