//! 第一页：交易记录浏览器
//!
//! 手动指定时间范围与交易对象过滤 → 拉取记录。
//! 新建交易组：组名 + 多行记录表单。
//! 「记录」标题右侧提供排序方向切换（默认从新到旧，可切为从旧到新）。
//! 删除单条记录会弹出确认框并列出该记录内容；
//! 删除整组仅确认组名（不逐一列出组内记录），确认后整组删除。

use super::{
    ConfirmDialog, DayTimeInput, GlobalState, GroupDraftRow, RecordView, SearchSelect, build_iso,
    cp_label, dt_local, http_post, http_put, parse_money, push_toast,
};
use crate::models::{Counterparty, Currency, RecordSubmit};
use dioxus::prelude::*;

#[component]
pub(crate) fn RecordsPage() -> Element {
    let mut st = use_context::<GlobalState>();

    // 进入本页时主动刷新记录（切页返回后无需手动操作即可看到最新数据）
    let did_auto_load = use_signal(|| false);
    use_effect(move || {
        let mut st = st;
        let mut did = did_auto_load;
        if !(did)() {
            did.set(true);
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
                    Ok(v) => st
                        .records
                        .set(serde_json::from_value(v).unwrap_or_default()),
                    Err(e) => push_toast(&st, &format!("刷新记录失败: {e}")),
                }
            });
        }
    });

    let cp: Vec<Counterparty> = (st.counterparties)().clone();
    let currencies: Vec<Currency> = (st.currencies)().clone();
    // 展示用的记录副本按当前排序方向排列：不改动 st.records 缓存本身，
    // 否则切换排序会污染下一次刷新的落库顺序
    let mut views: Vec<RecordView> = (st.records)().clone();
    let order = (st.records_order)();
    sort_records(&mut views, order);
    let query_from = dt_local(&(st.query_from)());
    let query_to = dt_local(&(st.query_to)());
    let query_cp = (st.query_cp)();
    // 对象筛选候选（含顶部「全部」项由 empty_label 提供）
    let filter_candidates: Vec<(i64, String)> = cp
        .iter()
        .map(|c| (c.id, cp_label(c, &currencies)))
        .collect();
    let query_cp_value: i64 = query_cp.parse().unwrap_or(0);
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
                SearchSelect {
                    candidates: filter_candidates,
                    value: query_cp_value,
                    placeholder: String::from("筛选对象"),
                    empty_label: Some(String::from("全部")),
                    onchange: move |id: i64| {
                        st.query_cp.set(if id == 0 { String::new() } else { id.to_string() });
                    },
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

            // 记录标题 + 排序切换按钮（标题右侧）
            div { style: "display:flex;align-items:center;gap:8px",
                h4 { style: "margin:0", "记录" }
                SortToggle {
                    order,
                    onchange: move |o: SortOrder| st.records_order.set(o),
                }
            }
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
    let row = (st.group_drafts)()
        .get(idx)
        .cloned()
        .unwrap_or_else(GroupDraftRow::new);
    let currency_id = row.currency_id.to_string();
    let amount = row.amount.clone();
    let remark = row.remark.clone();
    let traded_at = row.traded_at.clone();

    // 候选列表：(交易对象 id, 标签「名称 (货币)」)——供 SearchSelect 搜索选择
    let candidates: Vec<(i64, String)> = cp
        .iter()
        .map(|c| (c.id, cp_label(c, &currencies)))
        .collect();

    rsx! {
        div { style: "display:flex;gap:8px;flex-wrap:wrap;align-items:center;margin-top:6px;border-top:1px dashed #eee;padding-top:6px",
            SearchSelect {
                candidates: candidates.clone(),
                value: row.from_id,
                placeholder: String::from("转出 (从)"),
                onchange: move |id| {
                    let mut d = st.group_drafts.write();
                    if idx < d.len() { d[idx].from_id = id; }
                },
            }
            span { "→" }
            SearchSelect {
                candidates,
                value: row.to_id,
                placeholder: String::from("转入 (到)"),
                onchange: move |id| {
                    let mut d = st.group_drafts.write();
                    if idx < d.len() { d[idx].to_id = id; }
                },
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
    let group_label = rec
        .group_name
        .clone()
        .unwrap_or_else(|| format!("组#{gid}"));
    // 组包含的月份（用于删除组时定位组实例；同 group_id 可跨月复用）
    let group_months = rec.group_months.clone();
    // 供删除回调闭包捕获的独立副本（move 闭包会消耗捕获得值）
    let delete_traded_at = traded_at.clone();
    // 供删除组闭包捕获的组月份副本
    let delete_group_months = group_months.clone();
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
            button {
                onclick: move |_| pending_delete_record.set(true),
                "🗑 删"
            }
            span { color: "#bbb", "{group_label}" }
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
                // 闭包需 FnMut：gid 是 Copy 类型可直接捕获；
                // group_months 为 String，用预副本 clone 供定位组实例
                let group_months = delete_group_months.clone();
                // serde_json::json! 会 move 走 group_months，额外保留一份用于前端过滤
                let group_months_filter = group_months.clone();
                spawn(async move {
                    let b = (st.base)();
                    let body = serde_json::json!({ "id": gid, "group_months": group_months });
                    match http_post(&b, "/api/groups/delete", Some(body)).await {
                        Ok(_) => {
                            // 先刷新交易对象列表（余额已联动变化），
                            // 最后再移除本组记录：此时组件仍挂载，
                            // 若先 retain 会卸载当前组件导致 spawn 被取消。
                            // 新方案下同 group_id 可跨月复用，须按「组实例」
                            // （group_id + group_months）定位，避免误删同号其他组。
                            let b2 = (st.base)();
                            if let Ok(v2) = http_put(&b2, "/api/counterparties", None).await {
                                st.counterparties.set(serde_json::from_value(v2).unwrap_or_default());
                            }
                            st.records.write().retain(|r| {
                                r.group_id != gid
                                    || r.group_months.as_deref() != group_months_filter.as_deref()
                            });
                        }
                        Err(e) => push_toast(&st, &format!("删除组失败: {e}")),
                    }
                });
            }
        }
    }
}

