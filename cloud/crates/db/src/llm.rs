use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;
use zene_cloud_domain::{UpdateLlmSettingsRequest, UserLlmSettings};

use crate::{parse_time, Db};

impl Db {
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
