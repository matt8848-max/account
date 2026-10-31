//! 记账软件共享库
//!
//! - `models` — 数据模型（server 与 client 共用，纯 serde）
//! - `export` — xlsx 导出（纯逻辑，server/client 均可复用，不触碰文件系统）
//! - `ui` — 客户端界面（Dioxus，native + wasm）
//! - `db` — SQLite 数据访问层（仅 native 编译）
//! - `api` — HTTP API 实现（仅 native 编译）

pub mod export;
pub mod models;
pub mod ui;

#[cfg(not(target_arch = "wasm32"))]
pub mod api;
#[cfg(not(target_arch = "wasm32"))]
pub mod db;
