use tiktoken_rs::tokenizer::{Tokenizer, get_tokenizer};
use tiktoken_rs::{
    CoreBPE, cl100k_base_singleton, o200k_base_singleton, o200k_harmony_singleton,
};
use zene_llm::{Message, MessageKind, Role, ToolDefinition};

/// Provider family for token estimation heuristics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EstimateProvider {
    #[default]
    OpenAi,
    Anthropic,
}

impl EstimateProvider {
    pub fn from_name(name: &str) -> Self {
        match name.trim().to_lowercase().as_str() {
            "anthropic" => Self::Anthropic,
            _ => Self::OpenAi,
        }
    }
}

/// Per tool-call JSON/API framing beyond argument string length.
const TOOL_CALL_FRAME_TOKENS: u32 = 12;

/// OpenAI tiktoken BPE encoding used for accurate text counting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TiktokenEncoding {
    Cl100kBase,
    O200kBase,
    O200kHarmony,
}

impl TiktokenEncoding {
    /// Resolve encoding for a known OpenAI model name (via `tiktoken-rs` model map).
    pub fn from_model(model: &str) -> Option<Self> {
        match get_tokenizer(model)? {
            Tokenizer::Cl100kBase => Some(Self::Cl100kBase),
            Tokenizer::O200kBase => Some(Self::O200kBase),
            Tokenizer::O200kHarmony => Some(Self::O200kHarmony),
            _ => None,
        }
    }

    fn bpe(self) -> &'static CoreBPE {
        match self {
            Self::Cl100kBase => cl100k_base_singleton(),
            Self::O200kBase => o200k_base_singleton(),
            Self::O200kHarmony => o200k_harmony_singleton(),
        }
    }

    fn count(self, text: &str) -> u32 {
        if text.is_empty() {
            return 0;
        }
        self.bpe().encode_ordinary(text).len() as u32
    }
}

/// How text is converted to token estimates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EstimateMode {
    /// Latin ~`chars_per_token`; CJK ~1 token/char (closer to real BPE for mixed text).
    #[default]
    ScriptAware,
    /// Legacy uniform chars/token ceiling division.
    Uniform,
    /// OpenAI BPE via `tiktoken-rs` (encoding selected from model name).
    Tiktoken(TiktokenEncoding),
}

/// Token estimator: heuristic by default; OpenAI models use tiktoken BPE.
#[derive(Debug, Clone, Copy)]
pub struct TokenEstimator {
    pub chars_per_token: f32,
    pub mode: EstimateMode,
}

impl Default for TokenEstimator {
    fn default() -> Self {
        Self {
            chars_per_token: 4.0,
            mode: EstimateMode::ScriptAware,
        }
    }
}

impl TokenEstimator {
    pub fn new(chars_per_token: f32) -> Self {
        Self {
            chars_per_token: chars_per_token.max(1.0),
            mode: EstimateMode::ScriptAware,
        }
    }

    pub fn with_mode(mut self, mode: EstimateMode) -> Self {
        self.mode = mode;
        self
    }

    /// Build an estimator for the active provider/model.
    ///
    /// OpenAI path: use `tiktoken-rs` when the model maps to a known encoding.
    /// Unknown openai-compatible model names and Anthropic keep the script-aware heuristic.
    pub fn for_provider(provider: EstimateProvider, model: &str, chars_per_token: f32) -> Self {
        let chars_per_token = chars_per_token.max(1.0);
        match provider {
            EstimateProvider::OpenAi => {
                if let Some(encoding) = TiktokenEncoding::from_model(model) {
                    Self {
                        chars_per_token,
                        mode: EstimateMode::Tiktoken(encoding),
                    }
                } else {
                    Self::new(chars_per_token)
                }
            }
            EstimateProvider::Anthropic => Self::new(chars_per_token),
        }
    }

    pub fn estimate_chars_as_tokens(&self, text: &str) -> u32 {
        if text.is_empty() {
            return 0;
        }
        match self.mode {
            EstimateMode::Uniform => {
                (text.chars().count() as f32 / self.chars_per_token).ceil() as u32
            }
            EstimateMode::ScriptAware => estimate_script_aware(text, self.chars_per_token),
            EstimateMode::Tiktoken(encoding) => encoding.count(text),
        }
    }

