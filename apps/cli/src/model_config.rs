use llm_providers::{get_endpoint, get_model_for_endpoint, get_providers_data, Model};
use zene_config::ZeneConfig;
use zene_core::Agent;

#[derive(Debug, Clone, Copy)]
pub struct ModelVariant {
    pub display_name: &'static str,
    pub model_id: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderPreset {
    pub display_name: &'static str,
    /// `llm_providers` endpoint id (`provider:region`), e.g. `deepseek:cn`.
    pub endpoint_id: Option<&'static str>,
    /// Zene wire protocol (`openai` / `anthropic`).
    pub provider: &'static str,
    pub requires_key: bool,
    /// Models for local presets not in `llm_providers` (e.g. Ollama).
    pub local_models: Option<&'static [ModelVariant]>,
}

const OLLAMA_MODELS: &[ModelVariant] = &[ModelVariant {
    display_name: "qwen2.5-coder",
    model_id: "qwen2.5-coder",
}];

/// Curated endpoints shown in `/provider` and model pickers.
pub const PROVIDER_PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        display_name: "DeepSeek",
        endpoint_id: Some("deepseek:cn"),
        provider: "openai",
        requires_key: true,
        local_models: None,
    },
    ProviderPreset {
        display_name: "Kimi (Moonshot)",
        endpoint_id: Some("moonshot:cn"),
        provider: "openai",
        requires_key: true,
        local_models: None,
    },
    ProviderPreset {
        display_name: "GLM (Zhipu)",
        endpoint_id: Some("zhipu:cn"),
        provider: "openai",
        requires_key: true,
        local_models: None,
    },
    ProviderPreset {
        display_name: "OpenAI",
        endpoint_id: Some("openai:global"),
        provider: "openai",
        requires_key: true,
        local_models: None,
    },
    ProviderPreset {
        display_name: "Anthropic",
        endpoint_id: Some("anthropic:global"),
        provider: "anthropic",
        requires_key: true,
        local_models: None,
    },
    ProviderPreset {
        display_name: "Ollama (Local)",
        endpoint_id: None,
        provider: "openai",
        requires_key: false,
        local_models: Some(OLLAMA_MODELS),
    },
];

#[derive(Debug, Clone)]
pub struct ResolvedModel {
    pub model: String,
    pub provider: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
}

pub fn normalize_base_url(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

/// OpenAI-compatible bases may omit or include `/v1`.
fn openai_compatible_base_key(url: &str) -> String {
    let normalized = normalize_base_url(url);
    normalized
        .strip_suffix("/v1")
        .unwrap_or(normalized.as_str())
        .to_string()
}

pub fn preset_base_url(preset: &ProviderPreset) -> &'static str {
    match preset.endpoint_id {
        Some(id) => get_endpoint(id)
            .map(|(_, ep)| ep.base_url)
            .unwrap_or(""),
        None => "http://localhost:11434/v1",
    }
}

fn models_for_endpoint(endpoint_id: &str) -> &'static [Model] {
    let Some((family_id, endpoint_key)) = endpoint_id.split_once(':') else {
        return &[];
    };
    let Some(provider) = get_providers_data().get(family_id) else {
        return &[];
    };
    if let Some(endpoint_models) = provider.endpoint_models {
        if let Some(models) = endpoint_models.get(endpoint_key) {
            return models;
        }
    }
    provider.models
}

pub fn preset_models(preset: &ProviderPreset) -> Vec<ModelVariant> {
    if let Some(local) = preset.local_models {
        return local.to_vec();
    }
    let Some(endpoint_id) = preset.endpoint_id else {
        return Vec::new();
    };
    models_for_endpoint(endpoint_id)
        .iter()
        .map(|m| ModelVariant {
            display_name: m.name,
            model_id: m.id,
        })
        .collect()
}

pub fn provider_index_for_config(config: &ZeneConfig) -> Option<usize> {
    let current = openai_compatible_base_key(&config.base_url);
    PROVIDER_PRESETS.iter().position(|p| {
        openai_compatible_base_key(preset_base_url(p)) == current
    })
}

pub fn current_provider(config: &ZeneConfig) -> Option<&'static ProviderPreset> {
    provider_index_for_config(config).map(|i| &PROVIDER_PRESETS[i])
}

pub fn is_provider_configured(config: &ZeneConfig) -> bool {
    provider_index_for_config(config).is_some()
}

pub fn is_provider_ready(config: &ZeneConfig) -> bool {
    let Some(index) = provider_index_for_config(config) else {
        return false;
    };
    has_api_key_for_provider(config, &PROVIDER_PRESETS[index])
}

pub fn model_index_for_provider(provider: &ProviderPreset, model_id: &str) -> usize {
    preset_models(provider)
        .iter()
        .position(|v| v.model_id == model_id)
        .unwrap_or(0)
}

