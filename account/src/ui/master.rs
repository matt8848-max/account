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

            h4 { "货币列表" }
            div { style: "display:flex;gap:8px;flex-wrap:wrap",
                for c in currencies {
                    CurrencyRow { c: c }
                }
                if !has_cur {
                    p { color: "#999", "暂无货币" }
                }
            }

            h4 { "交易对象列表" }
            div { style: "display:flex;gap:8px;flex-wrap:wrap",
                for c in counterparties {
                    CounterpartyRow { c: c }
                }
                if !has_cp {
                    p { color: "#999", "暂无交易对象" }
                }
            }
        }
    }
}

/// 单条货币行
#[component]
fn CurrencyRow(c: Currency) -> Element {
    let st = use_context::<GlobalState>();
    let cid = c.id;
    let cname = c.name.clone();
    // 删除确认弹窗开关
    let mut pending_delete = use_signal(|| false);

    rsx! {
        div { style: "border:1px solid #ddd;padding:4px 8px;border-radius:4px;display:flex;gap:8px;align-items:center",
            span { "{cname}" }
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

/// 单条交易对象行
#[component]
fn CounterpartyRow(c: Counterparty) -> Element {
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
        div { style: "border:1px solid #ddd;padding:4px 8px;border-radius:4px;display:flex;gap:8px;align-items:center",
            span { "{cname}" }
            span { style: "font-size:12px;color:#888", "余额 {bal} {cur_name}" }
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