    /// Role-specific framing overhead (JSON keys, role labels, separators).
    fn role_overhead(&self, role: Role, kind: Option<MessageKind>) -> u32 {
        let base = match self.mode {
            // OpenAI cookbook: ~3 tokens framing per chat message.
            EstimateMode::Tiktoken(_) => 3,
            EstimateMode::ScriptAware | EstimateMode::Uniform => match role {
                Role::System => 8,
                Role::User => 4,
                Role::Assistant => 4,
                Role::Tool => 8,
            },
        };
        if kind == Some(MessageKind::CompactionSummary) {
            base + 4
        } else {
            base
        }
    }

    /// JSON tool arguments: BPE count as-is; heuristic adds structural punctuation overhead.
    fn estimate_json_args_tokens(&self, json: &str) -> u32 {
        if json.is_empty() {
            return 0;
        }
        match self.mode {
            EstimateMode::Tiktoken(_) => self.estimate_chars_as_tokens(json),
            EstimateMode::ScriptAware | EstimateMode::Uniform => {
                let content = self.estimate_chars_as_tokens(json);
                let structure = json
                    .chars()
                    .filter(|c| matches!(c, '{' | '}' | '[' | ']' | ':' | ',' | '"'))
                    .count();
                content + (structure as f32 / self.chars_per_token).ceil() as u32
            }
        }
    }

    pub fn estimate_message_tokens(&self, message: &Message) -> u32 {
        let mut tokens = self.role_overhead(message.role, message.kind);

        if let Some(content) = &message.content {
            tokens += self.estimate_chars_as_tokens(content);
        }

        if let Some(tool_calls) = &message.tool_calls {
            for call in tool_calls {
                // Cookbook uses ~1 token per tool call; keep a slightly higher
                // constant for heuristic modes where argument framing is approximate.
                let frame = match self.mode {
                    EstimateMode::Tiktoken(_) => 1,
                    EstimateMode::ScriptAware | EstimateMode::Uniform => TOOL_CALL_FRAME_TOKENS,
                };
                tokens += frame;
                tokens += self.estimate_chars_as_tokens(&call.id);
                tokens += self.estimate_chars_as_tokens(&call.name);
                tokens += self.estimate_json_args_tokens(&call.arguments);
            }
        }

        if let Some(tool_call_id) = &message.tool_call_id {
            tokens += self.estimate_chars_as_tokens(tool_call_id);
        }

        if let Some(name) = &message.name {
            tokens += self.estimate_chars_as_tokens(name);
        }

        if message.is_error == Some(true) {
            tokens += 2;
        }

        tokens
    }

    pub fn estimate_messages_tokens(&self, messages: &[Message]) -> u32 {
        messages.iter().map(|m| self.estimate_message_tokens(m)).sum()
    }

    pub fn estimate_tools_tokens(&self, tools: &[ToolDefinition]) -> u32 {
        if tools.is_empty() {
            return 0;
        }
        let json = serde_json::to_string(tools).unwrap_or_default();
        self.estimate_chars_as_tokens(&json) + 4
    }

    pub fn estimate_request_tokens(&self, messages: &[Message], tools: &[ToolDefinition]) -> u32 {
        let mut tokens =
            self.estimate_messages_tokens(messages) + self.estimate_tools_tokens(tools);
        // OpenAI cookbook reply priming: <|start|>assistant<|message|>
        if matches!(self.mode, EstimateMode::Tiktoken(_)) {
            tokens += 3;
        }
        tokens
    }
}

fn is_cjk(ch: char) -> bool {
    matches!(ch,
        '\u{4E00}'..='\u{9FFF}' | // CJK Unified
        '\u{3400}'..='\u{4DBF}' | // Extension A
        '\u{F900}'..='\u{FAFF}' | // Compatibility
        '\u{3040}'..='\u{30FF}' | // Hiragana/Katakana
        '\u{AC00}'..='\u{D7AF}'   // Hangul
    )
}

