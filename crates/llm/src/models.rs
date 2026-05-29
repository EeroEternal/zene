use zene_config::{default_context_window_for_model, ZeneConfig};

pub fn context_window_for_model(model: &str, config: &ZeneConfig) -> u32 {
    if let Some(tokens) = config.model_context_windows.get(model) {
        return *tokens;
    }
    default_context_window_for_model(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_model_uses_default_map() {
        let config = ZeneConfig::default();
        assert_eq!(context_window_for_model("gpt-4o", &config), 128_000);
    }

    #[test]
    fn config_override_wins() {
        let mut config = ZeneConfig::default();
        config
            .model_context_windows
            .insert("custom-model".to_string(), 32_000);
        assert_eq!(context_window_for_model("custom-model", &config), 32_000);
    }
}
