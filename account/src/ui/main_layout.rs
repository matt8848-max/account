//! 主界面布局
//!
//! 与旧版保持一致：左侧 200px 菜单栏、右侧内容区、底部 26px 状态栏。
//! 页面切换只改 `page` 字段，布局本身不重建。

use super::app::AccountApp;
use super::state::Page;
use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, Window, div, px,
};

impl AccountApp {
    /// 侧栏 + 内容区 + 底部状态栏。
    pub fn main_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let page = self.page;

        let nav = |id: &'static str, label: &'static str, target: Page| {
            let entity = entity.clone();
            Button::new(id)
                .ghost()
                .w_full()
                .selected(page == target)
                .label(label)
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.page = target;
                        if target == Page::Records {
                            // 进入记录页自动刷新（与旧版一致）
                            this.refresh_records(cx);
                        }
                        cx.notify();
                    });
                })
        };

        let sidebar = {
            let entity = entity.clone();
            v_flex()
                .w(px(200.))
                .h_full()
                .flex_shrink_0()
                .gap_2()
                .p_2()
                .border_r_1()
                .border_color(cx.theme().border)
                .child(div().text_lg().child("菜单栏"))
                .child(nav("nav-records", "🧾 交易记录", Page::Records))
                .child(nav("nav-master", "🏷 对象/货币", Page::Master))
                .child(nav("nav-stats", "📊 数据统计", Page::Stats))
                .child(
                    div().mt_auto().child(
                        Button::new("back-connect")
                            .small()
                            .w_full()
                            .label("↩ 返回连接页")
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.show_connect = true;
                                    cx.notify();
                                });
                            }),
                    ),
                )
        };

        let content = v_flex()
            .id("content-scroll")
            .flex_1()
            .h_full()
            .min_h_0()
            .min_w_0()
            .p_4()
            .overflow_y_scrollbar()
            .child(match self.page {
                Page::Records => self.records_page(window, cx).into_any_element(),
                Page::Master => self.master_page(window, cx).into_any_element(),
                Page::Stats => self.stats_page(window, cx).into_any_element(),
            });

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(h_flex().flex_1().min_h_0().child(sidebar).child(content))
            .child(self.status_bar(window, cx))
    }
}
