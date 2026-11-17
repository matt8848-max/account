//! 服务器端（后端）父模块
//!
//! 与 `ui.rs` 采用相同的组织方式：**本文件只负责注册子模块并决定导出**，
//! 具体的业务代码分别位于同名的 `server/` 目录下，便于逐个文件审阅。
//!
//! 子模块职责：
//! - [`money`]   — 定点数金额（前后端共用；`Display`/`FromStr`/比较 trait/四则运算）
//! - [`models`]  — 数据模型（前后端共用，纯 serde）
//! - [`export`]  — xlsx 导出（前后端共用，纯逻辑，不触碰文件系统）
//! - [`api`]     — HTTP API（仅 native；PUT=读，POST=其余）
//! - [`db`]      — SQLite 数据访问层（仅 native；按月分表 + 对象月度明细）
//!
//! “前后端共用的代码默认放在后端”是项目的约定：客户端（`ui`）直接
//! `use crate::server::{Money, models::*, export::*}`，避免同一份逻辑维护两遍。

pub mod export;
pub mod models;
pub mod money;

#[cfg(not(target_arch = "wasm32"))]
pub mod api;
#[cfg(not(target_arch = "wasm32"))]
pub mod db;

// 常用类型在父模块直接导出，子模块与客户端都从 `crate::server` 导入。
pub use models::*;
pub use money::{Money, REDUCE_DIVISOR_THRESHOLD};
