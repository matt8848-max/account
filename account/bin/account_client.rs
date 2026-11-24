//! 桌面客户端启动器（GPUI）
//!
//! 只负责 GPUI 的启动配置；所有界面与业务逻辑在 `account::ui` 中。
//! 关闭窗口时由 `AccountApp` 的 `Drop` 实现联动关闭后端（若仍处于连接状态）。

use gpui_kit::AppContext as _;

fn main() {
    // 统一 rustls provider（HTTP/3 客户端与 GPUI / reqwest 共用）
    let _ = rustls::crypto::ring::default_provider().install_default();
    // 客户端日志：RUST_LOG 可覆盖。
    //
    // 单独把 `gpui::window::a11y` 的 info 日志压到 warn：gpui-kit 0.10 的
    // `Input` 外框为了把文本语义留给内部编辑器，固定使用
    // `RoleOverride::Presentational`，但外框仍然 `track_focus`；于是每次输入框
    // 获得焦点，gpui 都会打印 “focused element ... has an id but no role” 的
    // info 日志（上游误报，应用层无法修改该内部 role）。
    // 这里只过滤该模块的 info，warn/error 级 a11y 问题仍会显示。
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .filter_module("gpui::window::a11y", log::LevelFilter::Warn)
        .init();

    gpui_kit::application()
        .with_assets(account::ui::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            let mut options = gpui_kit::WindowOptions {
                icon: account::ui::assets::window_icon(),
                ..Default::default()
            };
            // 初始窗口取主显示器可用区域的 80%，并居中显示：
            // 旧实现使用 GPUI 默认尺寸，在 1080p/4K 屏上会接近满宽。
            if let Some(display) = cx.primary_display() {
                let bounds = display.visible_bounds();
                let width = bounds.size.width * 0.8;
                let height = bounds.size.height * 0.8;
                let origin = gpui_kit::point(
                    bounds.origin.x + (bounds.size.width - width) / 2.0,
                    bounds.origin.y + (bounds.size.height - height) / 2.0,
                );
                options.window_bounds = Some(gpui_kit::WindowBounds::Windowed(
                    gpui_kit::Bounds::new(origin, gpui_kit::size(width, height)),
                ));
            }
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| account::ui::AccountApp::new(window, cx))
            })
            .expect("打开主窗口失败");
        });
}
