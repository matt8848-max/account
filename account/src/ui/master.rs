//! 交易对象与货币管理页
//!
//! 货币/交易对象的增删改（重命名）与搜索；布局与旧版一致：
//! 两个表单 + 两个列表。

use super::app::{AccountApp, RenameTarget};
use super::app_state::{PAGE_MIN_WIDTH, apply_counterparty_rename, apply_currency_rename};
use super::confirm::{self, ConfirmAction};
use super::http;
use crate::server::models::{Counterparty, Currency, Money};
use gpui_kit::AppContext as _;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use gpui_kit::{
    Context, IntoElement, ParentElement as _, SharedString, Styled as _, Window, div, px,
};
use serde_json::json;

impl AccountApp {
    /// 对象/货币管理页。
    pub fn master_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let cur_q = self.cur_search.read(cx).value().trim().to_lowercase();
        let cp_q = self.cp_search.read(cx).value().trim().to_lowercase();
        let currencies: Vec<_> = self
            .currencies
            .iter()
            .filter(|c| cur_q.is_empty() || c.name.to_lowercase().contains(&cur_q))
            .cloned()
            .collect();
        let cur_names: std::collections::HashMap<i64, String> = self
            .currencies
            .iter()
            .map(|c| (c.id, c.name.clone()))
            .collect();
        let counterparties: Vec<_> = self
            .counterparties
            .iter()
            .filter(|c| {
                cp_q.is_empty()
                    || c.name.to_lowercase().contains(&cp_q)
                    || cur_names
                        .get(&c.currency_id)
                        .is_some_and(|n| n.to_lowercase().contains(&cp_q))
            })
            .cloned()
            .collect();

