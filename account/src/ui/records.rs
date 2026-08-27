//! 第一页：交易记录浏览器
//!
//! 手动指定时间范围与交易对象过滤 → 拉取记录。
//! 新建交易组：组名 + 多行记录表单。
//! 删除单条记录会弹出确认框并列出该记录内容；
//! 删除整组仅确认组名（不逐一列出组内记录），确认后整组删除。

use super::{
    ConfirmDialog, DayTimeInput, GlobalState, GroupDraftRow, RecordView, build_iso, cp_label,
    dt_local, http_post, http_put, parse_money, push_toast,
};
use crate::models::{Counterparty, Currency, RecordSubmit};
use dioxus::prelude::*;

#[component]
pub(crate) fn RecordsPage() -> Element {
    let mut st = use_context::<GlobalState>();
    let cp: Vec<Counterparty> = (st.counterparties)().clone();
    let currencies: Vec<Currency> = (st.currencies)().clone();
    let views: Vec<RecordView> = (st.records)().clone();
    let query_from = dt_local(&(st.query_from)());
    let query_to = dt_local(&(st.query_to)());
    let query_cp = (st.query_cp)();
    let drafts_len = (st.group_drafts)().len();
    let has_records = !views.is_empty();
    let has_cp = !cp.is_empty();

    rsx! {
        div { style: "display:flex;flex-direction:column;gap:12px",
            h3 { "交易记录浏览器" }
            div { style: "display:flex;gap:8px;flex-wrap:wrap;align-items:center",
                label { "从:" }
                DayTimeInput {
                    value: query_from,
                    onchange: move |v| st.query_from.set(v),
                }
                label { "到:" }
                DayTimeInput {
                    value: query_to,
                    onchange: move |v| st.query_to.set(v),
                }
                label { "对象:" }
                select {
                    value: query_cp,
                    onchange: move |e| st.query_cp.set(e.value()),
                    option { value: "", "全部" }
                    for c in cp.iter() {
                        option { value: "{c.id}", "{cp_label(c, &currencies)}" }
                    }
                }
                button {
                    onclick: move |_| {
                        let mut st = st;
                        spawn(async move {
                            let from = (st.query_from)();
                            let to = (st.query_to)();
                            let cpid = (st.query_cp)();
                            let b = (st.base)();
                            let body = serde_json::json!({
                                "from": if from.trim().is_empty() { None } else { Some(build_iso(&from)) },
                                "to": if to.trim().is_empty() { None } else { Some(build_iso(&to)) },
                                "counterparty_id": if cpid.is_empty() { None } else { cpid.parse::<i64>().ok() },
                            });
                            match http_put(&b, "/api/records", Some(body)).await {
                                Ok(v) => st.records.set(serde_json::from_value(v).unwrap_or_default()),
                                Err(e) => push_toast(&st, &format!("查询失败: {e}")),
                            }
                        });
                    },
                    "🔍 拉取记录"
                }
            }

            div { style: "border:1px solid #ccc;padding:8px;border-radius:4px",
                strong { "新建交易组" }
                if !has_cp {
                    p { style: "font-size:12px;color:#888", "请先在「对象/货币」页添加交易对象与货币" }
                }
                div { style: "display:flex;gap:8px;margin-top:6px;align-items:center",
                    label { "组名:" }
                    input {
                        value: st.new_group_name,
                        placeholder: "组名称（≤64 字符）",
                        oninput: move |e| st.new_group_name.set(e.value()),
                    }
                }
                for i in 0..drafts_len {
                    GroupRecordRow { idx: i }
                }
                div { style: "display:flex;gap:8px;margin-top:8px;align-items:center",
                    button {
                        onclick: move |_| st.group_drafts.write().push(GroupDraftRow::new()),
                        "➕ 添加记录行"
                    }
                    button {
                        onclick: move |_| {
                            let mut st = st;
                            spawn(async move {
                                let group_name_check = (st.new_group_name)();
                                if group_name_check.trim().is_empty() {
                                    push_toast(&st, "组名称不能为空");
                                    return;
                                }
                                let drafts = (st.group_drafts)();
                                if drafts.is_empty() {
                                    push_toast(&st, "请至少添加一条记录");
                                    return;
                                }
                                let mut submits: Vec<RecordSubmit> = Vec::new();
                                for (i, d) in drafts.iter().enumerate() {
                                    if d.from_id == 0 || d.to_id == 0 || d.currency_id == 0 {
                                        push_toast(&st, &format!("第 {} 行：请选择转出/转入对象与货币", i + 1));
                                        return;
                                    }
                                    if d.from_id == d.to_id {
                                        push_toast(&st, &format!("第 {} 行：转出与转入不能是同一对象", i + 1));
                                        return;
                                    }
                                    let amount = match parse_money(&d.amount) {
                                        Some(m) => m,
                                        None => {
                                            push_toast(&st, &format!("第 {} 行：金额格式不正确", i + 1));
                                            return;
                                        }
                                    };
                                    submits.push(RecordSubmit {
                                        from_id: d.from_id,
                                        to_id: d.to_id,
                                        amount,
                                        currency_id: d.currency_id,
                                        remark: if d.remark.trim().is_empty() {
                                            None
                                        } else {
                                            Some(d.remark.trim().to_string())
                                        },
                                        traded_at: build_iso(&d.traded_at),
                                    });
                                }
                                let b = (st.base)();
                                let group_name = (st.new_group_name)();
                                let body = serde_json::json!({ "name": group_name.trim(), "records": submits });
                                match http_post(&b, "/api/groups", Some(body)).await {
                                    Ok(v) => {
                                        push_toast(&st, &format!("组已创建 id={}", v["id"]));
                                        st.group_drafts.set(vec![GroupDraftRow::new()]);
                                        st.new_group_name.set("".into());
                                        // 余额已联动变化，刷新交易对象列表
                                        let b2 = (st.base)();
                                        if let Ok(v2) = http_put(&b2, "/api/counterparties", None).await {
                                            st.counterparties.set(serde_json::from_value(v2).unwrap_or_default());
                                        }
                                    }
                                    Err(e) => push_toast(&st, &format!("创建失败: {e}")),
                                }
                            });
                        },
                        "💾 创建组"
                    }
                }
            }

            h4 { "记录" }
            for rec in views {
                RecordRow { rec: rec }
            }
            if !has_records {
                p { color: "#999", "暂无记录，点击拉取" }
            }
        }
    }
}