/// Mixed-script estimate: CJK ≈ 1 tok/char; Latin runs use chars_per_token.
fn estimate_script_aware(text: &str, chars_per_token: f32) -> u32 {
    let cpt = chars_per_token.max(1.0);
    let mut tokens = 0.0f32;
    let mut latin_run = 0usize;
    for ch in text.chars() {
        if is_cjk(ch) {
            if latin_run > 0 {
                tokens += (latin_run as f32 / cpt).ceil();
                latin_run = 0;
            }
            tokens += 1.0;
        } else if ch.is_whitespace() {
            if latin_run > 0 {
                tokens += (latin_run as f32 / cpt).ceil();
                latin_run = 0;
            }
            tokens += 0.25;
        } else {
            latin_run += 1;
        }
    }
    if latin_run > 0 {
        tokens += (latin_run as f32 / cpt).ceil();
    }
    tokens.ceil().max(1.0) as u32
}

/// Total estimated context size (messages + tool definitions) for compaction checks.
pub fn estimate_context(
    messages: &[Message],
    tools: &[ToolDefinition],
    estimator: &TokenEstimator,
) -> usize {
    estimator.estimate_request_tokens(messages, tools) as usize
}

/// Heuristic token estimate: ~4 characters per token (legacy default).
#[allow(dead_code)]
pub fn estimate_chars_as_tokens(text: &str) -> u32 {
    TokenEstimator::default().estimate_chars_as_tokens(text)
}

#[allow(dead_code)]
pub fn estimate_message_tokens(message: &Message) -> u32 {
    TokenEstimator::default().estimate_message_tokens(message)
}

#[allow(dead_code)]
pub fn estimate_messages_tokens(messages: &[Message]) -> u32 {
    TokenEstimator::default().estimate_messages_tokens(messages)
}

#[allow(dead_code)]
pub fn estimate_tools_tokens(tools: &[ToolDefinition]) -> u32 {
    TokenEstimator::default().estimate_tools_tokens(tools)
}

