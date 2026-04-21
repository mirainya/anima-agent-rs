//! System Prompt 管理模块
//!
//! 提供段落化的系统提示词组装能力，支持 identity、tools、environment 等内置段落。

pub mod assembly;
pub mod sections;
pub mod types;

pub use assembly::PromptAssembler;
pub use types::{EnvironmentInfo, PromptSection, SystemPrompt};
