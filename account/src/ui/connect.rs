//! 连接页
//!
//! 五行布局（整体居中）：
//! 1. 标题
//! 2. 地址输入 + HTTP 版本选择（新增）
//! 3. 默认地址灰字提示
//! 4. 连接 / 断开 / 关闭服务器
//! 5. 信息提示 + Toast
//!
//! 「连接」兼任“进入/返回主页面”：地址与协议未变则直接返回，否则先断开再重连。

use super::app::{AccountApp, DEFAULT_BASE};
use super::confirm::{self, ConfirmAction};
use super::http::HttpVersion;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    v_flex,
};
use gpui_kit::{Context, IntoElement, ParentElement as _, Styled as _, Window, div, px};

impl AccountApp {
    /// 连接页渲染。
    pub fn connect_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let _ = &window;
        let connected = self.connected;
        let connecting = self.connecting;
        let status = self.status_msg.clone();
        let toast = self.toast.clone();
        let entity = cx.entity();

        let connect_button = {
            let entity = entity.clone();
            Button::new("connect")
                .primary()
                .disabled(connecting)
                .label(if connecting {
                    "连接中..."
                } else {
                    "🔗 连接"
                })
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.connect(cx));
                })
        };
        let disconnect_button = {
            let entity = entity.clone();
            Button::new("disconnect")
                .disabled(!connected)
                .label("⏏ 断开")
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        if this.connected {
                            this.disconnect(cx);
                            this.status_msg = "已断开连接".to_string();
                            this.push_toast(cx, "已断开连接");
                        }
                    });
                })
        };
        let close_button = {
            let entity = entity.clone();
            Button::new("close-server")
                .danger()
                .disabled(!connected)
                .label("⛔ 关闭服务器")
                .on_click(move |_, window, cx| {
                    if let Some(active) = super::http::session::active()
                        && active.0.is_empty()
                    {
                        return;
                    }
                    confirm::confirm(
                        window,
                        cx,
                        entity.clone(),
                        "确认关闭服务器？",
                        "关闭后后端将停止运行，客户端将无法继续访问。\n请确认是否关闭？"
                            .to_string(),
                        ConfirmAction::CloseServer,
                    );
                })
        };

        let version_buttons = HttpVersion::all().into_iter().map(|v| {
            let entity = entity.clone();
            Button::new(match v {
                HttpVersion::Http2 => "http-version-2",
                HttpVersion::Http3 => "http-version-3",
            })
            .small()
            .selected(self.http_version == v)
            .label(v.label())
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    this.http_version = v;
                    cx.notify();
                });
            })
        });

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .p_10()
            .gap_3()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(div().text_2xl().child("记账软件"))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().w(px(360.)).child(Input::new(&self.address)))
                    .child(div().text_sm().child("HTTP 版本:"))
                    .children(version_buttons),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("后端地址默认 {DEFAULT_BASE}")),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(connect_button)
                    .child(disconnect_button)
                    .child(close_button),
            )
            .child(if status.is_empty() {
                div().text_sm().child("").into_any_element()
            } else {
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(status)
                    .into_any_element()
            })
            .child(if connected {
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("点击「连接」可返回主页面")
                    .into_any_element()
            } else {
                div().into_any_element()
            })
            .child(if let Some(msg) = toast {
                div()
                    .text_sm()
                    .text_color(gpui_kit::rgb(0xC0392B))
                    .child(msg)
                    .into_any_element()
            } else {
                div().into_any_element()
            })
    }
}
