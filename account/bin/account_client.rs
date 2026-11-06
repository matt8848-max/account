//! 客户端启动器（桌面 / Web）
//!
//! 只负责 Dioxus 的启动配置，纯 UI 组件在 `account::ui` 中。
//! - native（桌面）：`Config` + `WindowBuilder`（窗口标题、图标、关闭联动）
//! - wasm（Web）：使用默认启动方式
//!
//! **关闭联动**：桌面端关闭窗口（含系统标题栏的关闭按钮）时，若仍与后端保持连接，
//! 会先调用 `POST /api/close` 关闭后端服务器，再退出客户端，
//! 避免「客户端已关闭但后端进程仍在后台运行」。

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use dioxus::desktop::{
            Config, WindowBuilder, WindowEvent, icon_from_memory, tao::event::Event,
        };
        use std::path::PathBuf;

        let data_dir_local = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
        let data_dir = PathBuf::from(data_dir_local).join("account-app");

        // 窗口图标：构建时嵌入（由 tools/make_icons.py 从个人照片生成的资产）。
        // 解码失败时退回 Dioxus 默认图标，不影响启动。
        let window_icon = icon_from_memory(include_bytes!("../assets/icon-256.png")).ok();

        let config = Config::new()
            .with_data_directory(data_dir)
            .with_menu(None)
            .with_window(
                WindowBuilder::new()
                    .with_always_on_top(false)
                    .with_window_icon(window_icon)
                    .with_title("记账软件"),
            )
            // 关闭窗口事件在 Dioxus 关闭窗口之前回调，此时调用后端关闭接口仍能完成往返
            .with_custom_event_handler(|event, _target| {
                if matches!(
                    event,
                    Event::WindowEvent {
                        event: WindowEvent::CloseRequested,
                        ..
                    }
                ) {
                    shutdown_backend();
                }
            });

        dioxus::LaunchBuilder::new()
            .with_cfg(config)
            .launch(account::ui::App);
    }

    #[cfg(target_arch = "wasm32")]
    {
        // Web 端（wasm）：关闭浏览器标签页时无法可靠地发出请求，故不做关闭联动
        dioxus::LaunchBuilder::new().launch(account::ui::App);
    }
}

/// 退出前关闭后端服务器（仅桌面端）
///
/// 事件回调是同步 `FnMut`，这里临时建一个单线程 tokio 运行时阻塞等待
/// `POST /api/close`，并加 1.5s 超时：最坏情况只是退出稍慢，
/// 不会因为后端已失联卡住进程。任何失败都只记 `warn` 日志，不阻塞退出。
#[cfg(not(target_arch = "wasm32"))]
fn shutdown_backend() {
    use account::ui::session;
    use std::time::Duration;

    /// 关闭请求超时：超过则放弃等待，直接退出
    const CLOSE_TIMEOUT: Duration = Duration::from_millis(1500);

    // 未连接（或已手动断开 / 已关闭服务器）：无需请求后端
    let Some(base) = session::active_base() else {
        return;
    };
    log::info!("退出前关闭后端服务器: {base}");

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            log::warn!("创建关闭后端所需的运行时失败: {e}");
            return;
        }
    };

    let result = runtime.block_on(async move {
        let client = match reqwest::Client::builder()
            // 与 UI 侧一致：后端使用自签名证书
            .danger_accept_invalid_certs(true)
            .build()
        {
            Ok(c) => c,
            Err(e) => return Err(e.to_string()),
        };
        let request = client
            .post(format!("{base}/api/close"))
            .json(&serde_json::json!({}))
            .send();
        match tokio::time::timeout(CLOSE_TIMEOUT, request).await {
            Ok(Ok(resp)) => Ok(resp.status().as_u16()),
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Err("请求超时".to_string()),
        }
    });

    match result {
        Ok(status) => log::info!("后端关闭请求已发出（HTTP {status}）"),
        Err(e) => log::warn!("关闭后端失败: {e}"),
    }
    // 清掉会话缓存：即使请求失败也不再重复尝试（进程即将退出）
    session::clear();
}
