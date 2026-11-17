//! 数据统计页
//!
//! 月份范围 + 双栏交易对象选择 → 按对象分组的逐月收入/支出/结算余额；
//! 右上角「导出交易记录」进入导出流程（后台生成 xlsx）。

use super::app::AccountApp;
use super::confirm::{self, ConfirmAction};
use super::date;
use super::state::{MonthStatView, ObjectGroupView, ObjectStatView, cp_label};
use crate::server::models::Money;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    date_picker::DatePicker,
    h_flex,
    input::Input,
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

/// 从按月对象统计构建分组视图，并倒推每月月末结算余额。
pub fn build_groups(
    all_stats: &[ObjectStatView],
    current_balances: &std::collections::HashMap<i64, Money>,
    display_to: Option<&str>,
) -> Vec<ObjectGroupView> {
    use std::collections::{BTreeMap, HashMap};

    struct Acc {
        id: i64,
        name: String,
        currency: String,
        balance: Option<Money>,
        monthly: BTreeMap<String, (Option<Money>, Option<Money>)>,
    }

    let mut all_months: Vec<String> = all_stats.iter().map(|m| m.month.clone()).collect();
    all_months.sort();
    all_months.dedup();

    let mut accs: BTreeMap<(i64, String), Acc> = BTreeMap::new();
    for month in all_stats {
        for item in &month.objects {
            let key = (item.id, item.currency.clone());
            let acc = accs.entry(key).or_insert_with(|| Acc {
                id: item.id,
                name: item.name.clone(),
                currency: item.currency.clone(),
                balance: None,
                monthly: BTreeMap::new(),
            });
            match item.direction.as_str() {
                "balance" => acc.balance = Some(item.amount.normalized()),
                "income" => {
                    let entry = acc
                        .monthly
                        .entry(month.month.clone())
                        .or_insert((None, None));
                    entry.0 = Some(match entry.0 {
                        Some(prev) => prev.checked_add(item.amount).unwrap_or(prev),
                        None => item.amount.normalized(),
                    });
                }
                "expense" => {
                    let entry = acc
                        .monthly
                        .entry(month.month.clone())
                        .or_insert((None, None));
                    entry.1 = Some(match entry.1 {
                        Some(prev) => prev.checked_add(item.amount).unwrap_or(prev),
                        None => item.amount.normalized(),
                    });
                }
                _ => {}
            }
        }
    }

    let mut groups = Vec::new();
    for (_, acc) in accs {
        let current = acc
            .balance
            .or_else(|| current_balances.get(&acc.id).copied())
            .unwrap_or(Money::ZERO)
            .normalized();

        let mut running = current;
        let mut settlements: HashMap<String, Money> = HashMap::new();
        for month in all_months.iter().rev() {
            settlements.insert(month.clone(), running);
            if let Some((income, expense)) = acc.monthly.get(month) {
                if let Some(income) = income {
                    running = running.checked_sub(*income).unwrap_or(running);
                }
                if let Some(expense) = expense {
                    running = running.checked_add(*expense).unwrap_or(running);
                }
            }
        }

        let monthly: Vec<MonthStatView> = acc
            .monthly
            .iter()
            .filter(|(month, _)| display_to.is_none_or(|to| month.as_str() <= to))
            .map(|(month, (income, expense))| MonthStatView {
                month: month.clone(),
                income: *income,
                expense: *expense,
                settlement: settlements.get(month).copied().unwrap_or(current),
            })
            .collect();
        if monthly.is_empty() {
            continue;
        }
        groups.push(ObjectGroupView {
            name: acc.name,
            currency: acc.currency,
            balance: current,
            monthly,
        });
    }
    groups.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.currency.cmp(&b.currency))
    });
    groups
}

