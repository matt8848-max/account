//! 底部状态栏
//!
//! 一行固定高度承载三类信息（取代原顶部黑色 Toast 浮层）：
//! - **后端连接状态**：只显示「存活 / 失联」（有颜色），点击在上方展开详情浮层；
//!   浮层用绝对定位，**不改变底栏高度**，因此不会挤动页面内容。
//! - **更新检测**：有新版（红，点击打开发布页）/ 已最新（绿，点击重测）/ 失败（灰，点击重测）
//! - **提示信息 + 长任务进度**：提示来自 `push_toast`，进度来自 `GlobalState.task`
//!
//! 三类组件都只占一行：详情浮层向上弹出，进度条为细横条，避免布局跳动。

use super::update::{self, UpdateState};
use super::{GlobalState, LIVE_INTERVAL_MS, now_str, open_external, ping, push_toast};
use dioxus::prelude::*;

/// 后端状态在底栏的展示（圆点、文案、颜色）
fn backend_badge(ok: bool) -> (&'static str, &'static str, &'static str) {
    if ok {
        ("●", "存活", "#2e7d32")
    } else {
        ("○", "失联", "#c0392b")
    }
}

#[component]
pub(crate) fn StatusBar() -> Element {
    let st = use_context::<GlobalState>();
    // 后端详情浮层开关（默认收起）
    let show_detail = use_signal(|| false);

    let live_ok = (st.live_ok)();
    let (live_dot, live_text, live_color) = backend_badge(live_ok);
    let detail_open = (show_detail)();
    let base = (st.base)();
    let api_version = (st.api_version)();
    let live_at = (st.live_at)();

    let update_state = (st.update_state)();
    let update_text = update_state.label();
    let update_color = update_state.color();
    let update_title = update_state.title();
    let update_clickable = update_state.is_clickable();
    let update_cursor = if update_clickable {
        "pointer"
    } else {
        "default"
    };
    // 未配置仓库（Idle）时整块隐藏，避免出现无意义的空按钮
    let show_update = !matches!(update_state, UpdateState::Idle);

    let toast_msg = (st.toast)();
    let task = (st.task)();
    // 自动探测间隔（分钟），用于详情浮层展示
    let probe_minutes = LIVE_INTERVAL_MS / 60_000;

    rsx! {
        div {
            style: "position:relative;display:flex;align-items:center;gap:14px;height:26px;padding:0 10px;border-top:1px solid #ddd;background:#f7f7f7;font-size:12px;color:#555;flex-shrink:0",

            // 后端连接状态：点击展开详情
            button {
                style: "background:none;border:none;padding:0;cursor:pointer;font-size:12px;color:{live_color};white-space:nowrap",
                onclick: move |_| {
                    let mut show_detail = show_detail;
                    show_detail.toggle();
                },
                "后端：{live_dot} {live_text}"
            }

            // 详情浮层：绝对定位于底栏上方，不占用布局高度
            if detail_open {
                div {
                    style: "position:absolute;left:8px;bottom:100%;margin-bottom:4px;background:#fff;border:1px solid #ddd;border-radius:6px;box-shadow:0 4px 12px rgba(0,0,0,0.15);padding:10px 12px;min-width:300px;z-index:1500;display:flex;flex-direction:column;gap:6px;color:#333",
                    div { style: "font-weight:600", "后端连接详情" }
                    div { "地址：{base}" }
                    div { "版本：{api_version}" }
                    div { "状态：{live_text}（上次探测 {live_at}）" }
                    div { "自动探测间隔：{probe_minutes} 分钟" }
                    div { style: "display:flex;gap:8px;justify-content:flex-end;margin-top:2px",
                        button {
                            onclick: move |_| {
                                let mut st = st;
                                spawn(async move {
                                    let ok = ping(&st).await;
                                    st.live_ok.set(ok);
                                    st.live_at.set(now_str());
                                });
                            },
                            "立即重测"
                        }
                        button {
                            onclick: move |_| {
                                let mut st = st;
                                let mut show_detail = show_detail;
                                show_detail.set(false);
                                // 仅切回连接页视图，不断开连接
                                st.show_connect.set(true);
                            },
                            "查看连接页"
                        }
                    }
                }
            }

            // 更新检测：无展开，点击行为随状态变化
            if show_update {
                button {
                    style: "background:none;border:none;padding:0;cursor:{update_cursor};font-size:12px;color:{update_color};white-space:nowrap",
                    disabled: !update_clickable,
                    title: "{update_title}",
                    onclick: move |_| {
                        let st = st;
                        match (st.update_state)() {
                            // 有新版本：打开发布页（桌面用系统浏览器，wasm 新标签页）
                            UpdateState::Available { page, .. } => {
                                if let Err(e) = open_external(&page) {
                                    push_toast(&st, &e);
                                }
                            }
                            // 已最新 / 失败：重新检测
                            UpdateState::Latest { .. } | UpdateState::Failed { .. } => {
                                update::spawn_check(st);
                            }
                            UpdateState::Idle | UpdateState::Checking => {}
                        }
                    },
                    "{update_text}"
                }
            }

            // 提示信息 + 长任务进度
            div { style: "display:flex;align-items:center;gap:8px;flex:1;min-width:0",
                if let Some(t) = &task {
                    span { style: "color:#555;white-space:nowrap", "{t.describe()}" }
                    div { style: "width:120px;height:8px;background:#e0e0e0;border-radius:4px;overflow:hidden;flex-shrink:0",
                        div { style: "height:100%;width:{t.percent()}%;background:#3498db;transition:width 0.2s" }
                    }
                }
                if let Some(msg) = &toast_msg {
                    div {
                        style: "flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:#333",
                        title: "{msg}",
                        "{msg}"
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::backend_badge;

    /// 验证后端状态的底栏展示：存活为绿色实心圆，失联为红色空心圆
    #[test]
    fn backend_badge_should_map_alive_and_lost() {
        assert_eq!(backend_badge(true), ("●", "存活", "#2e7d32"));
        assert_eq!(backend_badge(false), ("○", "失联", "#c0392b"));
    }
}