/// 新建组表单中的一行记录
#[component]
fn GroupRecordRow(idx: usize) -> Element {
    let mut st = use_context::<GlobalState>();
    let cp: Vec<Counterparty> = (st.counterparties)().clone();
    let currencies: Vec<Currency> = (st.currencies)().clone();
    let row = (st.group_drafts)().get(idx).cloned().unwrap_or_else(GroupDraftRow::new);
    let from_id = row.from_id.to_string();
    let to_id = row.to_id.to_string();
    let currency_id = row.currency_id.to_string();
    let amount = row.amount.clone();
    let remark = row.remark.clone();
    let traded_at = row.traded_at.clone();

    rsx! {
        div { style: "display:flex;gap:8px;flex-wrap:wrap;align-items:center;margin-top:6px;border-top:1px dashed #eee;padding-top:6px",
            select {
                value: from_id,
                onchange: move |e| {
                    let mut d = st.group_drafts.write();
                    if idx < d.len() { d[idx].from_id = e.value().parse().unwrap_or(0); }
                },
                option { value: "0", "转出 (从)" }
                for c in cp.iter() {
                    option { value: "{c.id}", "{cp_label(c, &currencies)}" }
                }
            }
            span { "→" }
            select {
                value: to_id,
                onchange: move |e| {
                    let mut d = st.group_drafts.write();
                    if idx < d.len() { d[idx].to_id = e.value().parse().unwrap_or(0); }
                },
                option { value: "0", "转入 (到)" }
                for c in cp.iter() {
                    option { value: "{c.id}", "{cp_label(c, &currencies)}" }
                }
            }
            input {
                value: amount,
                placeholder: "金额（正数）",
                oninput: move |e| {
                    let mut d = st.group_drafts.write();
                    if idx < d.len() {
                        // 交易记录金额仅允许正数：过滤负号，只保留数字与小数点
                        let v: String = e.value()
                            .chars()
                            .filter(|c| c.is_ascii_digit() || *c == '.')
                            .collect();
                        d[idx].amount = v;
                    }
                },
            }
            select {
                value: currency_id,
                onchange: move |e| {
                    let mut d = st.group_drafts.write();
                    if idx < d.len() { d[idx].currency_id = e.value().parse().unwrap_or(0); }
                },
                option { value: "0", "货币" }
                for c in currencies.iter() {
                    option { value: "{c.id}", "{c.name}" }
                }
            }
            DayTimeInput {
                value: traded_at,
                onchange: move |v| {
                    let mut d = st.group_drafts.write();
                    if idx < d.len() { d[idx].traded_at = v; }
                },
            }
            input {
                value: remark,
                placeholder: "备注 (≤256)",
                oninput: move |e| {
                    let mut d = st.group_drafts.write();
                    if idx < d.len() { d[idx].remark = e.value(); }
                },
            }
            button {
                onclick: move |_| {
                    let mut d = st.group_drafts.write();
                    if idx < d.len() { d.remove(idx); }
                },
                "✕"
            }
        }
    }
}

