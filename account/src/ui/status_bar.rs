//! 底部状态栏
//!
//! 一行固定 26px，承载：
//! - 后端存活/失联（点击展开详情）
//! - 更新检测状态（点击打开/重试）
//! - 提示信息 + 长任务进度条

use super::app::AccountApp;
use super::app_state::TaskProgress;
use super::update::{self, UpdateState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
};
use gpui_kit::{Context, IntoElement, ParentElement as _, Styled as _, Window, div, px};

impl AccountApp {
    /// 状态栏渲染。
    pub fn status_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = &window;
        let entity = cx.entity();
        let alive = self.live_ok;
        let (dot, text) = if alive {
            ("●", "存活")
        } else {
            ("○", "失联")
        };
        let color = if alive {
            cx.theme().success
        } else {
            gpui_kit::hsla(0.0, 0.7, 0.4, 1.0)
        };
        let detail_open = self.detail_open;
        let base = self.base(cx);
        let api_version = self.api_version.clone();
        let live_at = self.live_at.clone();
        let update_state = self.update_state.clone();
        let show_update = !matches!(update_state, UpdateState::Idle);
        let toast = self.toast.clone();
        let task = self.task.clone();

        let backend_button = {
            let entity = entity.clone();
            Button::new("backend-status")
                .ghost()
                .xsmall()
                .label(format!("后端：{dot} {text}"))
                .text_color(color)
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.detail_open = !this.detail_open;
                        cx.notify();
                    });
                })
        };

        let update_button = if show_update {
            let entity = entity.clone();
            let state_for_click = update_state.clone();
            let label = update_state.label();
            let clickable = update_state.is_clickable();
            let color = match &update_state {
                UpdateState::Available { .. } => gpui_kit::hsla(0.0, 0.7, 0.4, 1.0),
                UpdateState::Latest { .. } => cx.theme().success,
                _ => cx.theme().muted_foreground,
            };
            Some(
                Button::new("update-status")
                    .ghost()
                    .xsmall()
                    .disabled(!clickable)
                    .label(label)
                    .text_color(color)
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| match &state_for_click {
                            UpdateState::Available { page, .. } => {
                                if let Err(e) = update::open_external(page) {
                                    this.push_toast(cx, e);
                                }
                            }
                            UpdateState::Latest { .. } | UpdateState::Failed { .. } => {
                                this.spawn_update_check(cx);
                            }
                            _ => {}
                        });
                    }),
            )
        } else {
            None
        };

        let detail = if detail_open {
            Some(
                div()
                    .absolute()
                    .left(px(8.))
                    .bottom(px(26.))
                    .p_2()
                    .gap_1()
                    .flex()
                    .flex_col()
                    .min_w(px(300.))
                    .bg(cx.theme().background)
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(px(6.))
                    .text_sm()
                    .child(
                        div()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child("后端连接详情"),
                    )
                    .child(format!("地址：{base}"))
                    .child(format!("版本：{api_version}"))
                    .child(format!("状态：{text}（上次探测 {live_at}）"))
                    .child("自动探测间隔：5 分钟"),
            )
        } else {
            None
        };

        let progress = task.as_ref().map(|t| {
            (
                div().text_xs().child(t.describe()),
                div()
                    .w(px(120.))
                    .h(px(8.))
                    .rounded(px(4.))
                    .bg(cx.theme().border)
                    .child(
                        div()
                            .h_full()
                            .w(px(AccountApp::progress_width(t, 120.)))
                            .bg(cx.theme().primary)
                            .rounded(px(4.)),
                    ),
            )
        });

        h_flex()
            .relative()
            .h(px(26.))
            .flex_shrink_0()
            .items_center()
            .gap_3()
            .px_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted)
            .text_xs()
            .child(backend_button)
            .children(update_button)
            .children(detail)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .items_center()
                    .children(progress.map(|(label, bar)| {
                        h_flex().gap_2().items_center().child(label).child(bar)
                    }))
                    .child(if let Some(msg) = toast {
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .child(msg)
                            .into_any_element()
                    } else {
                        div().into_any_element()
                    }),
            )
    }

    /// 把任务百分比换算为像素宽度。
    pub fn progress_width(task: &TaskProgress, total_px: f32) -> f32 {
        total_px * task.percent() as f32 / 100.0
    }
}
