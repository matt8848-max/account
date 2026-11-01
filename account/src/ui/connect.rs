//! 连接页
//!
//! 输入后端地址 → `live` 探测连通 → 进入主界面。
//! 已连接时显示红色「关闭服务器」按钮（调用 `POST /api/close`），
//! 点击后需在弹窗中确认。
//! 从主界面切回连接页查看时保持 `connected` 不变，关闭按钮仍可用。

use super::{ConfirmDialog, GlobalState, http_post, http_put, ping, push_toast};
use dioxus::prelude::*;

#[component]
pub(crate) fn ConnectPage() -> Element {
    let mut st = use_context::<GlobalState>();
    let base_text = (st.base)();
    let connected = (st.connected)();
    let return_visible = connected;
    // 页面内提示：连接状态文字 + Toast（替代浏览器原生 alert）
    let status_text = (st.status_msg)();
    let status_color = if connected { "#2e7d32" } else { "#c0392b" };
    let toast_msg = (st.toast)();
    // 确认关闭服务器弹窗开关
    let mut pending_close = use_signal(|| false);
    // 关闭请求进行中，避免重复提交
    let closing = use_signal(|| false);

    rsx! {
        div {
            style: "height:100dvh;display:flex;align-items:center;justify-content:center;flex-direction:column;gap:12px",
            h1 { "记账软件" }
            p { "输入后端地址（HTTP/2.0 HTTPS）" }
            input {
                value: base_text,
                placeholder: super::DEFAULT_BASE,
                oninput: move |e| st.base.set(e.value()),
            }
            button {
                onclick: move |_| {
                    let mut st = st;
                    st.status_msg.set("正在连接...".into());
                    spawn(async move {
                        let ok = ping(&st).await;
                        if ok {
                            let b = (st.base)();
                            match http_put(&b, "/api/counterparties", None).await {
                                Ok(v) => st.counterparties.set(serde_json::from_value(v).unwrap_or_default()),
                                Err(e) => { st.status_msg.set(format!("拉取交易对象失败: {e}")); return; }
                            }
                            match http_put(&b, "/api/currencies", None).await {
                                Ok(v) => st.currencies.set(serde_json::from_value(v).unwrap_or_default()),
                                Err(e) => { st.status_msg.set(format!("拉取货币失败: {e}")); return; }
                            }
                            // 拉取 API 与数据库版本号
                            match http_put(&b, "/api/version", None).await {
                                Ok(v) => {
                                    let api = v["api"].as_i64().unwrap_or(0);
                                    let dbv = v["db"].as_i64().unwrap_or(0);
                                    st.api_version.set(format!(
                                        "v{} API={api} DB={dbv}",
                                        env!("CARGO_PKG_VERSION")
                                    ));
                                }
                                Err(e) => { st.status_msg.set(format!("拉取版本失败: {e}")); return; }
                            }
                            st.connected.set(true);
                            // 连接成功进入主界面；已连接时切回连接页不会清除此标记
                            st.show_connect.set(false);
                            st.status_msg.set("连接成功".into());
                        } else {
                            st.status_msg.set("连接失败：检查地址与证书".into());
                            push_toast(&st, "连接失败：后端不可达，或需先信任自签名证书");
                        }
                    });
                },
                "🔗 连接"
            }
            p { color: "#666", "{base_text}" }
            p { style: "font-size:12px;color:#999", "后端地址默认 {super::DEFAULT_BASE}" }

            // 返回主界面按钮：已连接时可见，仅切回主界面视图（不断开）
            if return_visible {
                button {
                    style: "background:#2ecc71;color:#fff;border:none;padding:6px 16px;border-radius:4px;cursor:pointer",
                    onclick: move |_| st.show_connect.set(false),
                    "↩ 返回主界面"
                }
            }

            // 关闭服务器按钮：未连接时灰色禁用；连接后红色可点（需确认）
            button {
                disabled: !connected,
                style: if connected {
                    "background:#c0392b;color:#fff;border:none;padding:6px 16px;border-radius:4px;cursor:pointer"
                } else {
                    "background:#ccc;color:#888;border:none;padding:6px 16px;border-radius:4px;cursor:not-allowed"
                },
                onclick: move |_| {
                    if connected {
                        pending_close.set(true);
                    }
                },
                "⛔ 关闭服务器"
            }

            // 连接页没有底部状态栏，提示直接显示在页面内
            // （原 Web 端用浏览器原生 alert，会弹出「URL says …」，已移除）
            if !status_text.is_empty() {
                p { style: "color:{status_color};font-size:13px;margin:0", "{status_text}" }
            }
            if let Some(msg) = &toast_msg {
                p { style: "color:#c0392b;font-size:13px;margin:0", "{msg}" }
            }

            // 关闭确认弹窗
            ConfirmDialog {
                open: pending_close,
                title: String::from("确认关闭服务器？"),
                message: String::from(
                    "关闭后后端将停止运行，客户端将无法继续访问。\n请确认是否关闭？",
                ),
                on_confirm: move |_| {
                    let mut st = st;
                    let mut closing = closing;
                    spawn(async move {
                        if (closing)() {
                            return;
                        }
                        closing.set(true);
                        let b = (st.base)();
                        match http_post(&b, "/api/close", Some(serde_json::json!({}))).await {
                            Ok(v) => {
                                if v["closed"].as_bool().unwrap_or(false) {
                                    push_toast(&st, "服务器已关闭");
                                    st.connected.set(false);
                                    st.status_msg.set("服务器已关闭".into());
                                }
                            }
                            Err(e) => push_toast(&st, &format!("关闭服务器失败: {e}")),
                        }
                        closing.set(false);
                    });
                }
            }
        }
    }
}