pub fn default_model_for_provider(provider: &ProviderPreset, current_model: &str) -> String {
    let models = preset_models(provider);
    if models.iter().any(|v| v.model_id == current_model) {
        current_model.to_string()
    } else {
        models
            .first()
            .map(|v| v.model_id.to_string())
            .unwrap_or_default()
    }
}

pub fn find_model_variant(model_id: &str) -> Option<(&'static ProviderPreset, ModelVariant)> {
    for provider in PROVIDER_PRESETS {
        for variant in preset_models(provider) {
            if variant.model_id == model_id {
                return Some((provider, variant));
            }
        }
    }
    None
}

pub fn is_known_model(model_id: &str) -> bool {
    find_model_variant(model_id).is_some()
}

pub fn selection_for_model(model_id: &str) -> (usize, usize) {
    for (provider_index, provider) in PROVIDER_PRESETS.iter().enumerate() {
        for (model_index, variant) in preset_models(provider).into_iter().enumerate() {
            if variant.model_id == model_id {
                return (provider_index, model_index);
            }
        }
    }
    (0, 0)
}

pub fn selection_for_model_with_base_url(model_id: &str, base_url: &str) -> (usize, usize) {
    let (provider_index, model_index) = selection_for_model(model_id);
    if is_known_model(model_id) {
        return (provider_index, model_index);
    }
    if base_url.contains("11434") {
        if let Some(ollama_index) = PROVIDER_PRESETS
            .iter()
            .position(|p| p.endpoint_id.is_none())
        {
            return (ollama_index, 0);
        }
    }
    if base_url.contains("deepseek") {
        if let Some(i) = PROVIDER_PRESETS
            .iter()
            .position(|p| p.endpoint_id == Some("deepseek:cn"))
        {
            return (i, 0);
        }
    }
    (provider_index, model_index)
}

pub fn resolve_model_args(raw_model: &str, parts: &[&str]) -> ResolvedModel {
    let mut model = raw_model.to_string();
    let mut provider = parts.get(1).map(|s| s.to_string());
    let mut base_url = parts.get(2).map(|s| s.to_string());
    let mut api_key = parts.get(3).map(|s| s.to_string());

    if let Some((preset, _)) = find_model_variant(raw_model) {
        provider = Some(preset.provider.to_string());
        base_url = Some(preset_base_url(preset).to_string());
    } else {
        match raw_model {
            "kimi" | "moonshot-v1-32k" | "moonshot-v1-8k" | "moonshot-v1-128k" => {
                if raw_model == "kimi" {
                    model = "moonshot-v1-32k".to_string();
                }
                if let Some(preset) = PROVIDER_PRESETS
                    .iter()
                    .find(|p| p.endpoint_id == Some("moonshot:cn"))
                {
                    provider = Some(preset.provider.to_string());
                    base_url = Some(preset_base_url(preset).to_string());
                }
            }
            "glm" | "glm-4" | "glm-4-flash" | "glm-4-plus" => {
                if raw_model == "glm" {
                    model = "glm-4-flash".to_string();
                }
                if let Some(preset) = PROVIDER_PRESETS
                    .iter()
                    .find(|p| p.endpoint_id == Some("zhipu:cn"))
                {
                    provider = Some(preset.provider.to_string());
                    base_url = Some(preset_base_url(preset).to_string());
                }
            }
            _ => {
                if let Some(ollama_model) = raw_model.strip_prefix("ollama/") {
                    model = ollama_model.to_string();
                    if let Some(preset) = PROVIDER_PRESETS.iter().find(|p| p.endpoint_id.is_none())
                    {
                        provider = Some(preset.provider.to_string());
                        base_url = Some(preset_base_url(preset).to_string());
                    }
                    if api_key.is_none() {
                        api_key = Some("ollama".to_string());
                    }
                }
            }
        }
    }

    ResolvedModel {
        model,
        provider,
        base_url,
        api_key,
    }
}

pub fn env_vars_for_base_url(base_url: &str) -> &'static [&'static str] {
    let base = base_url.to_lowercase();
    if base.contains("deepseek") {
        &["DEEPSEEK_API_KEY", "ZENE_API_KEY", "OPENAI_API_KEY"]
    } else if base.contains("moonshot") {
        &["MOONSHOT_API_KEY", "ZENE_API_KEY"]
    } else if base.contains("bigmodel") || base.contains("z.ai") {
        &["ZHIPUAI_API_KEY", "ZHIPU_API_KEY", "ZENE_API_KEY"]
    } else if base.contains("anthropic") {
        &["ANTHROPIC_API_KEY", "ZENE_API_KEY"]
    } else {
        &["ZENE_API_KEY", "OPENAI_API_KEY"]
    }
}

