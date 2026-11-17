//! 通用确认弹窗
//!
//! 删除记录/组、删除货币/对象、关闭服务器等危险操作统一走这里：
//! 先弹出 gpui-kit 的 Dialog，确认后由 [`crate::ui::app::AccountApp::perform_confirm`]
//! 执行对应异步操作。

use crate::ui::app::AccountApp;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::h_flex;
use gpui_kit::{App, Entity, ParentElement as _, Styled as _, Window, px};

/// 需要用户二次确认的操作。
#[derive(Debug, Clone, PartialEq)]
pub enum ConfirmAction {
    DeleteCurrency {
        id: i64,
    },
    DeleteCounterparty {
        id: i64,
    },
    DeleteRecord {
        id: i64,
        traded_at: String,
    },
    DeleteGroup {
        id: i64,
        group_months: Option<String>,
    },
    CloseServer,
    Export,
}

/// 打开确认弹窗；`title` 为静态标题，`message` 为详情文本。
pub fn confirm(
    window: &mut Window,
    cx: &mut App,
    entity: Entity<AccountApp>,
    title: &'static str,
    message: String,
    action: ConfirmAction,
) {
    window.open_dialog(cx, move |dialog, _, _| {
        let entity = entity.clone();
        let action = action.clone();
        let message = message.clone();
        dialog.title(title).content(move |content, _, _| {
            content.child(message.clone()).child(
                h_flex()
                    .gap(px(8.))
                    .justify_end()
                    .child(
                        Button::new("confirm-cancel")
                            .label("取消")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(Button::new("confirm-ok").danger().label("确认").on_click({
                        let entity = entity.clone();
                        let action = action.clone();
                        move |_, window, cx| {
                            window.close_dialog(cx);
                            let action = action.clone();
                            entity.update(cx, |this, cx| {
                                this.perform_confirm(action, cx);
                            });
                        }
                    })),
            )
        })
    });
}
