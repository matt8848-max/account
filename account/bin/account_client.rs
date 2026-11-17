//! 桌面客户端启动器（GPUI）
//!
//! 只负责 GPUI 的启动配置；所有界面与业务逻辑在 `account::ui` 中。
//! 关闭窗口时由 `AccountApp` 的 `Drop` 实现联动关闭后端（若仍处于连接状态）。

use gpui_kit::AppContext as _;

fn main() {
    // 统一 rustls provider（HTTP/3 客户端与 GPUI / reqwest 共用）
    let _ = rustls::crypto::ring::default_provider().install_default();
    // 客户端日志：RUST_LOG 可覆盖
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    gpui_kit::application()
        .with_assets(account::ui::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            let options = gpui_kit::WindowOptions {
                icon: account::ui::assets::window_icon(),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| account::ui::AccountApp::new(window, cx))
            })
            .expect("打开主窗口失败");
        });
}
