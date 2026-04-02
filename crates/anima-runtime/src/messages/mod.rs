//! 消息协议三层映射
//!
//! 对标 claude-code-main 的 `messages.ts`（normalize + lookup + pairing）。

pub mod lookup;
pub mod normalize;
pub mod pairing;
pub mod types;

pub use lookup::*;
pub use normalize::*;
pub use pairing::*;
pub use types::*;
