//! 连接页
//!
//! 单页五行（整体仍居中）：
//! 1. 标题
//! 2. `输入后端地址：` + 输入框（默认 `https://127.0.0.2:3000`）
//! 3. 灰色可复制文本：后端地址默认值
//! 4. 连接 / 断开 / 关闭服务器（不可用的一律灰底灰字并 `disabled`）
//! 5. 信息提示（连接状态 + Toast）
//!
//! **连接按钮兼任「进入/返回主页面」**：
//! - 未连接：探测连通并拉取初始数据后进入主界面
//! - 已连接且地址未变：直接返回主界面，不重新建连
//! - 已连接但地址被改过：先断开旧会话，再按新地址重连
//!
//! 「断开」只断开网络连接（清空缓存、保留地址、留在本页）；
//! 「关闭服务器」调用 `POST /api/close`，需在确认弹窗中确认。

use super::{
    ConfirmDialog, DEFAULT_BASE, GlobalState, http_post, http_put, ping, push_toast, session,
};
use dioxus::prelude::*;

/// 不可用按钮的统一样式（与 `disabled` 属性配合，双重防呆）
const BTN_DISABLED: &str =
    "background:#ccc;color:#888;border:none;padding:6px 16px;border-radius:4px;cursor:not-allowed";

/// 连接后端并拉取初始数据（交易对象 / 货币 / 版本号）
///
/// 任一步失败即返回错误信息，由调用方提示；成功时**不**修改 `connected`，
/// 保持「连接成功后才算已连接」的语义。
async fn connect_backend(st: &GlobalState) -> Result<(), String> {
    if !ping(st).await {
        return Err("连接失败：后端不可达，或需先信任自签名证书".to_string());
    }
    let mut st = *st; // GlobalState 是 Copy：解引用复制一份供写入信号
    let b = (st.base)();
    match http_put(&b, "/api/counterparties", None).await {
        Ok(v) => st
            .counterparties
            .set(serde_json::from_value(v).unwrap_or_default()),
        Err(e) => return Err(format!("拉取交易对象失败: {e}")),
    }
    match http_put(&b, "/api/currencies", None).await {
        Ok(v) => st
            .currencies
            .set(serde_json::from_value(v).unwrap_or_default()),
        Err(e) => return Err(format!("拉取货币失败: {e}")),
    }
    match http_put(&b, "/api/version", None).await {
        Ok(v) => {
            let api = v["api"].as_i64().unwrap_or(0);
            let dbv = v["db"].as_i64().unwrap_or(0);
            st.api_version
                .set(format!("v{} API={api} DB={dbv}", env!("CARGO_PKG_VERSION")));
        }
        Err(e) => return Err(format!("拉取版本失败: {e}")),
    }
    Ok(())
}

/// 断开连接：清空会话与所有缓存数据，**保留**输入框里的地址
///
/// 断开的语义是「不再与该后端通信」，因此交易对象/货币/记录/统计/版本都要清掉，
/// 避免主界面残留旧数据被误认为仍然在线。
fn disconnect(st: &GlobalState) {
    let mut st = *st;
    st.connected.set(false);
    st.counterparties.set(Vec::new());
    st.currencies.set(Vec::new());
    st.records.set(Vec::new());
    st.stats.set(Vec::new());
    st.api_version.set("未知".to_string());
    st.live_ok.set(false);
    st.live_at.set("未探测".to_string());
    session::clear();
}

/// 「连接」按钮的公共行为（第四行按钮调用；独立成函数便于后续复用）
///
/// 1. 未连接：探测连通 + 拉取初始数据 → 进入主界面
/// 2. 已连接且地址未变：直接返回主界面（连接按钮兼任「返回」）
/// 3. 已连接但地址被改过：先断开旧会话，再按新地址重连
fn on_connect_clicked(mut st: GlobalState, mut connecting: Signal<bool>) {
    // 进行中的请求未结束时忽略重复点击
    if (connecting)() {
        return;
    }
    spawn(async move {
        let base_now = (st.base)().trim().to_string();
        if base_now.is_empty() {
            st.status_msg.set("请输入后端地址".into());
            push_toast(&st, "请输入后端地址（如 https://127.0.0.2:3000）");
            return;
        }
        // 地址规范化后再使用：避免首尾空白导致地址比较失败
        st.base.set(base_now.clone());

        if (st.connected)() {
            if session::active_base().as_deref() == Some(base_now.as_str()) {
                st.show_connect.set(false);
                st.status_msg.set("已连接，已返回主页面".into());
                return;
            }
            // 地址被改过：先断开旧会话，再按新地址重连
            disconnect(&st);
        }

        connecting.set(true);
        st.status_msg.set("正在连接...".into());
        match connect_backend(&st).await {
            Ok(()) => {
                st.connected.set(true);
                st.show_connect.set(false);
                st.status_msg.set("连接成功".into());
            }
            Err(e) => {
                st.status_msg.set(e.clone());
                push_toast(&st, &e);
            }
        }
        connecting.set(false);
    });
}