#[allow(dead_code)]
pub fn estimate_request_tokens(messages: &[Message], tools: &[ToolDefinition]) -> u32 {
    TokenEstimator::default().estimate_request_tokens(messages, tools)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zene_llm::ToolCall;

    #[test]
    fn uniform_chars_heuristic_uses_ceiling_division() {
        let est = TokenEstimator::default().with_mode(EstimateMode::Uniform);
        assert_eq!(est.estimate_chars_as_tokens(""), 0);
        assert_eq!(est.estimate_chars_as_tokens("abcd"), 1);
        assert_eq!(est.estimate_chars_as_tokens("abcde"), 2);
    }

    #[test]
    fn script_aware_counts_cjk_higher() {
        let est = TokenEstimator::default();
        let latin = est.estimate_chars_as_tokens("abcd"); // ~1
        let cjk = est.estimate_chars_as_tokens("中文测试"); // ~4
        assert!(cjk > latin);
        assert_eq!(cjk, 4);
    }

    #[test]
    fn custom_ratio_increases_token_count() {
        let loose = TokenEstimator::new(8.0).with_mode(EstimateMode::Uniform);
        let tight = TokenEstimator::new(2.0).with_mode(EstimateMode::Uniform);
        let text = "abcdefgh";
        assert!(loose.estimate_chars_as_tokens(text) < tight.estimate_chars_as_tokens(text));
    }

    #[test]
    fn tool_message_includes_content() {
        let message = Message::tool_result("call_1", "Read", "file contents");
        assert!(estimate_message_tokens(&message) > 4);
    }

    #[test]
    fn assistant_with_tools_counts_arguments() {
        let message = Message::assistant_with_tools(
            None,
            vec![ToolCall {
                id: "call_1".into(),
                name: "Read".into(),
                arguments: r#"{"path":"a.rs"}"#.into(),
            }],
        );
        assert!(estimate_message_tokens(&message) > estimate_message_tokens(&Message::assistant("hi")));
    }

    #[test]
    fn tools_json_is_included() {
        let tools = vec![ToolDefinition {
            name: "Read".into(),
            description: "Read a file".into(),
            parameters: serde_json::json!({"type": "object"}),
        }];
        assert!(estimate_tools_tokens(&tools) > 0);
    }

    #[test]
    fn estimate_context_matches_request_tokens() {
        let messages = vec![Message::user("hello")];
        let tools = vec![ToolDefinition {
            name: "Read".into(),
            description: "Read".into(),
            parameters: serde_json::json!({}),
        }];
        let estimator = TokenEstimator::default();
        assert_eq!(
            estimate_context(&messages, &tools, &estimator),
            estimator.estimate_request_tokens(&messages, &tools) as usize
        );
    }

    #[test]
    fn system_role_has_higher_overhead_than_user() {
        let estimator = TokenEstimator::default();
        let system = Message::system("hi");
        let user = Message::user("hi");
        assert!(estimator.estimate_message_tokens(&system) > estimator.estimate_message_tokens(&user));
    }

    #[test]
    fn json_args_include_structure_overhead() {
        let estimator = TokenEstimator::default();
        let plain = Message::assistant("hi");
        let with_tools = Message::assistant_with_tools(
            None,
            vec![ToolCall {
                id: "call_1".into(),
                name: "Read".into(),
                arguments: r#"{"path":"src/main.rs","offset":1,"limit":100}"#.into(),
            }],
        );
        assert!(
            estimator.estimate_message_tokens(&with_tools)
                > estimator.estimate_message_tokens(&plain) + 10
        );
    }

    #[test]
    fn compaction_summary_has_extra_overhead() {
        let estimator = TokenEstimator::default();
        let plain = Message::assistant("summary text");
        let summary = Message::compaction_summary("summary text");
        assert!(estimator.estimate_message_tokens(&summary) > estimator.estimate_message_tokens(&plain));
    }

    #[test]
    fn openai_gpt4o_uses_o200k_tiktoken() {
        let est = TokenEstimator::for_provider(EstimateProvider::OpenAi, "gpt-4o", 4.0);
        assert_eq!(est.mode, EstimateMode::Tiktoken(TiktokenEncoding::O200kBase));
        // cl100k/o200k: "hello world" → 2 tokens
        assert_eq!(est.estimate_chars_as_tokens("hello world"), 2);
    }

    #[test]
    fn openai_gpt4_uses_cl100k_tiktoken() {
        let est = TokenEstimator::for_provider(EstimateProvider::OpenAi, "gpt-4", 4.0);
        assert_eq!(est.mode, EstimateMode::Tiktoken(TiktokenEncoding::Cl100kBase));
        assert_eq!(est.estimate_chars_as_tokens("hello world"), 2);
    }

    #[test]
    fn openai_compatible_unknown_model_falls_back_to_heuristic() {
        let est = TokenEstimator::for_provider(EstimateProvider::OpenAi, "deepseek-chat", 4.0);
        assert_eq!(est.mode, EstimateMode::ScriptAware);
    }

    #[test]
    fn anthropic_keeps_heuristic() {
        let est = TokenEstimator::for_provider(EstimateProvider::Anthropic, "claude-sonnet-4-5", 4.0);
        assert_eq!(est.mode, EstimateMode::ScriptAware);
    }

    #[test]
    fn tiktoken_counts_cjk_higher_than_latin_run() {
        let est = TokenEstimator::for_provider(EstimateProvider::OpenAi, "gpt-4o", 4.0);
        let latin = est.estimate_chars_as_tokens("abcd");
        let cjk = est.estimate_chars_as_tokens("中文测试");
        assert!(cjk > latin);
    }

    #[test]
    fn tiktoken_request_includes_reply_priming() {
        let est = TokenEstimator::for_provider(EstimateProvider::OpenAi, "gpt-4o", 4.0);
        let messages = vec![Message::user("hi")];
        let msg_only = est.estimate_messages_tokens(&messages);
        let request = est.estimate_request_tokens(&messages, &[]);
        assert_eq!(request, msg_only + 3);
    }
}
