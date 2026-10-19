//! 第三页：数据统计
//!
//! 对象统计：月份范围（`input type="month"`）+ 双栏交易对象选择
//! （左侧可搜索列表点「添加」，右侧为已选列表可「踢出」，添加自动去重），
//! 按**交易对象分组**展示：每组先显示一次当前余额（蓝），
//! 再列出所选月份范围内各月的收入（绿）/支出（红）/结算余额（蓝）。
//!
//! 结算余额由**前端 UI 计算**：从当前余额出发，按月份从新到旧倒推，
//! 某月月末结算余额 = 当前余额 - 该月之后所有月份的净流入（收入 - 支出）。
//! 因此查询时 `to` 固定传空，确保拿到所选起始月之后的全部月份；
//! 展示时再按用户选择的结束月份过滤。

use super::{
    GlobalState, MonthStatView, ObjectGroupView, ObjectStatView, cp_label, http_put, push_toast,
};
use crate::models::{Counterparty, Currency, Money};
use dioxus::prelude::*;

/// 从接口返回的按月对象统计构建分组视图，并计算每月月末结算余额。
///
/// - `current_balances`：交易对象当前余额（接口未返回 `balance` 条目时兜底）
/// - `display_to`：展示结束月份（含）；为 None 表示展示全部已返回月份。
///   注意：结算余额的倒推始终基于 `all_stats` 的全部月份，不受该过滤影响。
fn build_groups(
    all_stats: &[ObjectStatView],
    current_balances: &std::collections::HashMap<i64, Money>,
    display_to: Option<&str>,
) -> Vec<ObjectGroupView> {
    use std::collections::{BTreeMap, HashMap};

    /// 单个交易对象的累计容器
    struct Acc {
        id: i64,
        name: String,
        currency: String,
        balance: Option<Money>,
        /// 月份 → (收入, 支出)
        monthly: BTreeMap<String, (Option<Money>, Option<Money>)>,
    }

    // 所有返回月份（升序去重），倒推结算余额时需要按时间从新到旧遍历
    let mut all_months: Vec<String> = all_stats.iter().map(|m| m.month.clone()).collect();
    all_months.sort();
    all_months.dedup();

    // 按 (对象 ID, 货币名) 聚合
    let mut accs: BTreeMap<(i64, String), Acc> = BTreeMap::new();
    for m in all_stats {
        for item in &m.objects {
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
                    let entry = acc.monthly.entry(m.month.clone()).or_insert((None, None));
                    entry.0 = Some(match entry.0 {
                        Some(prev) => prev.checked_add(item.amount).unwrap_or(prev),
                        None => item.amount.normalized(),
                    });
                }
                "expense" => {
                    let entry = acc.monthly.entry(m.month.clone()).or_insert((None, None));
                    entry.1 = Some(match entry.1 {
                        Some(prev) => prev.checked_add(item.amount).unwrap_or(prev),
                        None => item.amount.normalized(),
                    });
                }
                _ => {}
            }
        }
    }

    let mut groups: Vec<ObjectGroupView> = Vec::new();
    for (_, acc) in accs {
        let current = acc
            .balance
            .or_else(|| current_balances.get(&acc.id).copied())
            .unwrap_or_else(|| Money::from_int(0))
            .normalized();

        // 从最新月份向前倒推：先记录当前月月末余额，再减去该月净流入得到上月末余额
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
        // 所选范围内没有该对象的记录时不展示空卡片
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

#[component]
pub(crate) fn StatsPage() -> Element {
    let mut st = use_context::<GlobalState>();
    let currencies: Vec<Currency> = (st.currencies)().clone();
    let counterparties: Vec<Counterparty> = (st.counterparties)().clone();
    let all_stats: Vec<ObjectStatView> = (st.stats)().clone();
    let selected: Vec<i64> = (st.stats_cp)().clone();
    let stats_from = (st.stats_from)();
    let stats_to = (st.stats_to)();
    // 左侧交易对象搜索词（空 → 显示全部）
    let mut cp_search = use_signal(String::new);

    // 左侧可搜索列表：按「名称 (货币)」子串匹配，忽略大小写
    let q = (cp_search)().trim().to_lowercase();
    let filtered_cps: Vec<(i64, String)> = counterparties
        .iter()
        .map(|c| (c.id, cp_label(c, &currencies)))
        .filter(|(_, label)| q.is_empty() || label.to_lowercase().contains(&q))
        .collect();

    // 右侧已选列表（保留添加顺序）
    let selected_pairs: Vec<(i64, String)> = selected
        .iter()
        .map(|id| {
            let label = counterparties
                .iter()
                .find(|c| c.id == *id)
                .map(|c| cp_label(c, &currencies))
                .unwrap_or_else(|| format!("#{id}"));
            (*id, label)
        })
        .collect();

    // 当前余额映射：用于接口未返回 balance 条目时的兜底
    let mut current_balances: std::collections::HashMap<i64, Money> =
        std::collections::HashMap::new();
    for c in &counterparties {
        current_balances.insert(c.id, c.balance);
    }

    let display_to = {
        let t = stats_to.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    };
    let groups = build_groups(&all_stats, &current_balances, display_to.as_deref());
    let has_stats = !all_stats.is_empty();

    rsx! {
        div { style: "display:flex;flex-direction:column;gap:16px",
            h3 { "数据统计" }

            div { style: "border:1px solid #eee;padding:8px;border-radius:4px",
                strong { "对象统计" }
                div { style: "display:flex;gap:8px;margin-top:6px;flex-wrap:wrap;align-items:center",
                    label { "月份:" }
                    input {
                        r#type: "month",
                        value: stats_from,
                        oninput: move |e| st.stats_from.set(e.value()),
                    }
                    span { "至" }
                    input {
                        r#type: "month",
                        value: stats_to,
                        oninput: move |e| st.stats_to.set(e.value()),
                    }
                }

                // 双栏：左「可搜索的交易对象列表」，右「已选择的查询目标」
                div { style: "display:flex;gap:12px;margin-top:8px;align-items:stretch",
                    // 左栏
                    div {
                        style: "flex:1;min-width:0;border:1px solid #ddd;border-radius:4px;display:flex;flex-direction:column",
                        div { style: "padding:6px 8px;border-bottom:1px solid #eee",
                            div { style: "display:flex;align-items:center;justify-content:space-between;gap:8px",
                                strong { "交易对象列表" }
                                span { style: "font-size:12px;color:#888", "{filtered_cps.len()} / {counterparties.len()}" }
                            }
                            input {
                                style: "width:100%;margin-top:6px",
                                value: cp_search,
                                placeholder: "搜索交易对象（名称或货币）",
                                oninput: move |e| cp_search.set(e.value()),
                            }
                        }
                        div { style: "max-height:280px;overflow:auto",
                            for (idx, (cp_id, cp_text)) in filtered_cps.iter().cloned().enumerate() {
                                div {
                                    style: format!(
                                        "display:flex;align-items:center;gap:8px;padding:5px 8px;border-bottom:1px solid #eee;background:{}",
                                        if idx % 2 == 1 { "#f5f6f8" } else { "#fff" }
                                    ),
                                    span {
                                        style: "flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap",
                                        "{cp_text}"
                                    }
                                    button {
                                        style: "flex-shrink:0",
                                        // 已在右侧也保留该条目且按钮可继续点击；添加时去重
                                        onclick: move |_| {
                                            let mut sel = st.stats_cp.write();
                                            if !sel.contains(&cp_id) {
                                                sel.push(cp_id);
                                            }
                                        },
                                        "添加"
                                    }
                                }
                            }
                            if filtered_cps.is_empty() {
                                div { style: "padding:8px;color:#999",
                                    if counterparties.is_empty() { "暂无交易对象" } else { "无匹配交易对象" }
                                }
                            }
                        }
                    }

                    // 右栏
                    div {
                        style: "flex:1;min-width:0;border:1px solid #ddd;border-radius:4px;display:flex;flex-direction:column",
                        div { style: "padding:6px 8px;border-bottom:1px solid #eee;display:flex;align-items:center;justify-content:space-between;gap:8px",
                            strong { "已选择的交易对象" }
                            span { style: "font-size:12px;color:#888", "{selected_pairs.len()} 项" }
                        }
                        div { style: "max-height:312px;overflow:auto",
                            for (idx, (cp_id, cp_text)) in selected_pairs.iter().cloned().enumerate() {
                                div {
                                    style: format!(
                                        "display:flex;align-items:center;gap:8px;padding:5px 8px;border-bottom:1px solid #eee;background:{}",
                                        if idx % 2 == 1 { "#f5f6f8" } else { "#fff" }
                                    ),
                                    span {
                                        style: "flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap",
                                        "{cp_text}"
                                    }
                                    button {
                                        style: "flex-shrink:0;color:#c0392b",
                                        onclick: move |_| {
                                            st.stats_cp.write().retain(|x| *x != cp_id);
                                        },
                                        "✕ 踢出"
                                    }
                                }
                            }
                            if selected_pairs.is_empty() {
                                div { style: "padding:8px;color:#999", "尚未选择交易对象，请从左侧添加" }
                            }
                        }
                    }
                }

                div { style: "display:flex;gap:8px;margin-top:8px;align-items:center;flex-wrap:wrap",
                    button {
                        onclick: move |_| {
                            let mut st = st;
                            spawn(async move {
                                let ids = (st.stats_cp)();
                                if ids.is_empty() {
                                    push_toast(&st, "请至少选择一个交易对象");
                                    return;
                                }
                                let from = (st.stats_from)().trim().to_string();
                                let to = (st.stats_to)().trim().to_string();
                                if !from.is_empty() && !to.is_empty() && from > to {
                                    push_toast(&st, "起始月份不能晚于结束月份");
                                    return;
                                }
                                let b = (st.base)();
                                // 结束月份不从后端截断：前端需要 from 之后的所有月份，
                                // 才能从当前余额准确倒推每个月的月末结算余额
                                let body = serde_json::json!({
                                    "counterparty_ids": ids,
                                    "from": if from.is_empty() { None } else { Some(from) },
                                    "to": serde_json::Value::Null,
                                });
                                match http_put(&b, "/api/stats/objects", Some(body)).await {
                                    Ok(v) => st.stats.set(serde_json::from_value(v).unwrap_or_default()),
                                    Err(e) => push_toast(&st, &format!("查询失败: {e}")),
                                }
                            });
                        },
                        "🔍 查对象统计"
                    }
                    span { style: "font-size:12px;color:#888", "结算余额按所选月份逐月显示（由当前余额向前倒推）" }
                }
            }

            h4 { "统计结果" }
            for g in groups.iter() {
                div { style: "border:1px solid #eee;padding:8px;border-radius:4px;margin-bottom:8px",
                    div { style: "display:flex;justify-content:space-between;align-items:baseline;gap:8px;flex-wrap:wrap",
                        strong { "{g.name} ({g.currency})" }
                        span { style: "color:#2980b9;font-size:13px", "当前余额 {g.balance.to_decimal_string()} {g.currency}" }
                    }
                    for row in g.monthly.iter() {
                        div { style: "display:flex;align-items:center;gap:12px;padding:4px 0;border-top:1px dashed #eee;font-size:13px;flex-wrap:wrap",
                            span { style: "width:76px;color:#555", "{row.month}" }
                            if let Some(inc) = row.income {
                                span { style: "color:#1b7e3f", "收入 {inc.to_decimal_string()} " }
                            }
                            if let Some(exp) = row.expense {
                                span { style: "color:#c0392b", "支出 {exp.to_decimal_string()}" }
                            }
                            if row.income.is_none() && row.expense.is_none() {
                                span { style: "color:#999", "无记录" }
                            }
                            span { style: "margin-left:auto;color:#2980b9", "结算余额 {row.settlement.to_decimal_string()} {g.currency}" }
                        }
                    }
                }
            }
            if !has_stats {
                p { color: "#999", "选择交易对象与月份范围，点击「查对象统计」" }
            } else if groups.is_empty() {
                p { color: "#999", "所选月份无匹配的收支记录" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{ObjectStatItem, ObjectStatView};
    use std::collections::HashMap;

    fn item(id: i64, direction: &str, value: i64, divisor: u32) -> ObjectStatItem {
        ObjectStatItem {
            id,
            name: "钱包".into(),
            direction: direction.into(),
            currency: "RMB".into(),
            amount: Money { value, divisor },
        }
    }

    /// 验证逐月结算余额：当前余额 70，7 月收入 100、8 月支出 30，
    /// 则 7 月末余额 100、8 月末余额 70（从当前余额向前倒推）。
    #[test]
    fn settlement_balance_walks_backwards_from_current() {
        let stats = vec![
            ObjectStatView {
                month: "2026-07".into(),
                objects: vec![item(1, "balance", 70, 1), item(1, "income", 100, 1)],
            },
            ObjectStatView {
                month: "2026-08".into(),
                objects: vec![item(1, "balance", 70, 1), item(1, "expense", 30, 1)],
            },
        ];
        let groups = build_groups(&stats, &HashMap::new(), None);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].balance, Money::from_int(70));
        assert_eq!(groups[0].monthly.len(), 2);
        assert_eq!(groups[0].monthly[0].month, "2026-07");
        assert_eq!(groups[0].monthly[0].settlement, Money::from_int(100));
        assert_eq!(groups[0].monthly[1].month, "2026-08");
        assert_eq!(groups[0].monthly[1].settlement, Money::from_int(70));
    }

    /// 验证展示结束月份只过滤显示：即使只看 7 月，
    /// 结算余额仍会用 8 月的支出倒推（100 而不是 70）
    #[test]
    fn display_to_filters_rows_but_keeps_accurate_settlement() {
        let stats = vec![
            ObjectStatView {
                month: "2026-07".into(),
                objects: vec![item(1, "balance", 70, 1), item(1, "income", 100, 1)],
            },
            ObjectStatView {
                month: "2026-08".into(),
                objects: vec![item(1, "balance", 70, 1), item(1, "expense", 30, 1)],
            },
        ];
        let groups = build_groups(&stats, &HashMap::new(), Some("2026-07"));
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].monthly.len(), 1);
        assert_eq!(groups[0].monthly[0].month, "2026-07");
        assert_eq!(groups[0].monthly[0].settlement, Money::from_int(100));
    }

    /// 验证前端汇总时也自动去零：1.3 + 2.7 = 4（4,1 而非 40,10）
    #[test]
    fn settlement_sums_are_normalized() {
        let stats = vec![ObjectStatView {
            month: "2026-08".into(),
            objects: vec![
                item(1, "balance", 4, 1),
                item(1, "income", 13, 10),
                item(1, "income", 27, 10),
            ],
        }];
        let groups = build_groups(&stats, &HashMap::new(), None);
        let row = &groups[0].monthly[0];
        assert_eq!(row.income, Some(Money::from_int(4)));
        assert_eq!(row.settlement, Money::from_int(4));
    }

    /// 验证所选范围内无记录的对象不生成空卡片
    #[test]
    fn empty_range_skips_group() {
        let stats = vec![ObjectStatView {
            month: "2026-08".into(),
            objects: vec![item(1, "balance", 0, 1), item(1, "income", 10, 1)],
        }];
        let groups = build_groups(&stats, &HashMap::new(), Some("2026-07"));
        assert!(groups.is_empty());
    }
}