#[component]
pub(crate) fn ConnectPage() -> Element {
    let mut st = use_context::<GlobalState>();
    let base_text = (st.base)();
    let connected = (st.connected)();
    // 页面内提示：连接状态文字 + Toast（替代浏览器原生 alert）
    let status_text = (st.status_msg)();
    let status_color = if connected { "#2e7d32" } else { "#c0392b" };
    let toast_msg = (st.toast)();
    // 连接请求进行中，避免重复提交
    let connecting = use_signal(|| false);
    // 确认关闭服务器弹窗开关
    let mut pending_close = use_signal(|| false);
    // 关闭请求进行中，避免重复提交
    let closing = use_signal(|| false);

    let connecting_now = (connecting)();
    let connect_style = if connecting_now {
        BTN_DISABLED
    } else {
        "background:#3498db;color:#fff;border:none;padding:6px 16px;border-radius:4px;cursor:pointer"
    };
    let disconnect_style = if connected {
        "background:#f39c12;color:#fff;border:none;padding:6px 16px;border-radius:4px;cursor:pointer"
    } else {
        BTN_DISABLED
    };
    let close_style = if connected {
        "background:#c0392b;color:#fff;border:none;padding:6px 16px;border-radius:4px;cursor:pointer"
    } else {
        BTN_DISABLED
    };

    rsx! {
        div {
            style: "height:100dvh;display:flex;align-items:center;justify-content:center;flex-direction:column;gap:12px",
            // 第一行：标题
            h1 { style: "margin:0", "记账软件" }

            // 第二行：地址输入（连接按钮在第四行）
            div { style: "display:flex;gap:8px;align-items:center",
                label { "输入后端地址：" }
                input {
                    value: base_text,
                    placeholder: DEFAULT_BASE,
                    style: "width:260px",
                    oninput: move |e| st.base.set(e.value()),
                }
            }

            // 第三行：默认地址提示（灰色、可复制）
            p { style: "font-size:12px;color:#999;margin:0;user-select:text",
                "后端地址默认 {DEFAULT_BASE}"
            }

            // 第四行：连接 / 断开 / 关闭服务器
            div { style: "display:flex;gap:8px",
                button {
                    style: connect_style,
                    disabled: connecting_now,
                    // 连接按钮兼任「返回主页面」：地址变更时按新地址重连
                    onclick: move |_| on_connect_clicked(st, connecting),
                    if connecting_now { "连接中..." } else { "🔗 连接" }
                }
                button {
                    style: disconnect_style,
                    disabled: !connected,
                    onclick: move |_| {
                        let mut st = st;
                        if (st.connected)() {
                            disconnect(&st);
                            st.status_msg.set("已断开连接".into());
                            push_toast(&st, "已断开连接");
                        }
                    },
                    "⏏ 断开"
                }
                button {
                    style: close_style,
                    disabled: !connected,
                    onclick: move |_| {
                        if connected {
                            pending_close.set(true);
                        }
                    },
                    "⛔ 关闭服务器"
                }
            }

            // 第五行：信息提示（连接页没有底部状态栏，提示直接显示在页面内）
            if !status_text.is_empty() {
                p { style: "color:{status_color};font-size:13px;margin:0", "{status_text}" }
            }
            // 已连接时说明「连接」按钮可返回主页面（避免用户误以为必须断开才能回去）
            if connected {
                p { style: "font-size:12px;color:#666;margin:0", "点击「连接」可返回主页面" }
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
                                    // 服务器已关闭：等价于断开连接，同时清掉会话缓存，
                                    // 避免桌面端退出时再向已关闭的后端发请求
                                    disconnect(&st);
                                    st.status_msg.set("服务器已关闭".into());
                                    push_toast(&st, "服务器已关闭");
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
