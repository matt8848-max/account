//! 客户端启动器
//!
//! 负责 Dioxus 的应用启动配置，纯 UI 组件在 `account::ui` 中。
//! - native（桌面）：使用 Config + WindowBuilder 配置窗口
//! - wasm（Web）：使用默认启动方式

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // 桌面端（native）
        use dioxus::desktop::{Config, WindowBuilder};
        use std::path::PathBuf;

        let data_dir_local = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
        let data_dir = PathBuf::from(data_dir_local).join("account-app");

        let config = Config::new()
            .with_data_directory(data_dir)
            .with_menu(None)
            .with_window(
                WindowBuilder::new()
                    .with_always_on_top(false)
                    .with_title("记账软件"),
            );

        dioxus::LaunchBuilder::new()
            .with_cfg(config)
            .launch(account::ui::App);
    }

    #[cfg(target_arch = "wasm32")]
    {
        // Web 端（wasm）
        dioxus::LaunchBuilder::new().launch(account::ui::App);
    }
}