impl AccountApp {
    /// 数据统计页。
    pub fn stats_page(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let currencies = self.currencies.clone();
        let cps = self.counterparties.clone();
        let selected = self.stats_cp.clone();
        let stats_from = date::picker_month(self.stats_from.read(cx));
        let stats_to = date::picker_month(self.stats_to.read(cx));
        let query = self.stats_search.read(cx).value().trim().to_lowercase();

        let labels: Vec<(i64, String)> = cps
            .iter()
            .map(|c| (c.id, cp_label(c, &currencies)))
            .collect();
        let filtered: Vec<(i64, String)> = labels
            .iter()
            .filter(|(_, label)| query.is_empty() || label.to_lowercase().contains(&query))
            .cloned()
            .collect();
        let selected_pairs: Vec<(i64, String)> = selected
            .iter()
            .map(|id| {
                labels
                    .iter()
                    .find(|(cid, _)| cid == id)
                    .cloned()
                    .unwrap_or((*id, format!("#{id}")))
            })
            .collect();

        let mut balances = std::collections::HashMap::new();
        for c in &cps {
            balances.insert(c.id, c.balance);
        }
        let groups = build_groups(&self.stats, &balances, stats_to.as_deref());

        let query_button = {
            let entity = entity.clone();
            Button::new("query-stats")
                .primary()
                .label("🔍 查对象统计")
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.refresh_stats(cx));
                })
        };
        let export_button = {
            let entity = entity.clone();
            let names = selected_pairs
                .iter()
                .map(|(_, label)| format!("· {label}"))
                .collect::<Vec<_>>()
                .join("\n");
            let scope = format!(
                "将导出以下交易对象的交易记录：\n{names}\n\n时间区间：{} ~ {}\n文件格式：.xlsx（每个交易对象一个 sheet）\n\n确认后选择保存位置。",
                stats_from.as_deref().unwrap_or("不限"),
                stats_to.as_deref().unwrap_or("不限"),
            );
            Button::new("export-records")
                .success()
                .disabled(selected.is_empty())
                .label("📤 导出交易记录")
                .on_click(move |_, window, cx| {
                    confirm::confirm(
                        window,
                        cx,
                        entity.clone(),
                        "确认导出范围",
                        scope.clone(),
                        ConfirmAction::Export,
                    );
                })
        };

        let left_rows: Vec<gpui_kit::AnyElement> = filtered
            .iter()
            .map(|(id, label)| {
                let entity = entity.clone();
                let id = *id;
                h_flex()
                    .justify_between()
                    .px_2()
                    .py_1()
                    .child(label.clone())
                    .child(
                        Button::new(SharedString::from(format!("stats-add-{id}")))
                            .small()
                            .label("添加")
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    if !this.stats_cp.contains(&id) {
                                        this.stats_cp.push(id);
                                    }
                                    cx.notify();
                                });
                            }),
                    )
                    .into_any_element()
            })
            .collect();
        let right_rows: Vec<gpui_kit::AnyElement> = selected_pairs
            .iter()
            .map(|(id, label)| {
                let entity = entity.clone();
                let id = *id;
                h_flex()
                    .justify_between()
                    .px_2()
                    .py_1()
                    .child(label.clone())
                    .child(
                        Button::new(SharedString::from(format!("stats-remove-{id}")))
                            .danger()
                            .small()
                            .label("踢出")
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.stats_cp.retain(|x| *x != id);
                                    cx.notify();
                                });
                            }),
                    )
                    .into_any_element()
            })
            .collect();

        let group_cards: Vec<gpui_kit::AnyElement> =
            groups
                .iter()
                .map(|g| {
                    let rows: Vec<gpui_kit::AnyElement> =
                        g.monthly
                            .iter()
                            .map(|row| {
                                h_flex()
                                    .gap_3()
                                    .items_center()
                                    .py_1()
                                    .text_sm()
                                    .child(div().w(px(76.)).child(row.month.clone()))
                                    .child(if let Some(inc) = row.income {
                                        div()
                                            .text_color(cx.theme().success)
                                            .child(format!("收入 {inc}"))
                                            .into_any_element()
                                    } else {
                                        div().into_any_element()
                                    })
                                    .child(if let Some(exp) = row.expense {
                                        div()
                                            .text_color(gpui_kit::rgb(0xC0392B))
                                            .child(format!("支出 {exp}"))
                                            .into_any_element()
                                    } else {
                                        div().into_any_element()
                                    })
                                    .child(div().ml_auto().text_color(cx.theme().primary).child(
                                        format!("结算余额 {} {}", row.settlement, g.currency),
                                    ))
                                    .into_any_element()
                            })
                            .collect();
                    v_flex()
                        .gap_1()
                        .p_2()
                        .border_1()
                        .border_color(cx.theme().border)
                        .rounded(px(6.))
                        .child(
                            h_flex()
                                .justify_between()
                                .child(format!("{} ({})", g.name, g.currency))
                                .child(
                                    div()
                                        .text_color(cx.theme().primary)
                                        .child(format!("当前余额 {}", g.balance)),
                                ),
                        )
                        .children(rows)
                        .into_any_element()
                })
                .collect();

        v_flex()
            .w_full()
            .items_start()
            .justify_start()
            .gap_3()
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .child(div().text_xl().child("数据统计"))
                    .child(export_button),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child("月份:")
                    .child(
                        div()
                            .w(px(170.))
                            .child(DatePicker::new(&self.stats_from).placeholder("起始月份")),
                    )
                    .child("至")
                    .child(
                        div()
                            .w(px(170.))
                            .child(DatePicker::new(&self.stats_to).placeholder("结束月份")),
                    )
                    .child(query_button),
            )
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded(px(6.))
                            .p_2()
                            .child(format!("交易对象列表 ({} / {})", filtered.len(), cps.len()))
                            .child(Input::new(&self.stats_search))
                            .child(
                                v_flex()
                                    .id("stats-left-list")
                                    .role(gpui_kit::Role::List)
                                    .gap_1()
                                    .max_h(px(320.))
                                    .overflow_y_scrollbar()
                                    .children(left_rows),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded(px(6.))
                            .p_2()
                            .child(format!("已选择的查询目标 ({})", selected.len()))
                            .child(
                                v_flex()
                                    .id("stats-right-list")
                                    .role(gpui_kit::Role::List)
                                    .gap_1()
                                    .max_h(px(320.))
                                    .overflow_y_scrollbar()
                                    .children(right_rows),
                            ),
                    ),
            )
            .children(group_cards)
            .child(if self.stats.is_empty() {
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child("选择交易对象与月份范围，点击「查对象统计」")
                    .into_any_element()
            } else if groups.is_empty() {
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child("所选月份无匹配的收支记录")
                    .into_any_element()
            } else {
                div().into_any_element()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::models::Money;

    fn item(
        id: i64,
        direction: &str,
        value: i128,
        divisor: i128,
    ) -> crate::ui::state::ObjectStatItem {
        crate::ui::state::ObjectStatItem {
            id,
            name: "钱包".into(),
            direction: direction.into(),
            currency: "RMB".into(),
            amount: Money::new(value, divisor).unwrap(),
        }
    }

    /// 验证逐月结算余额：当前余额 70，7 月收入 100、8 月支出 30，
    /// 则 7 月末余额 100、8 月末余额 70（从当前余额向前倒推）。
    #[test]
    fn settlement_balance_walks_backwards_from_current() {
        let stats = vec![
            ObjectStatView {
                month: "2026-07".into(),
                objects: vec![item(1, "income", 100, 1), item(1, "balance", 70, 1)],
            },
            ObjectStatView {
                month: "2026-08".into(),
                objects: vec![item(1, "expense", 30, 1)],
            },
        ];
        let groups = build_groups(&stats, &std::collections::HashMap::new(), None);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].balance, Money::from(70));
        assert_eq!(groups[0].monthly.len(), 2);
        assert_eq!(groups[0].monthly[0].settlement, Money::from(100));
        assert_eq!(groups[0].monthly[1].settlement, Money::from(70));
        // 不相等断言：结算余额不能两月相同
        assert_ne!(
            groups[0].monthly[0].settlement,
            groups[0].monthly[1].settlement
        );
    }

    /// 验证展示结束月份过滤：只影响展示，不影响倒推结算余额。
    #[test]
    fn display_to_filters_months_but_keeps_settlement() {
        let stats = vec![
            ObjectStatView {
                month: "2026-07".into(),
                objects: vec![item(1, "income", 100, 1), item(1, "balance", 70, 1)],
            },
            ObjectStatView {
                month: "2026-08".into(),
                objects: vec![item(1, "expense", 30, 1)],
            },
        ];
        let groups = build_groups(&stats, &std::collections::HashMap::new(), Some("2026-07"));
        assert_eq!(groups[0].monthly.len(), 1);
        assert_eq!(groups[0].monthly[0].month, "2026-07");
        assert_eq!(groups[0].monthly[0].settlement, Money::from(100));
        // 不相等断言：8 月必须被过滤掉
        assert_ne!(groups[0].monthly[0].month, "2026-08");
    }
}