// ---------- 记录排序 ----------

/// 记录列表的排序方向
///
/// 默认 [`SortOrder::NewestFirst`]：交易记录页打开时最新的记录排在最上面。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortOrder {
    /// 从新到旧（最新在前）
    NewestFirst,
    /// 从旧到新（最旧在前）
    OldestFirst,
}

impl SortOrder {
    /// 切换方向
    pub(crate) fn toggled(self) -> Self {
        match self {
            SortOrder::NewestFirst => SortOrder::OldestFirst,
            SortOrder::OldestFirst => SortOrder::NewestFirst,
        }
    }

    /// 供按钮 `title` 与提示使用的中文说明
    pub(crate) fn label(self) -> &'static str {
        match self {
            SortOrder::NewestFirst => "从新到旧",
            SortOrder::OldestFirst => "从旧到新",
        }
    }
}

/// 按交易时间排序记录
///
/// `traded_at` 是补零的 ISO 字符串（`YYYY-MM-DDTHH:MM:SS`），字典序即时间序，
/// 无需解析成时间类型；`sort_by` 为稳定排序，同一时刻的记录保持后端返回顺序。
pub(crate) fn sort_records(records: &mut [RecordView], order: SortOrder) {
    match order {
        SortOrder::NewestFirst => records.sort_by(|a, b| b.traded_at.cmp(&a.traded_at)),
        SortOrder::OldestFirst => records.sort_by(|a, b| a.traded_at.cmp(&b.traded_at)),
    }
}

/// 排序方向切换按钮
///
/// 竖排两个三角箭头：**实心代表当前方向**。
/// 从新到旧（最新在上）时向上实心；从旧到新（向下即越来越新）时向下实心。
#[component]
fn SortToggle(order: SortOrder, onchange: EventHandler<SortOrder>) -> Element {
    let newest_first = order == SortOrder::NewestFirst;
    let up = if newest_first { "▲" } else { "△" };
    let down = if newest_first { "▽" } else { "▼" };

    rsx! {
        button {
            title: "当前排序：{order.label()}（点击切换）",
            style: "display:flex;flex-direction:column;align-items:center;line-height:11px;font-size:11px;padding:2px 6px;border:1px solid #ccc;border-radius:4px;background:#fff;cursor:pointer;color:#333",
            onclick: move |_| onchange.call(order.toggled()),
            span { "{up}" }
            span { "{down}" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RecordView, SortOrder, sort_records};
    use crate::models::Money;

    /// 构造一条仅关心交易时间的记录视图（其余字段取固定值）
    fn rec(id: i64, traded_at: &str) -> RecordView {
        RecordView {
            id,
            group_id: 1,
            group_name: None,
            group_months: None,
            from: "支付宝".to_string(),
            from_id: 1,
            to: "微信钱包".to_string(),
            to_id: 2,
            amount: Money::from_int(100),
            currency: "RMB".to_string(),
            remark: None,
            traded_at: traded_at.to_string(),
        }
    }

    /// 验证默认「从新到旧」：最新时间排在最前；切换后变「从旧到新」
    #[test]
    fn sort_records_should_order_by_traded_at() {
        let mut records = vec![
            rec(1, "2026-11-02T08:00:00"),
            rec(2, "2026-11-01T23:59:59"),
            rec(3, "2026-11-03T00:00:00"),
        ];

        sort_records(&mut records, SortOrder::NewestFirst);
        let ids: Vec<i64> = records.iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![3, 1, 2], "从新到旧：最新在前");
        // 不相等断言：反向排序的结果必须不同（防止实现忽略方向）
        assert_ne!(ids, vec![2, 1, 3]);

        sort_records(&mut records, SortOrder::OldestFirst);
        let ids: Vec<i64> = records.iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![2, 1, 3], "从旧到新：最旧在前");
    }

    /// 验证边界：空集合与非空集合都不应 panic；单元素集合保持原样
    #[test]
    fn sort_records_should_handle_empty_and_single() {
        let mut empty: Vec<RecordView> = Vec::new();
        sort_records(&mut empty, SortOrder::NewestFirst);
        assert!(empty.is_empty());

        let mut single = vec![rec(7, "2026-11-06T12:00:00")];
        sort_records(&mut single, SortOrder::OldestFirst);
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].id, 7);
    }

    /// 验证切换方向：两个方向互逆，且方向文案不同
    #[test]
    fn sort_order_toggle_should_be_involutive() {
        assert_eq!(SortOrder::NewestFirst.toggled(), SortOrder::OldestFirst);
        assert_eq!(
            SortOrder::OldestFirst.toggled().toggled(),
            SortOrder::OldestFirst
        );
        assert_ne!(
            SortOrder::NewestFirst.label(),
            SortOrder::OldestFirst.label()
        );
    }
}
