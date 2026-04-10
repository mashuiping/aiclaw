//! LLM Providers module

pub mod anthropic;
pub mod deepseek;
pub mod minimax;
pub mod openai;
pub mod qwen;
pub mod zhipu;

pub use anthropic::AnthropicProvider;
pub use deepseek::DeepSeekProvider;
pub use minimax::MiniMaxProvider;
pub use openai::OpenAIProvider;
pub use qwen::QwenProvider;
pub use zhipu::ZhipuProvider;


/// Parse environment variable from config string
/// Supports ${VAR_NAME} syntax
pub fn parse_env_var(s: &str) -> String {
    if s.starts_with("${") && s.ends_with('}') {
        let var_name = &s[2..s.len() - 1];
        std::env::var(var_name).unwrap_or_else(|_| s.to_string())
    } else {
        s.to_string()
    }
}
