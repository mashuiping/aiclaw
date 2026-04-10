//! AIClaw - AI Ops Agent
//!
//! A Rust-based AI operations agent that connects to messaging platforms,
//! loads skills, queries observability data, and troubleshoots Kubernetes clusters.

#![allow(ambiguous_glob_reexports)]

pub mod agent;
pub mod aiops;
pub mod channels;
pub mod config;
pub mod feedback;
pub mod kubernetes;
pub mod llm;
pub mod mcp;
pub mod observability;

pub mod security;
pub mod skills;
pub mod utils;

pub use agent::*;
pub use aiops::*;
pub use channels::*;
pub use config::*;
pub use feedback::*;
pub use kubernetes::*;
pub use llm::*;
pub use mcp::*;
pub use observability::*;
pub use security::*;
pub use skills::*;
pub use utils::*;
