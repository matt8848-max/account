//! 交易记录页
//!
//! 筛选（时间范围 + 交易对象）→ 拉取记录；新建交易组（组名 + 多行记录）；
//! 记录列表支持删除单条/整组与排序切换。布局与旧版保持一致。

use super::app::{AccountApp, GroupDraftRow};
use super::app_state::{
    CpTarget, DraftCurrencyCheck, PAGE_MIN_WIDTH, RecordView, check_draft_currency, cp_label,
    filter_candidates, sort_records,
};
use super::confirm::{self, ConfirmAction};
use super::date;
use super::http;
use crate::server::models::Money;
use gpui_kit::AppContext as _;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    date_picker::DatePicker,
    h_flex,
    input::{Input, InputState},
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use serde_json::json;

impl AccountApp {
    /// 交易记录页。
    pub fn records_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        let cps = self.counterparties.clone();
        let currencies = self.currencies.clone();
        let labels: Vec<(i64, String)> = cps
            .iter()
            .map(|c| (c.id, cp_label(c, &currencies)))
            .collect();
        let query_cp_label = if self.query_cp == 0 {
            "全部".to_string()
        } else {
            labels
                .iter()
                .find(|(id, _)| *id == self.query_cp)
                .map(|(_, l)| l.clone())
                .unwrap_or_else(|| "全部".to_string())
        };

