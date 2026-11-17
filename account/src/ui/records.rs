//! 交易记录页
//!
//! 筛选（时间范围 + 交易对象）→ 拉取记录；新建交易组（组名 + 多行记录）；
//! 记录列表支持删除单条/整组与排序切换。布局与旧版保持一致。

use super::app::{AccountApp, GroupDraftRow};
use super::confirm::{self, ConfirmAction};
use super::http;
use super::state::{RecordView, build_iso, cp_label, sort_records};
use crate::server::models::Money;
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

/// 交易对象选择目标（筛选 / 草稿转出 / 草稿转入）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpTarget {
    Query,
    DraftFrom(usize),
    DraftTo(usize),
}

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
            .gap_3()
            .child(div().text_xl().child("交易记录浏览器"))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child("从:")
                    .child(div().w(px(180.)).child(Input::new(&self.query_from)))
                    .child("到:")
                    .child(div().w(px(180.)).child(Input::new(&self.query_to)))
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
        let traded_at =
            cx.new(|cx| InputState::new(window, cx).default_value(super::state::now_local_iso()));
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

        h_flex()
            .gap_2()
            .items_center()
            .child(from_button)
            .child(to_button)
            .child(div().w(px(130.)).child(Input::new(&row.amount)))
            .child(currency_button)
            .child(div().w(px(170.)).child(Input::new(&row.traded_at)))
            .child(div().w(px(200.)).child(Input::new(&row.remark)))
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
            let traded_at = d.traded_at.read(cx).value().trim().to_string();
            records.push(json!({
                "from_id": d.from_id,
                "to_id": d.to_id,
                "amount": amount,
                "currency_id": d.currency_id,
                "remark": if remark.is_empty() { None } else { Some(remark) },
                "traded_at": build_iso(&traded_at),
            }));
        }
        let body = json!({ "name": name, "records": records });
        // 先清空草稿与组名，避免成功后重复提交。
        self.group_drafts.clear();
        self.new_group_name
            .update(cx, |state, cx| state.set_value("", window, cx));

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(
                    async move { http::post(&base, "/api/groups", Some(body), version) },
                )
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(v) => {
                        let id = v["id"].as_i64().unwrap_or(0);
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
        let delete_group = {
            let entity = entity.clone();
            let group_months = group_months.clone();
            Button::new(SharedString::from(format!("del-grp-{gid}")))
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
            .child(div().w(px(150.)).child(traded_at))
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
            .child(delete_group)
    }

    /// 弹出交易对象选择框；`target` 决定是筛选还是草稿行。
    pub fn pick_counterparty(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        target: CpTarget,
    ) {
        let cps = self.counterparties.clone();
        let entity = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let mut list = v_flex().gap_1();
            let all_entity = entity.clone();
            list = list.child(Button::new("cp-pick-all").label("全部").on_click(
                move |_, window, cx| {
                    window.close_dialog(cx);
                    all_entity.update(cx, |this, cx| {
                        this.query_cp = 0;
                        this.refresh_records(cx);
                        cx.notify();
                    });
                },
            ));
            for cp in cps.clone() {
                let id = cp.id;
                let name = cp.name.clone();
                let entity = entity.clone();
                list = list.child(
                    Button::new(SharedString::from(format!("cp-pick-{id}")))
                        .label(name)
                        .on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            entity.update(cx, |this, cx| {
                                match target {
                                    CpTarget::Query => {
                                        this.query_cp = id;
                                        this.refresh_records(cx);
                                    }
                                    CpTarget::DraftFrom(i) => {
                                        if let Some(row) = this.group_drafts.get_mut(i) {
                                            row.from_id = id;
                                        }
                                    }
                                    CpTarget::DraftTo(i) => {
                                        if let Some(row) = this.group_drafts.get_mut(i) {
                                            row.to_id = id;
                                        }
                                    }
                                }
                                cx.notify();
                            });
                        }),
                );
            }
            dialog.title("选择交易对象").child(list)
        });
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
