//! 第二页：交易对象/货币管理
//!
//! 货币与交易对象的增删；交易对象支持初始余额输入。
//! 列表实时显示最新余额（增删记录后由全局状态自动刷新）；
//! 必须增删成功才更新本地内存缓存。

use super::{ConfirmDialog, GlobalState, http_post, parse_money, push_toast};
use crate::models::{Counterparty, Currency};
use dioxus::prelude::*;

#[component]
pub(crate) fn MasterPage() -> Element {
    let mut st = use_context::<GlobalState>();
    let currencies: Vec<Currency> = (st.currencies)().clone();
    let counterparties: Vec<Counterparty> = (st.counterparties)().clone();
    let has_cur = !currencies.is_empty();
    let has_cp = !counterparties.is_empty();
    // 搜索词（空 → 显示全部，符合「没有内容时全显示」的要求）
    let mut cur_search = use_signal(String::new);
    let mut cp_search = use_signal(String::new);

    // 货币列表过滤：按名称子串匹配，忽略大小写
    let cq = (cur_search)().trim().to_lowercase();
    let filtered_currencies: Vec<Currency> = currencies
        .iter()
        .filter(|c| cq.is_empty() || c.name.to_lowercase().contains(&cq))
        .cloned()
        .collect();

    // 交易对象列表过滤：名称或货币类型子串匹配，忽略大小写
    let cur_names: std::collections::HashMap<i64, String> =
        currencies.iter().map(|c| (c.id, c.name.clone())).collect();
    let pq = (cp_search)().trim().to_lowercase();
    let filtered_counterparties: Vec<Counterparty> = counterparties
        .iter()
        .filter(|c| {
            if pq.is_empty() {
                return true;
            }
            if c.name.to_lowercase().contains(&pq) {
                return true;
            }
            cur_names
                .get(&c.currency_id)
                .is_some_and(|n| n.to_lowercase().contains(&pq))
        })
        .cloned()
        .collect();

    rsx! {
        div { style: "display:flex;flex-direction:column;gap:16px",
            h3 { "交易对象与货币管理" }

            div { style: "border:1px solid #eee;padding:8px;border-radius:4px",
                strong { "添加货币" }
                div { style: "display:flex;gap:8px;margin-top:6px",
                    input {
                        value: st.new_currency_name,
                        placeholder: "货币名称，如 RMB",
                        oninput: move |e| st.new_currency_name.set(e.value()),
                    }
                    button {
                        onclick: move |_| {
                            let mut st = st;
                            let name = (st.new_currency_name)();
                            spawn(async move {
                                let b = (st.base)();
                                let body = serde_json::json!({ "name": name.trim() });
                                match http_post(&b, "/api/currencies", Some(body)).await {
                                    Ok(v) => {
                                        let id = v["id"].as_i64().unwrap_or(0);
                                        let nm = name.trim().to_string();
                                        st.currencies.write().push(Currency { id, name: nm });
                                        st.new_currency_name.set("".into());
                                    }
                                    Err(e) => push_toast(&st, &format!("添加货币失败: {e}")),
                                }
                            });
                        },
                        "添加"
                    }
                }
            }

            div { style: "border:1px solid #eee;padding:8px;border-radius:4px",
                strong { "添加交易对象" }
                div { style: "display:flex;gap:8px;margin-top:6px;flex-wrap:wrap;align-items:center",
                    input {
                        value: st.new_cp_name,
                        placeholder: "名称，如 微信钱包",
                        oninput: move |e| st.new_cp_name.set(e.value()),
                    }
                    select {
                        value: st.new_cp_currency,
                        onchange: move |e| st.new_cp_currency.set(e.value()),
                        for c in currencies.iter() {
                            option { value: "{c.id}", "{c.name}" }
                        }
                    }
                    input {
                        value: st.new_cp_balance,
                        placeholder: "初始余额",
                        oninput: move |e| st.new_cp_balance.set(e.value()),
                    }
                    button {
                        onclick: move |_| {
                            let mut st = st;
                            let name = (st.new_cp_name)();
                            let balance_text = (st.new_cp_balance)();
                            let cur_id: i64 = (st.new_cp_currency)().trim().parse().unwrap_or(0);
                            let balance = match parse_money(&balance_text) {
                                Some(m) => m,
                                None => {
                                    push_toast(&st, "初始余额格式不正确，应为数字（如 100 或 100.50）");
                                    return;
                                }
                            };
                            spawn(async move {
                                let b = (st.base)();
                                let body = serde_json::json!({
                                    "name": name.trim(),
                                    "currency_id": cur_id,
                                    "balance": {
                                        "value": balance.value,
                                        "divisor": balance.divisor,
                                    },
                                });
                                match http_post(&b, "/api/counterparties", Some(body)).await {
                                    Ok(v) => {
                                        let id = v["id"].as_i64().unwrap_or(0);
                                        let nm = name.trim().to_string();
                                        st.counterparties.write().push(Counterparty {
                                            id,
                                            name: nm,
                                            currency_id: cur_id,
                                            balance,
                                        });
                                        st.new_cp_name.set("".into());
                                        st.new_cp_balance.set("0".into());
                                    }
                                    Err(e) => push_toast(&st, &format!("添加对象失败: {e}")),
                                }
                            });
                        },
                        "添加"
                    }
                }
            }

            // 货币列表：搜索 + 灰白相间列表（名称居中）
            div { style: "display:flex;align-items:center;gap:8px;flex-wrap:wrap",
                h4 { style: "margin:0", "货币列表" }
                input {
                    style: "width:220px",
                    value: cur_search,
                    placeholder: "搜索货币...",
                    oninput: move |e| cur_search.set(e.value()),
                }
                span { style: "font-size:12px;color:#888", "{filtered_currencies.len()} / {currencies.len()}" }
            }
            div { style: "border:1px solid #ddd;border-radius:4px;overflow:hidden",
                div { style: "display:grid;grid-template-columns:minmax(0,1fr) 48px;align-items:center;padding:4px 10px;background:#eef0f3;color:#666;font-size:12px",
                    span { style: "text-align:center", "货币" }
                    span { "" }
                }
                for (idx, c) in filtered_currencies.iter().cloned().enumerate() {
                    CurrencyRow { c: c, striped: idx % 2 == 1 }
                }
                if filtered_currencies.is_empty() {
                    div { style: "padding:8px 10px;color:#999",
                        if has_cur { "无匹配货币" } else { "暂无货币" }
                    }
                }
            }

            // 交易对象列表：搜索 + 灰白相间三列（名称左 / 金额右 / 货币类型右）
            div { style: "display:flex;align-items:center;gap:8px;flex-wrap:wrap",
                h4 { style: "margin:0", "交易对象列表" }
                input {
                    style: "width:240px",
                    value: cp_search,
                    placeholder: "搜索名称或货币类型...",
                    oninput: move |e| cp_search.set(e.value()),
                }
                span { style: "font-size:12px;color:#888", "{filtered_counterparties.len()} / {counterparties.len()}" }
            }
            div { style: "border:1px solid #ddd;border-radius:4px;overflow:hidden",
                div { style: "display:grid;grid-template-columns:minmax(0,1fr) 120px 100px 48px;gap:8px;align-items:center;padding:4px 10px;background:#eef0f3;color:#666;font-size:12px",
                    span { "名称" }
                    span { style: "text-align:right", "金额" }
                    span { style: "text-align:right", "货币类型" }
                    span { "" }
                }
                for (idx, c) in filtered_counterparties.iter().cloned().enumerate() {
                    CounterpartyRow { c: c, striped: idx % 2 == 1 }
                }
                if filtered_counterparties.is_empty() {
                    div { style: "padding:8px 10px;color:#999",
                        if has_cp { "无匹配交易对象" } else { "暂无交易对象" }
                    }
                }
            }

        }
    }
}

