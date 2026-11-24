//! 数据统计页
//!
//! 月份范围 + 双栏交易对象选择 → 按对象生成横向卡片（逐月收入/支出/环比/同比/结算余额）、
//! 桑基图/流向图，以及导出入口（xlsx 生成在 `ui/export.rs`）。
//!
//! 环比/同比需要「上月/去年同月」作为基准，因此 `refresh_stats` 会多取一年数据，
//! 本模块的 `build_groups` 再按用户选择的起始月份过滤展示。

use super::app::AccountApp;
use super::app_state::{
    MonthCardRow, ObjectGroupView, ObjectStatView, PAGE_MIN_WIDTH, cp_label, growth_percent,
    month_range, shift_month,
};
use super::confirm;
use super::date;
use super::sankey;
use crate::server::export::RatioValue;
use crate::server::models::{Counterparty, Money};
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
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

/// 从按月对象统计构建卡片视图。
///
/// - `display_from` / `display_to`：用户选择的月份范围；范围内即使没有交易也生成一行
///   （收入/支出记为 0，结算余额保持不变）。
/// - `data_from` / `data_to`：数据库实际有记录的月份范围（账号开始记账 ~ 最后有数据的月份）。
///   环比/同比的基准月份只有落在这个范围内才会计算；范围外（例如开始记账之前）置空。
pub fn build_groups(
    all_stats: &[ObjectStatView],
    current_balances: &std::collections::HashMap<i64, Money>,
    display_from: Option<&str>,
    display_to: Option<&str>,
    data_from: Option<&str>,
    data_to: Option<&str>,
) -> Vec<ObjectGroupView> {
    use std::collections::{BTreeMap, HashMap};

    struct Acc {
        id: i64,
        name: String,
        currency: String,
        balance: Option<Money>,
        monthly: BTreeMap<String, (Option<Money>, Option<Money>)>,
    }

    let mut data_months: Vec<String> = all_stats.iter().map(|m| m.month.clone()).collect();
    data_months.sort();
    data_months.dedup();

    // 数据库记录范围：优先用调用方给出的账号起始月份，其次用数据首尾月份兜底
    let range_start = data_from
        .map(str::to_string)
        .or_else(|| data_months.first().cloned());
    let range_end = data_to
        .map(str::to_string)
        .or_else(|| data_months.last().cloned());

    // 展示月份：用户选了范围就按连续月份补零；缺少一侧时用数据范围兜底
    let display_months: Vec<String> = match (
        display_from.or(range_start.as_deref()),
        display_to.or(range_end.as_deref()),
    ) {
        (Some(from), Some(to)) => {
            let months = month_range(from, to);
            if months.is_empty() {
                data_months.clone()
            } else {
                months
            }
        }
        _ => data_months.clone(),
    };
    if display_months.is_empty() {
        return Vec::new();
    }

    // 结算余额需要覆盖「展示范围 + 数据范围」的连续月份：缺失月份余额不变
    let first_needed = display_months
        .first()
        .cloned()
        .into_iter()
        .chain(data_months.first().cloned())
        .min();
    let last_needed = display_months
        .last()
        .cloned()
        .into_iter()
        .chain(data_months.last().cloned())
        .max();
    let settlement_months = match (first_needed, last_needed) {
        (Some(from), Some(to)) => month_range(&from, &to),
        _ => Vec::new(),
    };

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

    let in_data_range = |month: &str| {
        range_start.as_deref().is_some_and(|from| month >= from)
            && range_end.as_deref().is_some_and(|to| month <= to)
    };
    let value_of = |acc: &Acc, month: &str, income: bool| -> Money {
        acc.monthly
            .get(month)
            .map(|(i, e)| {
                if income {
                    i.unwrap_or(Money::ZERO)
                } else {
                    e.unwrap_or(Money::ZERO)
                }
            })
            .unwrap_or(Money::ZERO)
    };

    let mut groups = Vec::new();
    for (_, acc) in accs {
        let current = acc
            .balance
            .or_else(|| current_balances.get(&acc.id).copied())
            .unwrap_or(Money::ZERO)
            .normalized();

        // 从当前余额向前连续倒推（覆盖没有交易的月份）
        let mut running = current;
        let mut settlements: HashMap<String, Money> = HashMap::new();
        for month in settlement_months.iter().rev() {
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

        let monthly: Vec<MonthCardRow> = display_months
            .iter()
            .map(|month| {
                let income = value_of(&acc, month, true);
                let expense = value_of(&acc, month, false);
                let ratio = |base: Option<String>, income: bool| -> RatioValue {
                    let Some(base) = base else {
                        return RatioValue::OutOfRange;
                    };
                    // 基准月份不在数据库记录范围内 → 留空
                    if !in_data_range(&base) {
                        return RatioValue::OutOfRange;
                    }
                    let previous = value_of(&acc, &base, income);
                    // 基准月份在范围内但数值为 0 → 无法计算增长率，显示 N.A.
                    if previous.is_zero() {
                        return RatioValue::NotApplicable;
                    }
                    growth_percent(value_of(&acc, month, income), previous)
                        .map(RatioValue::Value)
                        .unwrap_or(RatioValue::NotApplicable)
                };
                MonthCardRow {
                    month: month.clone(),
                    income: Some(income),
                    income_mom: ratio(shift_month(month, -1), true),
                    income_yoy: ratio(shift_month(month, -12), true),
                    expense: Some(expense),
                    expense_mom: ratio(shift_month(month, -1), false),
                    expense_yoy: ratio(shift_month(month, -12), false),
                    settlement: settlements.get(month).copied().unwrap_or(current),
                }
            })
            .collect();
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

/// 计算数据库记录范围（用于判断环比/同比基准是否可用）。
///
/// - 起始月份：账号最早创建月份与统计数据最早月份中的较小者（账号开始记账的时间）；
/// - 结束月份：统计数据中最晚的月份（最后有记录的月份）。
pub fn data_range(
    stats: &[ObjectStatView],
    counterparties: &[Counterparty],
) -> (Option<String>, Option<String>) {
    let stat_min = stats.iter().map(|m| m.month.as_str()).min();
    let stat_max = stats.iter().map(|m| m.month.as_str()).max();
    let created_min = counterparties
        .iter()
        .map(|c| c.created_month.as_str())
        .filter(|m| !m.is_empty())
        .min();
    let start = match (stat_min, created_min) {
        (Some(a), Some(b)) => Some(a.min(b).to_string()),
        (Some(a), None) => Some(a.to_string()),
        (None, Some(b)) => Some(b.to_string()),
        (None, None) => None,
    };
    (start, stat_max.map(str::to_string))
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
        let (data_start, data_end) = data_range(&self.stats, &cps);
        let groups = build_groups(
            &self.stats,
            &balances,
            stats_from.as_deref(),
            stats_to.as_deref(),
            data_start.as_deref(),
            data_end.as_deref(),
        );
        let group_label = selected_pairs
            .iter()
            .map(|(_, label)| label.clone())
            .collect::<Vec<_>>()
            .join("、");

        let query_button = {
            let entity = entity.clone();
            Button::new("query-stats")
                .primary()
                .label("🔍 查询")
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
                .label("📤 导出")
                .on_click(move |_, window, cx| {
                    confirm::confirm_export(window, cx, entity.clone(), scope.clone());
                })
        };
        let draw_button = {
            let entity = entity.clone();
            Button::new("draw-sankey")
                .disabled(selected.is_empty())
                .label("📈 画图")
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.refresh_flows(cx));
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
                                        // 查询目标变化后旧流向图不再成立
                                        this.flows.clear();
                                        this.show_sankey = false;
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
                                    this.flows.clear();
                                    this.show_sankey = false;
                                    cx.notify();
                                });
                            }),
                    )
                    .into_any_element()
            })
            .collect();

        // 卡片列：月份 / 收入 / 收入环比 / 收入同比 / 支出 / 支出环比 / 支出同比 / 结算余额
        const COLS: [&str; 8] = [
            "月份",
            "收入",
            "收入环比",
            "收入同比",
            "支出",
            "支出环比",
            "支出同比",
            "结算余额",
        ];
        const WIDTHS: [f32; 8] = [76., 92., 88., 88., 92., 88., 88., 128.];
        let income_color = cx.theme().success;
        let expense_color = gpui_kit::rgb(0xC0392B);
        let primary_color = cx.theme().primary;
        let muted_color = cx.theme().muted_foreground;
        let group_cards: Vec<gpui_kit::AnyElement> = groups
            .iter()
            .map(|g| {
                let header = h_flex().gap_2().px_2().child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .text_sm()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .children(
                            COLS.iter()
                                .zip(WIDTHS)
                                .map(|(title, w)| div().w(px(w)).child(*title)),
                        ),
                );
                let rows: Vec<gpui_kit::AnyElement> = g
                    .monthly
                    .iter()
                    .enumerate()
                    .map(|(i, row)| {
                        let bg = if i % 2 == 1 {
                            cx.theme().muted
                        } else {
                            cx.theme().background
                        };
                        let text =
                            |value: Option<Money>| value.map(|m| m.to_string()).unwrap_or_default();
                        let pct = |value: RatioValue| value.display();
                        h_flex()
                            .gap_2()
                            .items_center()
                            .px_2()
                            .py_1()
                            .text_sm()
                            .bg(bg)
                            .child(div().w(px(WIDTHS[0])).child(row.month.clone()))
                            .child(
                                div()
                                    .w(px(WIDTHS[1]))
                                    .text_color(income_color)
                                    .child(text(row.income)),
                            )
                            .child(
                                div()
                                    .w(px(WIDTHS[2]))
                                    .text_color(muted_color)
                                    .child(pct(row.income_mom)),
                            )
                            .child(
                                div()
                                    .w(px(WIDTHS[3]))
                                    .text_color(muted_color)
                                    .child(pct(row.income_yoy)),
                            )
                            .child(
                                div()
                                    .w(px(WIDTHS[4]))
                                    .text_color(expense_color)
                                    .child(text(row.expense)),
                            )
                            .child(
                                div()
                                    .w(px(WIDTHS[5]))
                                    .text_color(muted_color)
                                    .child(pct(row.expense_mom)),
                            )
                            .child(
                                div()
                                    .w(px(WIDTHS[6]))
                                    .text_color(muted_color)
                                    .child(pct(row.expense_yoy)),
                            )
                            .child(
                                div()
                                    .w(px(WIDTHS[7]))
                                    .text_color(primary_color)
                                    .child(format!("{} {}", row.settlement, g.currency)),
                            )
                            .into_any_element()
                    })
                    .collect();
                v_flex()
                    .flex_shrink_0()
                    // 列宽 + 7 个列间距 + 行内 padding + 卡片 padding
                    .w(px(WIDTHS.iter().sum::<f32>() + 88.))
                    .gap_1()
                    .p_2()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(px(6.))
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .child(format!("{} ({})", g.name, g.currency)),
                            )
                            .child(
                                div()
                                    .text_color(primary_color)
                                    .child(format!("当前余额 {}", g.balance)),
                            ),
                    )
                    .child(header)
                    .children(rows)
                    .into_any_element()
            })
            .collect();

        v_flex()
            .w_full()
            .min_w(px(PAGE_MIN_WIDTH))
            .items_start()
            .justify_start()
            .gap_3()
            .child(div().text_xl().child("数据统计"))
            // 查询 / 导出 / 画图：单独一行，位于标题与月份选择之间
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(query_button)
                    .child(export_button)
                    .child(draw_button),
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
                    ),
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
                                    .role(Role::List)
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
                                    .role(Role::List)
                                    .gap_1()
                                    .max_h(px(320.))
                                    .overflow_y_scrollbar()
                                    .children(right_rows),
                            ),
                    ),
            )
            .child(if groups.is_empty() {
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(if self.stats.is_empty() {
                        "选择交易对象与月份范围，点击「查询」"
                    } else {
                        "所选月份无匹配的收支记录"
                    })
                    .into_any_element()
            } else {
                // 卡片横向排列：便于下方桑基图获得完整宽度
                h_flex()
                    .id("stats-cards")
                    .role(Role::List)
                    .w_full()
                    .gap_3()
                    .items_start()
                    .pb_2()
                    .overflow_x_scrollbar()
                    .children(group_cards)
                    .into_any_element()
            })
            .child(if !self.show_sankey {
                div().into_any_element()
            } else if self.flows.is_empty() {
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child("所选范围内没有可绘制的流向")
                    .into_any_element()
            } else {
                sankey::sankey_view(&self.flows, &group_label, cx).into_any_element()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::export::RatioValue;
    use crate::server::models::Money;

    fn item(
        id: i64,
        direction: &str,
        value: i128,
        divisor: i128,
    ) -> crate::ui::app_state::ObjectStatItem {
        crate::ui::app_state::ObjectStatItem {
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
        let groups = build_groups(
            &stats,
            &std::collections::HashMap::new(),
            None,
            None,
            None,
            None,
        );
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

    /// 验证展示月份过滤：只影响展示，不影响倒推结算余额。
    #[test]
    fn display_range_filters_months_but_keeps_settlement() {
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
        let groups = build_groups(
            &stats,
            &std::collections::HashMap::new(),
            Some("2026-07"),
            Some("2026-07"),
            None,
            None,
        );
        assert_eq!(groups[0].monthly.len(), 1);
        assert_eq!(groups[0].monthly[0].month, "2026-07");
        assert_eq!(groups[0].monthly[0].settlement, Money::from(100));
        // 不相等断言：8 月必须被过滤掉
        assert_ne!(groups[0].monthly[0].month, "2026-08");
    }

    /// 验证环比/同比：以上月、去年同月为基准；只有基准月份不在数据库范围内才置空。
    #[test]
    fn mom_and_yoy_should_use_previous_month_and_last_year() {
        let stats = vec![
            ObjectStatView {
                month: "2025-08".into(),
                objects: vec![item(1, "income", 50, 1), item(1, "expense", 10, 1)],
            },
            ObjectStatView {
                month: "2026-07".into(),
                objects: vec![item(1, "income", 100, 1), item(1, "expense", 20, 1)],
            },
            ObjectStatView {
                month: "2026-08".into(),
                objects: vec![
                    item(1, "income", 150, 1),
                    item(1, "expense", 30, 1),
                    item(1, "balance", 70, 1),
                ],
            },
        ];
        let groups = build_groups(
            &stats,
            &std::collections::HashMap::new(),
            Some("2026-07"),
            None,
            None,
            None,
        );
        let rows = &groups[0].monthly;
        // 2025-08 被过滤；2026-07 的上月 2026-06 在数据库范围内但没有数据 → N.A.
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].month, "2026-07");
        assert_eq!(rows[0].income_mom, RatioValue::NotApplicable);
        assert_eq!(rows[0].expense_mom, RatioValue::NotApplicable);
        // 去年同月 2025-07 在数据库范围之前 → 留空
        assert_eq!(rows[0].income_yoy, RatioValue::OutOfRange);
        assert_eq!(rows[0].expense_yoy, RatioValue::OutOfRange);

        // 2026-08：收入 150 对比上月 100（+50%）、去年同月 50（+200%）
        assert_eq!(rows[1].month, "2026-08");
        assert_eq!(rows[1].income_mom.amount().unwrap().to_string(), "50");
        assert_eq!(rows[1].income_yoy.amount().unwrap().to_string(), "200");
        // 支出 30 对比上月 20（+50%）、去年同月 10（+200%）
        assert_eq!(rows[1].expense_mom.amount().unwrap().to_string(), "50");
        assert_eq!(rows[1].expense_yoy.amount().unwrap().to_string(), "200");
        // 不相等断言：环比不能误用同比基准
        assert_ne!(rows[1].income_mom, rows[1].income_yoy);
    }

    /// 验证「范围内没有交易的月份记为 0」以及「基准月份在范围内就计算环比」：
    /// 账号 2027-10 开始记账，2027-11 有环比、没有同比；2027-12 无数据也显示 0。
    #[test]
    fn missing_months_should_be_zero_and_ratios_follow_data_range() {
        let stats = vec![
            ObjectStatView {
                month: "2027-10".into(),
                objects: vec![item(1, "income", 100, 1)],
            },
            ObjectStatView {
                month: "2027-11".into(),
                objects: vec![item(1, "income", 150, 1), item(1, "balance", 70, 1)],
            },
        ];
        let groups = build_groups(
            &stats,
            &std::collections::HashMap::new(),
            Some("2027-10"),
            Some("2027-12"),
            Some("2027-10"),
            Some("2027-11"),
        );
        let rows = &groups[0].monthly;
        assert_eq!(rows.len(), 3);

        // 2027-10：账号起始月，上月/去年同月都在范围外 → 留空
        assert_eq!(rows[0].month, "2027-10");
        assert_eq!(rows[0].income, Some(Money::from(100)));
        assert_eq!(rows[0].expense, Some(Money::ZERO));
        assert_eq!(rows[0].income_mom, RatioValue::OutOfRange);
        assert_eq!(rows[0].income_yoy, RatioValue::OutOfRange);
        assert_eq!(rows[0].settlement, Money::from(-80));

        // 2027-11：上月 2027-10 在范围内 → 有环比；去年 2026-11 在范围外 → 无同比
        assert_eq!(rows[1].month, "2027-11");
        assert_eq!(rows[1].income_mom.amount().unwrap().to_string(), "50");
        assert_eq!(rows[1].income_yoy, RatioValue::OutOfRange);
        assert_eq!(rows[1].expense, Some(Money::ZERO));
        // 支出基准为 0 → 显示 N.A.
        assert_eq!(rows[1].expense_mom, RatioValue::NotApplicable);
        assert_eq!(rows[1].expense_mom.display(), "N.A.");
        assert_eq!(rows[1].settlement, Money::from(70));

        // 2027-12：没有交易也显示 0；上月 2027-11 在范围内 → 收入下降 100%
        assert_eq!(rows[2].month, "2027-12");
        assert_eq!(rows[2].income, Some(Money::ZERO));
        assert_eq!(rows[2].expense, Some(Money::ZERO));
        assert_eq!(rows[2].income_mom.amount().unwrap().to_string(), "-100");
        assert_eq!(rows[2].income_yoy, RatioValue::OutOfRange);
        assert_eq!(rows[2].settlement, Money::from(70));
        // 不相等断言：起始月不能凭空出现同比
        assert_ne!(rows[0].income_yoy, rows[1].income_mom);
    }

    /// 验证数据库记录范围：取「账号创建月份」与「统计月份」的并集边界。
    #[test]
    fn data_range_should_use_created_month_and_stats() {
        let cps = vec![Counterparty {
            id: 1,
            name: "钱包".into(),
            currency_id: 1,
            balance: Money::ZERO,
            created_month: "2027-10".into(),
        }];
        let stats = vec![ObjectStatView {
            month: "2027-11".into(),
            objects: vec![],
        }];
        assert_eq!(
            data_range(&stats, &cps),
            (Some("2027-10".to_string()), Some("2027-11".to_string()))
        );
        // 不相等断言：结束月份不能误取创建月份
        assert_ne!(data_range(&stats, &cps).1, Some("2027-10".to_string()));
    }
}
