//! 客户端 UI 父模块（GPUI）
//!
//! 本文件**只负责注册子模块并决定导出**，业务代码全部位于 `ui/` 目录：
//! - [`app`]          — 根视图 `AccountApp`（状态 + 生命周期 + 页面路由）
//! - [`main_layout`]  — 200px 侧栏 + 内容区（双向滚动） + 26px 底部状态栏
//! - [`connect`]      — 连接页（地址 / HTTP 版本 / 连接·断开·关闭服务器）
//! - [`records`]      — 交易记录页（筛选、新建组、删除、排序）
//! - [`master`]       — 交易对象/货币页（含重命名）
//! - [`stats_page`]   — 数据统计页（双栏选择、卡片、桑基图、导出入口）
//! - [`sankey`]       — 桑基图/流向图绘制（纯 div 实现，无第三方绘图库）
//! - [`export`]       — xlsx 导出任务（后台生成 + 另存为）
//! - [`status_bar`]   — 底部状态栏（后端状态 / 更新 / 提示 / 进度）
//! - [`update`]       — GitHub 版本检测
//! - [`app_state`]    — 与渲染无关的状态类型与纯函数（含测试）
//! - [`http`]         — HTTP/2 与 HTTP/3 请求、连接会话
//! - [`confirm`]      — 通用确认弹窗（含导出选项）
//!
//! 每个子文件内自带测试；跨文件联动测试放在本文件底部。

pub mod app;
pub mod app_state;
pub mod assets;
pub mod confirm;
pub mod connect;
pub mod date;
pub mod export;
pub mod http;
pub mod main_layout;
pub mod master;
pub mod records;
pub mod sankey;
pub mod stats_page;
pub mod status_bar;
pub mod update;

pub use app::AccountApp;
/// 供桌面端 bin 在关窗时读取当前后端地址与协议。
pub use http::session;

#[cfg(test)]
mod tests {
    use super::app_state::{Page, SortOrder, TaskProgress};

    /// 跨文件联动：页面枚举/排序/进度类型可被 ui 父模块统一导出使用。
    #[test]
    fn ui_modules_should_share_state_types() {
        assert_ne!(Page::Records, Page::Master);
        assert_eq!(SortOrder::NewestFirst.toggled(), SortOrder::OldestFirst);
        assert_eq!(TaskProgress::new("导出", 2).percent(), 0);
    }
}