        let pick_query_cp = {
            let entity = entity.clone();
            Button::new("pick-query-cp")
                .small()
                .label(format!("对象：{query_cp_label}"))
                .on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.pick_counterparty(window, cx, CpTarget::Query);
                    });
                })
        };
        let fetch_button = {
            let entity = entity.clone();
            Button::new("fetch-records")
                .primary()
                .label("🔍 拉取记录")
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.refresh_records(cx));
                })
        };
        let add_draft = {
            let entity = entity.clone();
            Button::new("add-draft")
                .label("➕ 添加记录行")
                .on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.add_draft(window, cx);
                        cx.notify();
                    });
                })
        };
        let save_group = {
            let entity = entity.clone();
            Button::new("save-group")
                .success()
                .label("💾 创建组")
                .on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| this.save_group(window, cx));
                })
        };

        let mut views = self.records.clone();
        sort_records(&mut views, self.records_order);
        let order = self.records_order;
        let draft_rows: Vec<gpui_kit::AnyElement> = (0..self.group_drafts.len())
            .map(|i| self.draft_row(window, cx, i).into_any_element())
            .collect();
        let record_rows: Vec<gpui_kit::AnyElement> = views
            .into_iter()
            .map(|r| self.record_row(window, cx, r).into_any_element())
            .collect();

        v_flex()
            .w_full()
            .min_w(px(PAGE_MIN_WIDTH))
            .items_start()
            .justify_start()
            .gap_3()
            .child(div().text_xl().child("交易记录浏览器"))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child("从:")
                    .child(div().w(px(200.)).child(DatePicker::new(&self.query_from)))
                    .child("到:")
                    .child(div().w(px(200.)).child(DatePicker::new(&self.query_to)))
                    .child(pick_query_cp)
                    .child(fetch_button),
            )
            .child(
                v_flex()
                    .gap_2()
                    .p_2()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(px(6.))
                    .child(
                        div()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child("新建交易组"),
                    )
                    .child(if cps.is_empty() {
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("请先在「对象/货币」页添加交易对象与货币")
                            .into_any_element()
                    } else {
                        div().into_any_element()
                    })
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child("组名:")
                            .child(div().w(px(320.)).child(Input::new(&self.new_group_name))),
                    )
                    .children(draft_rows)
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(add_draft)
                            .child(save_group),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().text_lg().child("记录"))
                    .child({
                        let entity = entity.clone();
                        Button::new("sort-toggle")
                            .small()
                            .label(format!("↕ {}", order.label()))
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.records_order = this.records_order.toggled();
                                    cx.notify();
                                });
                            })
                    }),
            )
            .children(record_rows)
            .child(if self.records.is_empty() {
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child("暂无记录，点击拉取")
                    .into_any_element()
            } else {
                div().into_any_element()
            })
    }

    /// 添加一条草稿记录行。
    pub fn add_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let amount = cx.new(|cx| InputState::new(window, cx).placeholder("金额（正数）"));
        let remark = cx.new(|cx| InputState::new(window, cx).placeholder("备注（≤256）"));
        let traded_at = date::new_datetime_picker(window, cx, Some(date::now_local()));
        self.group_drafts.push(GroupDraftRow {
            from_id: 0,
            to_id: 0,
            currency_id: 0,
            amount,
            remark,
            traded_at,
        });
    }

    /// 单条草稿记录行。
    pub fn draft_row(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        idx: usize,
    ) -> impl IntoElement {
        let _ = &window;
        let currencies = self.currencies.clone();
        let cps = self.counterparties.clone();
        let row = &self.group_drafts[idx];
        let from_label = cps
            .iter()
            .find(|c| c.id == row.from_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "转出".to_string());
        let to_label = cps
            .iter()
            .find(|c| c.id == row.to_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "转入".to_string());
        let currency_label = currencies
            .iter()
            .find(|c| c.id == row.currency_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "货币".to_string());
        let entity = cx.entity();

        let from_button = {
            let entity = entity.clone();
            Button::new(SharedString::from(format!("draft-from-{idx}")))
                .small()
                .label(format!("转出：{from_label}"))
                .on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.pick_counterparty(window, cx, CpTarget::DraftFrom(idx));
                    });
                })
        };
        let to_button = {
            let entity = entity.clone();
            Button::new(SharedString::from(format!("draft-to-{idx}")))
                .small()
                .label(format!("转入：{to_label}"))
                .on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.pick_counterparty(window, cx, CpTarget::DraftTo(idx));
                    });
                })
        };
        let currency_button = {
            let entity = entity.clone();
            Button::new(SharedString::from(format!("draft-cur-{idx}")))
                .small()
                .label(currency_label)
                .on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.pick_currency(window, cx, idx);
                    });
                })
        };
        let remove_button = {
            let entity = entity.clone();
            Button::new(SharedString::from(format!("draft-remove-{idx}")))
                .danger()
                .small()
                .label("✕")
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        if idx < this.group_drafts.len() {
                            this.group_drafts.remove(idx);
                        }
                        cx.notify();
                    });
                })
        };
        // 复制上一行的转出/转入/货币/日期：首行没有上一行，按钮禁用。
        let copy_previous = {
            let entity = entity.clone();
            Button::new(SharedString::from(format!("draft-copy-{idx}")))
                .small()
                .disabled(idx == 0)
                .label("⧉ 复制上一行")
                .on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.copy_previous_draft(window, cx, idx);
                    });
                })
        };

        h_flex()
            .gap_2()
            .items_center()
            .child(from_button)
            .child(to_button)
            .child(div().w(px(130.)).child(Input::new(&row.amount)))
            .child(currency_button)
            .child(div().w(px(200.)).child(DatePicker::new(&row.traded_at)))
            .child(div().w(px(200.)).child(Input::new(&row.remark)))
            .child(copy_previous)
            .child(remove_button)
    }

    /// 提交新建组。
    pub fn save_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let base = self.base(cx);
        let version = self.http_version;
        let name = self.new_group_name.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.push_toast(cx, "组名称不能为空");
            return;
        }
        if self.group_drafts.is_empty() {
            self.push_toast(cx, "请至少添加一条记录");
            return;
        }
        let mut records = Vec::new();
        for (i, d) in self.group_drafts.iter().enumerate() {
            if d.from_id == 0 || d.to_id == 0 || d.currency_id == 0 {
                self.push_toast(cx, format!("第 {} 行：请选择转出/转入对象与货币", i + 1));
                return;
            }
            if d.from_id == d.to_id {
                self.push_toast(cx, format!("第 {} 行：转出与转入不能是同一对象", i + 1));
                return;
            }
            let amount_text = d.amount.read(cx).value().trim().to_string();
            let amount = match Money::from_string(&amount_text) {
                Ok(m) if m.value > 0 => m,
                _ => {
                    self.push_toast(cx, format!("第 {} 行：金额格式不正确", i + 1));
                    return;
                }
            };
            let remark = d.remark.read(cx).value().trim().to_string();
            let Some(traded_at) = date::picker_datetime(d.traded_at.read(cx)) else {
                self.push_toast(cx, format!("第 {} 行：请选择交易时间", i + 1));
                return;
            };
            records.push(json!({
                "from_id": d.from_id,
                "to_id": d.to_id,
                "amount": amount,
                "currency_id": d.currency_id,
                "remark": if remark.is_empty() { None } else { Some(remark) },
                "traded_at": date::to_api_datetime(traded_at),
            }));
        }
        let body = json!({ "name": name, "records": records });
        // 注意：这里**不能**先清空草稿/组名。后端仍可能因业务校验失败而拒绝创建，
        // 若提前清空，用户已录入的内容会丢失。成功回调里再清理。
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(
                    async move { http::post(&base, "/api/groups", Some(body), version) },
                )
                .await;
            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(v) => {
                        let id = v["id"].as_i64().unwrap_or(0);
                        this.group_drafts.clear();
                        this.new_group_name
                            .update(cx, |state, cx| state.set_value("", window, cx));
                        this.push_toast(cx, format!("组已创建 id={id}"));
                        this.refresh_counterparties(cx);
                        this.refresh_records(cx);
                    }
                    Err(e) => this.push_toast(cx, format!("创建失败: {e}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 单条交易记录行。
    pub fn record_row(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        rec: RecordView,
    ) -> impl IntoElement {
        let _ = &window;
        let entity = cx.entity();
        let id = rec.id;
        let gid = rec.group_id;
        let traded_at = rec.traded_at.clone();
        let traded_at_display = date::display_api_datetime(&traded_at);
        let group_months = rec.group_months.clone();
        let group_label = rec
            .group_name
            .clone()
            .unwrap_or_else(|| format!("组#{gid}"));
        let amount = format!("{} {}", rec.amount, rec.currency);
        let remark = rec.remark.clone();
        let from_to = format!("{} → {}", rec.from, rec.to);

        let delete_record = {
            let entity = entity.clone();
            let traded_at = traded_at.clone();
            let from_to = from_to.clone();
            let amount = amount.clone();
            Button::new(SharedString::from(format!("del-rec-{id}")))
                .danger()
                .small()
                .label("🗑 删")
                .on_click(move |_, window, cx| {
                    confirm::confirm(
                        window,
                        cx,
                        entity.clone(),
                        "确认删除该记录？",
                        format!("将删除记录：\n{from_to}\n金额 {amount}\n时间 {traded_at}"),
                        ConfirmAction::DeleteRecord {
                            id,
                            traded_at: traded_at.clone(),
                        },
                    );
                })
        };
        let group_label_text = group_label.clone();
        let delete_group = {
            let entity = entity.clone();
            let group_months = group_months.clone();
            Button::new(SharedString::from(format!("del-grp-{gid}-{id}")))
                .danger()
                .small()
                .label("删组")
                .on_click(move |_, window, cx| {
                    confirm::confirm(
                        window,
                        cx,
                        entity.clone(),
                        "确认删除该组？",
                        format!(
                            "确定删除组 {group_label}？\n该组下全部记录将一并删除，余额自动回滚。"
                        ),
                        ConfirmAction::DeleteGroup {
                            id: gid,
                            group_months: group_months.clone(),
                        },
                    );
                })
        };

        h_flex()
            .gap_2()
            .items_center()
            .text_sm()
            .child(div().w(px(150.)).child(traded_at_display))
            .child(div().flex_1().min_w_0().child(from_to))
            .child(
                div()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child(amount),
            )
            .child(if let Some(r) = remark {
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("({r})"))
                    .into_any_element()
            } else {
                div().into_any_element()
            })
            .child(delete_record)
            // 组名位于「删除本条」与「删除整组」之间，便于确认该记录属于哪个组。
            .child(
                div()
                    .max_w(px(180.))
                    .truncate()
                    .text_color(cx.theme().muted_foreground)
                    .child(group_label_text),
            )
            .child(delete_group)
    }

    /// 弹出交易对象选择窗口（筛选 / 草稿转出 / 草稿转入共用）。
    ///
    /// 与「选择货币」保持一致：使用 `open_dialog` 弹出窗口；窗口内保留搜索框，
    /// 交易对象较多时也能快速定位。
    pub fn pick_counterparty(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        target: CpTarget,
    ) {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索交易对象名称/货币"));
        self._subscriptions.push(cx.subscribe_in(
            &search,
            window,
            |_: &mut Self,
             _: &Entity<InputState>,
             _: &gpui_kit::component::input::InputEvent,
             _: &mut Window,
             cx| {
                cx.notify();
            },
        ));
        let entity = cx.entity();
        let currencies = self.currencies.clone();
        let cps = self.counterparties.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let search = search.clone();
            let currencies = currencies.clone();
            let cps = cps.clone();
            let entity = entity.clone();
            dialog.title("选择交易对象").content(move |content, _, cx| {
                let query = search.read(cx).value().trim().to_string();
                let mut candidates: Vec<(i64, String)> = cps
                    .iter()
                    .map(|c| (c.id, cp_label(c, &currencies)))
                    .collect();
                if matches!(target, CpTarget::Query) {
                    candidates.insert(0, (0, "全部".to_string()));
                }
                let rows: Vec<gpui_kit::AnyElement> = filter_candidates(&candidates, &query)
                    .into_iter()
                    .map(|(id, label)| {
                        let entity = entity.clone();
                        Button::new(SharedString::from(format!("cp-pick-{id}")))
                            .w_full()
                            .label(label)
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                entity.update(cx, |this, cx| {
                                    this.apply_cp_choice(target, id, cx);
                                });
                            })
                            .into_any_element()
                    })
                    .collect();
                content
                    .child(Input::new(&search))
                    .child(
                        v_flex()
                            .id("cp-picker-list")
                            .role(Role::List)
                            .gap_1()
                            .max_h(px(280.))
                            .overflow_y_scrollbar()
                            .children(rows),
                    )
                    .child(
                        h_flex().justify_end().child(
                            Button::new("cp-picker-cancel")
                                .small()
                                .label("取消")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        ),
                    )
            })
        });
    }

    /// 应用交易对象选择结果。
    ///
    /// 草稿行两侧都选中后自动校验货币：一致时自动填入货币类型；
    /// 不一致时提示并清空转入方（与用户先选转出还是先选转入无关）。
    pub fn apply_cp_choice(&mut self, target: CpTarget, id: i64, cx: &mut Context<Self>) {
        if matches!(target, CpTarget::Query) {
            self.query_cp = id;
            self.refresh_records(cx);
            return;
        }
        let Some(idx) = target.draft_index() else {
            return;
        };
        let Some(row) = self.group_drafts.get(idx) else {
            return;
        };
        let from_id = if matches!(target, CpTarget::DraftFrom(_)) {
            id
        } else {
            row.from_id
        };
        let to_id = if matches!(target, CpTarget::DraftTo(_)) {
            id
        } else {
            row.to_id
        };
        let from_cur = self
            .counterparties
            .iter()
            .find(|c| c.id == from_id)
            .map(|c| c.currency_id);
        let to_cur = self
            .counterparties
            .iter()
            .find(|c| c.id == to_id)
            .map(|c| c.currency_id);
        let check = check_draft_currency(from_cur, to_cur);
        let mut mismatch = false;
        if let Some(row) = self.group_drafts.get_mut(idx) {
            match target {
                CpTarget::DraftFrom(_) => row.from_id = id,
                CpTarget::DraftTo(_) => row.to_id = id,
                CpTarget::Query => {}
            }
            match check {
                DraftCurrencyCheck::Matched(currency_id) => row.currency_id = currency_id,
                DraftCurrencyCheck::Mismatch => {
                    row.to_id = 0;
                    mismatch = true;
                }
                DraftCurrencyCheck::Pending => {}
            }
        }
        if mismatch {
            self.push_toast(
                cx,
                format!(
                    "第 {} 行：转入/转出对象的货币类型不一致，已清空转入方",
                    idx + 1
                ),
            );
        }
        cx.notify();
    }

    /// 复制上一行的转出/转入/货币类型/日期（不含金额与备注）。
    pub fn copy_previous_draft(&mut self, window: &mut Window, cx: &mut Context<Self>, idx: usize) {
        if idx == 0 || idx >= self.group_drafts.len() {
            return;
        }
        let (from_id, to_id, currency_id, traded_at) = {
            let prev = &self.group_drafts[idx - 1];
            (
                prev.from_id,
                prev.to_id,
                prev.currency_id,
                date::picker_datetime(prev.traded_at.read(cx)),
            )
        };
        let Some(row) = self.group_drafts.get_mut(idx) else {
            return;
        };
        row.from_id = from_id;
        row.to_id = to_id;
        row.currency_id = currency_id;
        if let Some(dt) = traded_at {
            row.traded_at
                .update(cx, |state, cx| state.set_date_time(dt, window, cx));
        }
        cx.notify();
    }

    /// 弹出货币选择框（草稿行）。
    pub fn pick_currency(&mut self, window: &mut Window, cx: &mut Context<Self>, idx: usize) {
        let currencies = self.currencies.clone();
        let entity = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let mut list = v_flex().gap_1();
            for currency in currencies.clone() {
                let entity = entity.clone();
                list = list.child(
                    Button::new(SharedString::from(format!("cur-pick-{}", currency.id)))
                        .label(currency.name)
                        .on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            entity.update(cx, |this, cx| {
                                if let Some(row) = this.group_drafts.get_mut(idx) {
                                    row.currency_id = currency.id;
                                }
                                cx.notify();
                            });
                        }),
                );
            }
            dialog.title("选择货币").child(list)
        });
    }
}