/// 单条货币行（灰白相间；名称居中）
#[component]
fn CurrencyRow(c: Currency, striped: bool) -> Element {
    let st = use_context::<GlobalState>();
    let cid = c.id;
    let cname = c.name.clone();
    // 删除确认弹窗开关
    let mut pending_delete = use_signal(|| false);

    rsx! {
        div {
            style: format!(
                "display:grid;grid-template-columns:minmax(0,1fr) 48px;align-items:center;gap:8px;padding:5px 10px;border-bottom:1px solid #eee;background:{}",
                if striped { "#f5f6f8" } else { "#fff" }
            ),
            span { style: "text-align:center;overflow:hidden;text-overflow:ellipsis;white-space:nowrap", "{cname}" }
            button {
                onclick: move |_| pending_delete.set(true),
                "✕"
            }
        }

        ConfirmDialog {
            open: pending_delete,
            title: String::from("确认删除该货币？"),
            message: format!("将删除货币: {cname}\n\n若货币仍被交易对象或记录引用，删除将失败。"),
            on_confirm: move |_| {
                let mut st = st;
                spawn(async move {
                    let b = (st.base)();
                    let body = serde_json::json!({ "id": cid });
                    match http_post(&b, "/api/currencies/delete", Some(body)).await {
                        Ok(_) => st.currencies.write().retain(|x| x.id != cid),
                        Err(e) => push_toast(&st, &format!("删除失败: {e}")),
                    }
                });
            }
        }
    }
}

/// 单条交易对象行（灰白相间；名称左 / 金额右 / 货币类型右）
#[component]
fn CounterpartyRow(c: Counterparty, striped: bool) -> Element {
    let st = use_context::<GlobalState>();
    let cid = c.id;
    let cname = c.name.clone();
    let bal = c.balance.to_decimal_string();
    let cur_name = (st.currencies)()
        .iter()
        .find(|x| x.id == c.currency_id)
        .map(|x| x.name.clone())
        .unwrap_or_else(|| format!("id={}", c.currency_id));
    // 删除确认弹窗开关
    let mut pending_delete = use_signal(|| false);

    rsx! {
        div {
            style: format!(
                "display:grid;grid-template-columns:minmax(0,1fr) 120px 100px 48px;gap:8px;align-items:center;padding:5px 10px;border-bottom:1px solid #eee;background:{}",
                if striped { "#f5f6f8" } else { "#fff" }
            ),
            span { style: "text-align:left;overflow:hidden;text-overflow:ellipsis;white-space:nowrap", "{cname}" }
            span { style: "text-align:right;font-variant-numeric:tabular-nums", "{bal}" }
            span { style: "text-align:right;color:#666;overflow:hidden;text-overflow:ellipsis;white-space:nowrap", "{cur_name}" }
            button {
                onclick: move |_| pending_delete.set(true),
                "✕"
            }
        }

        ConfirmDialog {
            open: pending_delete,
            title: String::from("确认删除该交易对象？"),
            message: format!("将删除交易对象: {cname}\n\n若对象仍被记录引用，删除将失败。"),
            on_confirm: move |_| {
                let mut st = st;
                spawn(async move {
                    let b = (st.base)();
                    let body = serde_json::json!({ "id": cid });
                    match http_post(&b, "/api/counterparties/delete", Some(body)).await {
                        Ok(_) => st.counterparties.write().retain(|x| x.id != cid),
                        Err(e) => push_toast(&st, &format!("删除失败: {e}")),
                    }
                });
            }
        }
    }
}