/// 单条交易记录行
#[component]
fn RecordRow(rec: RecordView) -> Element {
    let st = use_context::<GlobalState>();
    let rid = rec.id;
    let gid = rec.group_id;
    let traded_at = rec.traded_at.clone();
    let from = rec.from.clone();
    let to = rec.to.clone();
    let amount_currency = format!("{} {}", rec.amount.to_decimal_string(), rec.currency);
    let remark = rec.remark.clone();
    let group_label = rec.group_name.clone().unwrap_or_else(|| format!("组#{gid}"));
    // 供删除回调闭包捕获的独立副本（move 闭包会消耗捕获得值）
    let delete_traded_at = traded_at.clone();
    // 删除确认弹窗开关
    let mut pending_delete_record = use_signal(|| false);
    let mut pending_delete_group = use_signal(|| false);

    rsx! {
        div { style: "border:1px solid #eee;padding:6px;border-radius:4px;display:flex;gap:8px;align-items:center",
            span { "{traded_at}" }
            span { "{from} → {to}" }
            span { strong { "{amount_currency}" } }
            if let Some(r) = &remark {
                span { color: "#888", "({r})" }
            }
            span { color: "#bbb", "{group_label}" }
            button {
                onclick: move |_| pending_delete_record.set(true),
                "🗑 删"
            }
            button {
                onclick: move |_| pending_delete_group.set(true),
                "🗑 删组"
            }
        }

        // 删除单条记录确认弹窗：列出将要删除的记录内容
        ConfirmDialog {
            open: pending_delete_record,
            title: String::from("确认删除该记录？"),
            message: format!(
                "时间: {traded_at}\n交易: {from} → {to}\n金额: {amount_currency}\n组: {group_label}\n\n删除后交易对象余额将自动回滚。"
            ),
            on_confirm: move |_| {
                let mut st = st;
                // 闭包需 FnMut：从预副本 clone，避免 move 共享的 String 影响后续弹窗
                let traded_at = delete_traded_at.clone();
                spawn(async move {
                    let b = (st.base)();
                    let body = serde_json::json!({ "id": rid, "traded_at": traded_at });
                    match http_post(&b, "/api/records/delete", Some(body)).await {
                        Ok(_) => {
                            // 先刷新交易对象列表（余额已联动变化），
                            // 最后再移除本行：此时组件仍挂载，
                            // 若先 retain 会卸载当前组件导致 spawn 被取消
                            let b2 = (st.base)();
                            if let Ok(v2) = http_put(&b2, "/api/counterparties", None).await {
                                st.counterparties.set(serde_json::from_value(v2).unwrap_or_default());
                            }
                            st.records.write().retain(|r| r.id != rid);
                        }
                        Err(e) => push_toast(&st, &format!("删除失败: {e}")),
                    }
                });
            }
        }

        // 删除整组确认弹窗：只提示组名，不列出组内具体交易记录
        ConfirmDialog {
            open: pending_delete_group,
            title: String::from("确认删除该组？"),
            message: format!(
                "确定删除组 {group_label}？\n该组下的全部交易记录将一并删除，删除后交易对象余额将自动回滚。"
            ),
            on_confirm: move |_| {
                let mut st = st;
                // 闭包需 FnMut：gid 是 Copy 类型，可直接捕获
                spawn(async move {
                    let b = (st.base)();
                    let body = serde_json::json!({ "id": gid });
                    match http_post(&b, "/api/groups/delete", Some(body)).await {
                        Ok(_) => {
                            // 先刷新交易对象列表（余额已联动变化），
                            // 最后再移除本组记录：此时组件仍挂载，
                            // 若先 retain 会卸载当前组件导致 spawn 被取消
                            let b2 = (st.base)();
                            if let Ok(v2) = http_put(&b2, "/api/counterparties", None).await {
                                st.counterparties.set(serde_json::from_value(v2).unwrap_or_default());
                            }
                            st.records.write().retain(|r| r.group_id != gid);
                        }
                        Err(e) => push_toast(&st, &format!("删除组失败: {e}")),
                    }
                });
            }
        }
    }
}