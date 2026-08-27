//! 第三页：数据统计
//!
//! 对象统计：月份范围（`input type="month"`）+ 交易对象多选（checkbox），
//! 按**交易对象分组**展示：每组先显示一次当前余额（蓝），
//! 再列出所选月份范围内各月的收入（绿）/支出（红）。

use super::{
    GlobalState, ObjectGroupView, ObjectStatView, cp_label, http_put, push_toast,
};
use crate::models::{Counterparty, Currency, Money};
use dioxus::prelude::*;

#[component]
pub(crate) fn StatsPage() -> Element {
    let mut st = use_context::<GlobalState>();
    let currencies: Vec<Currency> = (st.currencies)().clone();
    let counterparties: Vec<Counterparty> = (st.counterparties)().clone();
    let all_stats: Vec<ObjectStatView> = (st.stats)().clone();
    let selected: Vec<i64> = (st.stats_cp)().clone();
    let stats_from = (st.stats_from)();
    let stats_to = (st.stats_to)();
    let has_stats = !all_stats.is_empty();
    // RSX 循环内不能写 let，先在函数体构建 (id, 标签) 对
    let cp_pairs: Vec<(i64, String)> = counterparties
        .iter()
        .map(|c| (c.id, cp_label(c, &currencies)))
        .collect();

    // 按交易对象分组：先显示余额（只显示一次），再显示各月收支
    let mut group_map: std::collections::HashMap<(i64, String), ObjectGroupView> =
        std::collections::HashMap::new();
    for m in all_stats.iter() {
        for item in m.objects.iter() {
            let key = (item.id, item.currency.clone());
            let g = group_map
                .entry(key)
                .or_insert_with(|| ObjectGroupView {
                    name: item.name.clone(),
                    currency: item.currency.clone(),
                    balance: Money::from_int(0),
                    monthly: Vec::new(),
                });
            if item.direction == "balance" {
                g.balance = item.amount;
            } else if let Some((_, inc, exp)) = g
                .monthly
                .iter_mut()
                .find(|(mon, _, _)| mon == &m.month)
            {
                if item.direction == "income" {
                    *inc = Some(item.amount);
                } else {
                    *exp = Some(item.amount);
                }
            } else {
                let inc = if item.direction == "income" {
                    Some(item.amount)
                } else {
                    None
                };
                let exp = if item.direction == "expense" {
                    Some(item.amount)
                } else {
                    None
                };
                g.monthly.push((m.month.clone(), inc, exp));
            }
        }
    }
    let mut groups: Vec<ObjectGroupView> = group_map.into_values().collect();
    groups.sort_by(|a, b| a.name.cmp(&b.name));
    for g in groups.iter_mut() {
        g.monthly.sort_by(|a, b| a.0.cmp(&b.0));
    }

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
                div { style: "display:flex;gap:8px;margin-top:6px;flex-wrap:wrap;align-items:center",
                    label { "交易对象:" }
                    for (cp_id, cp_text) in cp_pairs.clone() {
                        label { style: "display:flex;align-items:center;gap:4px",
                            input {
                                r#type: "checkbox",
                                checked: selected.contains(&cp_id),
                                onchange: move |e| {
                                    let mut sel = st.stats_cp.write();
                                    let id = cp_id;
                                    if e.checked() {
                                        if !sel.contains(&id) { sel.push(id); }
                                    } else {
                                        sel.retain(|x| *x != id);
                                    }
                                },
                            }
                            "{cp_text}"
                        }
                    }
                    if counterparties.is_empty() {
                        span { color: "#999", "暂无交易对象" }
                    }
                }
                div { style: "display:flex;gap:8px;margin-top:8px;align-items:center",
                    button {
                        onclick: move |_| {
                            let mut st = st;
                            spawn(async move {
                                let ids = (st.stats_cp)();
                                if ids.is_empty() {
                                    push_toast(&st, "请至少选择一个交易对象");
                                    return;
                                }
                                let from = (st.stats_from)();
                                let to = (st.stats_to)();
                                let b = (st.base)();
                                let body = serde_json::json!({
                                    "counterparty_ids": ids,
                                    "from": if from.trim().is_empty() { None } else { Some(from.trim()) },
                                    "to": if to.trim().is_empty() { None } else { Some(to.trim()) },
                                });
                                match http_put(&b, "/api/stats/objects", Some(body)).await {
                                    Ok(v) => st.stats.set(serde_json::from_value(v).unwrap_or_default()),
                                    Err(e) => push_toast(&st, &format!("查询失败: {e}")),
                                }
                            });
                        },
                        "🔍 查对象统计"
                    }
                }
            }

            // 人工检查发现，由于增删每条记录和组时，会自动更新交易对象余额，因此废弃
            // 需完整的单元测试验证（见 db/month.rs 余额联动测试），
            // 该功能的前端 UI 与后端 API（/api/stats/update）均已废弃
            // （见原 ui.rs 的注释块）

            h4 { "统计结果" }
            for g in groups.iter() {
                div { style: "border:1px solid #eee;padding:8px;border-radius:4px",
                    strong { "{g.name} ({g.currency})" }
                    p { style: "color:#2980b9;margin:4px 0", "当前余额: {g.balance.to_decimal_string()} {g.currency}" }
                    for (month, income, expense) in g.monthly.iter() {
                        p { style: "margin:2px 0",
                            span { "{month}: " }
                            if let Some(inc) = income {
                                span { style: "color:#1b7e3f", "收入 {inc.to_decimal_string()} " }
                            }
                            if let Some(exp) = expense {
                                span { style: "color:#c0392b", "支出 {exp.to_decimal_string()}" }
                            }
                            if income.is_none() && expense.is_none() {
                                span { color: "#999", "无记录" }
                            }
                        }
                    }
                    if g.monthly.is_empty() {
                        p { color: "#999", "所选月份无收支记录" }
                    }
                }
            }
            if !has_stats {
                p { color: "#999", "选择交易对象与月份范围，点击「查对象统计」" }
            }
        }
    }
}