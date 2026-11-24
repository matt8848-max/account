//! 通用确认弹窗
//!
//! 删除记录/组、删除货币/对象、关闭服务器等危险操作统一走这里：
//! 先弹出 gpui-kit 的 Dialog，确认后由 [`crate::ui::app::AccountApp::perform_confirm`]
//! 执行对应异步操作。

use crate::ui::app::AccountApp;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
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

/// 打开「确认导出范围」弹窗。
///
/// 在通用确认弹窗基础上增加两个可选内容：
/// - 导出交易对象卡片（独立 sheet，带自动筛选）；
/// - 导出桑基图/流向图（独立 sheet）。
///
/// 勾选状态直接写入 `AccountApp.export_cards` / `export_sankey`，
/// 确认后由 [`AccountApp::perform_confirm`] 读取并执行导出。
pub fn confirm_export(
    window: &mut Window,
    cx: &mut App,
    entity: Entity<AccountApp>,
    message: String,
) {
    window.open_dialog(cx, move |dialog, _, _| {
        let entity = entity.clone();
        let message = message.clone();
        dialog.title("确认导出范围").content(move |content, _, cx| {
            let cards_checked = entity.read(cx).export_cards;
            let sankey_checked = entity.read(cx).export_sankey;
            let toggle_cards = {
                let entity = entity.clone();
                move |checked: &bool, _: &mut Window, cx: &mut App| {
                    entity.update(cx, |this, cx| {
                        this.export_cards = *checked;
                        cx.notify();
                    });
                }
            };
            let toggle_sankey = {
                let entity = entity.clone();
                move |checked: &bool, _: &mut Window, cx: &mut App| {
                    entity.update(cx, |this, cx| {
                        this.export_sankey = *checked;
                        cx.notify();
                    });
                }
            };
            let confirm = {
                let entity = entity.clone();
                Button::new("confirm-ok")
                    .danger()
                    .label("确认")
                    .on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        entity.update(cx, |this, cx| {
                            this.perform_confirm(ConfirmAction::Export, cx);
                        });
                    })
            };
            content
                .child(message.clone())
                .child(
                    Checkbox::new("export-cards")
                        .checked(cards_checked)
                        .label("导出交易对象卡片（独立 sheet，可筛选）")
                        .on_change(toggle_cards),
                )
                .child(
                    Checkbox::new("export-sankey")
                        .checked(sankey_checked)
                        .label("导出桑基图/流向图（独立 sheet）")
                        .on_change(toggle_sankey),
                )
                .child(
                    h_flex()
                        .gap(px(8.))
                        .justify_end()
                        .child(
                            Button::new("confirm-cancel")
                                .label("取消")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(confirm),
                )
        })
    });
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
