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

/// 在**独立线程**里执行 `task`，并在当前线程用 std 通道阻塞等待结果（带超时）
///
/// 为什么不能在当前线程直接 `block_on`：桌面端窗口关闭回调运行在 Dioxus 事件循环线程上，
/// 而该线程本身已在 tokio 运行时上下文内，再 `block_on` 一个新运行时会 panic
/// （`Cannot start a runtime from within a runtime`）。
/// 因此运行时只在别的线程创建，当前线程只做纯 std 的阻塞等待。
///
/// 返回 `None` 表示超时（调用方应放弃等待、照常退出）。
#[cfg(not(target_arch = "wasm32"))]
fn run_on_new_thread<T, F>(task: F, timeout: std::time::Duration) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // 接收端可能已超时退出，发送失败无需处理
        let _ = tx.send(task());
    });
    rx.recv_timeout(timeout).ok()
}

/// 退出前关闭后端服务器（仅桌面端）
///
/// `POST /api/close` 在独立线程的 tokio 运行时里发出（见 [`run_on_new_thread`]），
/// 当前线程最多等待 1.5s：最坏情况只是退出稍慢，不会因为后端已失联卡住进程。
/// 任何失败都只记 `warn` 日志，不影响退出。
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

    let result = run_on_new_thread(
        move || {
            let request = async move {
                let client = reqwest::Client::builder()
                    // 与 UI 侧一致：后端使用自签名证书
                    .danger_accept_invalid_certs(true)
                    // 客户端级超时：避免线程内请求长时间挂着
                    .timeout(CLOSE_TIMEOUT)
                    .build()
                    .map_err(|e| e.to_string())?;
                let resp = client
                    .post(format!("{base}/api/close"))
                    .json(&serde_json::json!({}))
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                Ok::<u16, String>(resp.status().as_u16())
            };

            match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt.block_on(request),
                Err(e) => Err(format!("创建运行时失败: {e}")),
            }
        },
        CLOSE_TIMEOUT,
    );

    match result {
        Some(Ok(status)) => log::info!("后端关闭请求已发出（HTTP {status}）"),
        Some(Err(e)) => log::warn!("关闭后端失败: {e}"),
        None => log::warn!("关闭后端超时，直接退出"),
    }
    // 清掉会话缓存：即使请求失败也不再重复尝试（进程即将退出）
    session::clear();
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::run_on_new_thread;
    use std::time::Duration;

    /// 验证「在独立线程执行任务」在**已处于 tokio 运行时内**时不 panic，
    /// 并能拿到任务返回值；超时则返回 None（桌面端关窗时据此放弃等待）
    ///
    /// 回归背景：早期实现在窗口关闭回调里直接 `block_on` 新运行时，
    /// 会 panic `Cannot start a runtime from within a runtime`。
    #[tokio::test]
    async fn run_on_new_thread_should_work_inside_runtime() {
        // 本测试体运行在 tokio 运行时内，等价于事件循环线程的上下文
        let value = run_on_new_thread(|| 42u32, Duration::from_secs(2));
        assert_eq!(value, Some(42));

        // 任务慢于超时：返回 None，不阻塞调用方
        let slow = run_on_new_thread(
            || {
                std::thread::sleep(Duration::from_millis(300));
                7u32
            },
            Duration::from_millis(50),
        );
        assert_eq!(slow, None);
        // 不相等断言：超时结果不能是任务的返回值
        assert_ne!(slow, Some(7));
    }
}