pub fn has_api_key_for_provider(config: &ZeneConfig, provider: &ProviderPreset) -> bool {
    if !provider.requires_key {
        return true;
    }
    if config
        .api_key
        .as_deref()
        .is_some_and(|k| !k.is_empty())
    {
        return true;
    }
    env_vars_for_base_url(preset_base_url(provider))
        .iter()
        .any(|var| std::env::var(var).is_ok_and(|k| !k.is_empty()))
}

pub fn has_api_key_for_openai_provider(config: &ZeneConfig, base_url: &str) -> bool {
    if config
        .api_key
        .as_deref()
        .is_some_and(|k| !k.is_empty())
    {
        return true;
    }
    env_vars_for_base_url(base_url)
        .iter()
        .any(|var| std::env::var(var).is_ok_and(|k| !k.is_empty()))
}

pub fn model_requires_key(model_id: &str) -> bool {
    find_model_variant(model_id)
        .map(|(provider, _)| provider.requires_key)
        .unwrap_or(true)
}

pub fn lookup_registry_model(endpoint_id: &str, model_id: &str) -> Option<Model> {
    get_model_for_endpoint(endpoint_id, model_id)
}

pub fn models_help_message(agent: &Agent) -> String {
    let config = agent.config();
    let mut msg = String::new();
    msg.push_str("Commands:\n");
    msg.push_str("  /model                Provider → model picker (↑/↓, Enter, API key if needed)\n");
    msg.push_str("  /model <model_id>     Quick switch by model id\n");
    msg.push_str("  /provider             Configure or change provider\n\n");
    msg.push_str("Current configuration:\n");
    msg.push_str(&format!("  Model:    {}\n", config.model));
    msg.push_str(&format!("  Provider: {}\n", config.provider));
    msg.push_str(&format!("  Base URL: {}\n", config.base_url));
    let key_set = has_api_key_for_openai_provider(config, &config.base_url);
    msg.push_str(&format!(
        "  API Key:  {}\n",
        if key_set { "Configured" } else { "Not set" }
    ));

    if let Some(provider) = current_provider(config) {
        msg.push_str(&format!(
            "\nAvailable models ({}) — run /model for interactive picker:\n",
            provider.display_name
        ));
        for variant in preset_models(provider) {
            let marker = if variant.model_id == config.model {
                "*"
            } else {
                " "
            };
            if let Some(endpoint_id) = provider.endpoint_id {
                if let Some(meta) = lookup_registry_model(endpoint_id, variant.model_id) {
                    msg.push_str(&format!(
                        "  {marker} /model {:<22} {} (ctx {:?}, ${}/{}/1M in/out)\n",
                        variant.model_id,
                        variant.display_name,
                        meta.context_length,
                        meta.input_price,
                        meta.output_price
                    ));
                    continue;
                }
            }
            msg.push_str(&format!(
                "  {marker} /model {:<22} {}\n",
                variant.model_id, variant.display_name
            ));
        }
        if !is_provider_ready(config) && provider.requires_key {
            msg.push_str("\n  API key required. Run /provider to configure.\n");
        }
    } else {
        msg.push_str("\nProvider not recognized. Run /provider to configure.\n");
    }

    msg.push_str("\nAliases: /model kimi, /model glm, /model ollama/<name>");
    msg.push_str(&format!(
        "\nRegistry: llm_providers v{} ({})",
        llm_providers::registry_version(),
        llm_providers::registry_updated_at()
    ));
    msg
}

pub fn providers_help_message() -> String {
    let mut msg = String::from("Available providers — use /provider to configure:\n\n");
    for provider in PROVIDER_PRESETS {
        msg.push_str(&format!(
            "  {}  {}\n",
            provider.display_name,
            preset_base_url(provider)
        ));
        for variant in preset_models(provider) {
            msg.push_str(&format!("      /model {}\n", variant.model_id));
        }
        msg.push('\n');
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_finds_deepseek_v4_flash() {
        let (pi, mi) = selection_for_model("deepseek-v4-flash");
        assert_eq!(PROVIDER_PRESETS[pi].display_name, "DeepSeek");
        assert_eq!(
            preset_models(&PROVIDER_PRESETS[pi])[mi].model_id,
            "deepseek-v4-flash"
        );
    }

    #[test]
    fn resolve_v4_models_sets_deepseek_base_url() {
        let resolved = resolve_model_args("deepseek-v4-pro", &[]);
        assert_eq!(resolved.model, "deepseek-v4-pro");
        assert_eq!(resolved.provider.as_deref(), Some("openai"));
        assert_eq!(
            resolved.base_url.as_deref(),
            Some("https://api.deepseek.com/v1")
        );
    }

    #[test]
    fn registry_has_moonshot_cn_models() {
        let models = preset_models(
            PROVIDER_PRESETS
                .iter()
                .find(|p| p.endpoint_id == Some("moonshot:cn"))
                .expect("moonshot preset"),
        );
        assert!(models.iter().any(|m| m.model_id == "moonshot-v1-32k"));
    }
}
