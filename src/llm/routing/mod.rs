//! LLM Routing module
//!
//! Supports: Direct API, OpenRouter, Ollama

pub mod direct;
pub mod ollama;
pub mod openrouter;

pub use direct::DirectRouter;
pub use ollama::OllamaRouter;
pub use openrouter::OpenRouterWrapper;
