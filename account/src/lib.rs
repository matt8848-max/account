//! 记账软件共享库
//!
//! 模块分层（与 `ui.rs` / `server.rs` 的“父模块只注册”约定一致）：
//! - [`server`] — 后端与前后端共用代码（Money、数据模型、xlsx 导出、API、DB）
//! - [`ui`]     — 桌面客户端（GPUI，仅 `client` 特性下编译）
//!
//! 共用代码放在 `server/` 下，客户端直接复用，避免前后端各写一份逻辑。

pub mod server;

/// 客户端 UI：依赖 gpui-kit / rfd，server 不启用该特性，
/// 从而编译 `account_server` 时不会触碰任何 GUI 依赖。
#[cfg(feature = "client")]
pub mod ui;