        let add_currency = {
            let entity = entity.clone();
            Button::new("add-currency")
                .primary()
                .label("添加")
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.add_currency(cx));
                })
        };
        let add_cp = {
            let entity = entity.clone();
            Button::new("add-counterparty")
                .primary()
                .label("添加")
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.add_counterparty(cx));
                })
        };
        let cur_pick_label = self
            .currencies
            .iter()
            .find(|c| c.id == self.new_cp_currency)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "选择货币".to_string());
        let pick_currency = {
            let entity = entity.clone();
            Button::new("pick-new-cp-currency")
                .small()
                .label(cur_pick_label)
                .on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| this.pick_new_cp_currency(window, cx));
                })
        };

        let currency_rows: Vec<gpui_kit::AnyElement> = currencies
            .iter()
            .enumerate()
            .map(|(i, c)| {
                self.currency_row(window, cx, c.clone(), i % 2 == 1)
                    .into_any_element()
            })
            .collect();
        let cp_rows: Vec<gpui_kit::AnyElement> = counterparties
            .iter()
            .enumerate()
            .map(|(i, c)| {
                self.counterparty_row(window, cx, c.clone(), i % 2 == 1)
                    .into_any_element()
            })
            .collect();

        v_flex()
            .w_full()
            .min_w(px(PAGE_MIN_WIDTH))
            .items_start()
            .justify_start()
            .gap_4()
            .child(div().text_xl().child("交易对象与货币管理"))
            .child(
                v_flex()
                    .gap_2()
                    .p_2()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(px(6.))
                    .child("添加货币")
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().w(px(220.)).child(Input::new(&self.new_currency_name)))
                            .child(add_currency),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .p_2()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(px(6.))
                    .child("添加交易对象")
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child("名称:")
                            .child(div().w(px(220.)).child(Input::new(&self.new_cp_name)))
                            .child("货币:")
                            .child(pick_currency)
                            .child("初始余额:")
                            .child(div().w(px(160.)).child(Input::new(&self.new_cp_balance)))
                            .child(add_cp),
                    ),
            )
            .child(
                h_flex()
                    .gap_4()
                    .items_start()
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_2()
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded(px(6.))
                            .p_2()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .justify_between()
                                    .child("货币")
                                    .child(format!(
                                        "{} / {}",
                                        currencies.len(),
                                        self.currencies.len()
                                    )),
                            )
                            .child(Input::new(&self.cur_search))
                            .children(currency_rows),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_2()
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded(px(6.))
                            .p_2()
                            .child(h_flex().gap_2().justify_between().child("交易对象").child(
                                format!("{} / {}", counterparties.len(), self.counterparties.len()),
                            ))
                            .child(Input::new(&self.cp_search))
                            .children(cp_rows),
                    ),
            )
    }

    /// 添加货币。
    pub fn add_currency(&mut self, cx: &mut Context<Self>) {
        let base = self.base(cx);
        let version = self.http_version;
        let name = self.new_currency_name.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.push_toast(cx, "货币名称不能为空");
            return;
        }
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    http::post(
                        &base,
                        "/api/currencies",
                        Some(json!({ "name": name })),
                        version,
                    )
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(v) => {
                        let id = v["id"].as_i64().unwrap_or(0);
                        let name = v["name"].as_str().unwrap_or("").to_string();
                        this.currencies.push(Currency { id, name });
                    }
                    Err(e) => this.push_toast(cx, format!("添加货币失败: {e}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 添加交易对象（含初始余额）。
    pub fn add_counterparty(&mut self, cx: &mut Context<Self>) {
        let base = self.base(cx);
        let version = self.http_version;
        let name = self.new_cp_name.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.push_toast(cx, "交易对象名称不能为空");
            return;
        }
        if self.new_cp_currency == 0 {
            self.push_toast(cx, "请选择货币");
            return;
        }
        let balance_text = self.new_cp_balance.read(cx).value().trim().to_string();
        let balance = match Money::from_string(&balance_text) {
            Ok(m) => m,
            Err(e) => {
                self.push_toast(cx, format!("初始余额格式不正确: {e}"));
                return;
            }
        };
        let currency_id = self.new_cp_currency;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    http::post(
                        &base,
                        "/api/counterparties",
                        Some(json!({
                            "name": name,
                            "currency_id": currency_id,
                            "balance": balance,
                        })),
                        version,
                    )
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(_) => this.refresh_counterparties(cx),
                    Err(e) => this.push_toast(cx, format!("添加交易对象失败: {e}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 选择新交易对象的货币。
    pub fn pick_new_cp_currency(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let currencies = self.currencies.clone();
        let entity = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let mut list = v_flex().gap_1();
            for currency in currencies.clone() {
                let entity = entity.clone();
                list = list.child(
                    Button::new(SharedString::from(format!("new-cur-{}", currency.id)))
                        .label(currency.name)
                        .on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            entity.update(cx, |this, cx| {
                                this.new_cp_currency = currency.id;
                                cx.notify();
                            });
                        }),
                );
            }
            dialog.title("选择货币").child(list)
        });
    }

    /// 单条货币行（灰白相间）。
    pub fn currency_row(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        currency: Currency,
        striped: bool,
    ) -> impl IntoElement {
        let id = currency.id;
        let name = currency.name.clone();
        let editing = self.renaming == Some(RenameTarget::Currency(id));
        if editing {
            return self.rename_editor(window, cx);
        }
        let entity = cx.entity();
        let bg = if striped {
            cx.theme().muted
        } else {
            cx.theme().background
        };
        let rename = {
            let entity = entity.clone();
            let name = name.clone();
            Button::new(SharedString::from(format!("rename-cur-{id}")))
                .small()
                .label("✎")
                .on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.start_rename(window, cx, RenameTarget::Currency(id), &name);
                    });
                })
        };
        let delete = {
            let entity = entity.clone();
            let name = name.clone();
            Button::new(SharedString::from(format!("del-cur-{id}")))
                .danger()
                .small()
                .label("✕")
                .on_click(move |_, window, cx| {
                    confirm::confirm(
                        window,
                        cx,
                        entity.clone(),
                        "确认删除该货币？",
                        format!("将删除货币: {name}\n\n若货币仍被交易对象或记录引用，删除将失败。"),
                        ConfirmAction::DeleteCurrency { id },
                    );
                })
        };
        h_flex()
            .gap_2()
            .items_center()
            .justify_between()
            .px_2()
            .py_1()
            .bg(bg)
            .child(div().flex_1().child(name))
            .child(rename)
            .child(delete)
            .into_any_element()
    }

    /// 单条交易对象行。
    pub fn counterparty_row(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        cp: Counterparty,
        striped: bool,
    ) -> impl IntoElement {
        let id = cp.id;
        let name = cp.name.clone();
        let balance = cp.balance.to_string();
        let cur_name = self
            .currencies
            .iter()
            .find(|c| c.id == cp.currency_id)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        let editing = self.renaming == Some(RenameTarget::Counterparty(id));
        if editing {
            return self.rename_editor(window, cx);
        }
        let entity = cx.entity();
        let bg = if striped {
            cx.theme().muted
        } else {
            cx.theme().background
        };
        let rename = {
            let entity = entity.clone();
            let name = name.clone();
            Button::new(SharedString::from(format!("rename-cp-{id}")))
                .small()
                .label("✎")
                .on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.start_rename(window, cx, RenameTarget::Counterparty(id), &name);
                    });
                })
        };
        let delete = {
            let entity = entity.clone();
            let name = name.clone();
            Button::new(SharedString::from(format!("del-cp-{id}")))
                .danger()
                .small()
                .label("✕")
                .on_click(move |_, window, cx| {
                    confirm::confirm(
                        window,
                        cx,
                        entity.clone(),
                        "确认删除该交易对象？",
                        format!("将删除交易对象: {name}\n\n若对象仍被记录引用，删除将失败。"),
                        ConfirmAction::DeleteCounterparty { id },
                    );
                })
        };
        h_flex()
            .gap_2()
            .items_center()
            .justify_between()
            .px_2()
            .py_1()
            .bg(bg)
            .child(div().flex_1().child(name))
            .child(div().child(balance))
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(cur_name),
            )
            .child(rename)
            .child(delete)
            .into_any_element()
    }

    /// 进入内联重命名：创建输入框并记录目标。
    pub fn start_rename(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        target: RenameTarget,
        current: &str,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(current));
        self.renaming = Some(target);
        self.rename_input = Some(input);
        cx.notify();
    }

    /// 内联重命名编辑器。
    pub fn rename_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let _ = &window;
        let Some(input) = self.rename_input.clone() else {
            return div().into_any_element();
        };
        let entity = cx.entity();
        let save = {
            let entity = entity.clone();
            Button::new("rename-save")
                .primary()
                .small()
                .label("保存")
                .on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| this.save_rename(window, cx));
                })
        };
        let cancel = {
            let entity = entity.clone();
            Button::new("rename-cancel")
                .small()
                .label("取消")
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.renaming = None;
                        this.rename_input = None;
                        cx.notify();
                    });
                })
        };
        h_flex()
            .gap_2()
            .items_center()
            .child(div().w(px(220.)).child(Input::new(&input)))
            .child(save)
            .child(cancel)
            .into_any_element()
    }

    /// 提交重命名：调用后端并同步前端公共状态。
    pub fn save_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = &window;
        let Some(target) = self.renaming else {
            return;
        };
        let Some(input) = self.rename_input.clone() else {
            return;
        };
        let new_name = input.read(cx).value().trim().to_string();
        if new_name.is_empty() {
            self.push_toast(cx, "名称不能为空");
            return;
        }
        let base = self.base(cx);
        let version = self.http_version;
        let id = match target {
            RenameTarget::Currency(id) | RenameTarget::Counterparty(id) => id,
        };
        let path = match target {
            RenameTarget::Currency(_) => "/api/currencies/rename",
            RenameTarget::Counterparty(_) => "/api/counterparties/rename",
        };
        let body = json!({ "id": id, "name": new_name });
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { http::post(&base, path, Some(body), version) })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(_) => {
                        match target {
                            RenameTarget::Currency(id) => apply_currency_rename(
                                &mut this.currencies,
                                &mut this.records,
                                &mut this.stats,
                                id,
                                &new_name,
                            ),
                            RenameTarget::Counterparty(id) => apply_counterparty_rename(
                                &mut this.counterparties,
                                &mut this.records,
                                &mut this.stats,
                                id,
                                &new_name,
                            ),
                        }
                        this.renaming = None;
                        this.rename_input = None;
                        this.push_toast(cx, "已重命名");
                    }
                    Err(e) => this.push_toast(cx, format!("重命名失败: {e}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
