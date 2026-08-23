//! 客户端 UI（Dioxus 页面）
//!
//! 与平台无关：native（桌面）与 wasm（Web）共用同一份 UI 组件。
//! HTTP 层通过 cfg 分支：native 用 reqwest（跳过自签名证书验证），
//! wasm 用 gloo-net。定时 sleep 同样 cfg 分支。
//!
//! 每个列表行抽成独立组件（RecordRow / CurrencyRow / CounterpartyRow），
//! 闭包只捕获自有数据，避免借用生命周期问题。

use crate::models::{Counterparty, Currency, Money, RecordSubmit};
#[cfg(not(target_arch = "wasm32"))]
use chrono::Offset;
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

const DEFAULT_BASE: &str = "https://127.0.0.2:3000";
const LIVE_INTERVAL_MS: u64 = 300_000; // 5 分钟

// ---------- 序列化视图 ----------

/// 对象统计视图（后端 /api/stats/objects 返回）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ObjectStatView {
    month: String,
    objects: Vec<ObjectStatItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ObjectStatItem {
    id: i64,
    name: String,
    direction: String,
    currency: String,
    amount: Money,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct RecordView {
    id: i64,
    group_id: i64,
    group_name: Option<String>,
    from: String,
    to: String,
    amount: Money,
    currency: String,
    remark: Option<String>,
    traded_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Records,
    Master,
    Stats,
}

// ---------- 全局状态 ----------

#[derive(Clone, Copy)]
struct GlobalState {
    base: Signal<String>,
    connected: Signal<bool>,
    status_msg: Signal<String>,
    live_ok: Signal<bool>,
    live_at: Signal<String>,
    api_version: Signal<String>,
    page: Signal<Page>,
    counterparties: Signal<Vec<Counterparty>>,
    currencies: Signal<Vec<Currency>>,
    query_from: Signal<String>,
    query_to: Signal<String>,
    query_cp: Signal<String>,
    records: Signal<Vec<RecordView>>,
    new_group_name: Signal<String>,
    new_currency_name: Signal<String>,
    new_cp_name: Signal<String>,
    new_cp_currency: Signal<String>,
    new_cp_balance: Signal<String>,
    group_drafts: Signal<Vec<GroupDraftRow>>,
    stats: Signal<Vec<ObjectStatView>>,
    stats_from: Signal<String>,
    stats_to: Signal<String>,
    stats_cp: Signal<Vec<i64>>,
}

/// 新建组草稿行（表单编辑用）
#[derive(Debug, Clone, PartialEq)]
struct GroupDraftRow {
    from_id: i64,
    to_id: i64,
    amount: String,
    currency_id: i64,
    remark: String,
    traded_at: String,
}

impl GroupDraftRow {
    fn new() -> Self {
        Self {
            from_id: 0,
            to_id: 0,
            amount: String::new(),
            currency_id: 0,
            remark: String::new(),
            traded_at: now_local_iso(),
        }
    }
}

/// 当前时间（datetime-local 格式，无秒）
fn now_local_iso() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        chrono::Utc::now().format("%Y-%m-%dT%H:%M").to_string()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        chrono::Local::now().format("%Y-%m-%dT%H:%M").to_string()
    }
}

/// datetime-local 值转后端 ISO 字符串（补秒）
fn build_iso(dt: &str) -> String {
    let dt = dt.trim();
    if dt.len() == 16 {
        format!("{dt}:00")
    } else {
        dt.to_string()
    }
}

/// 后端 ISO 时间转为 datetime-local 显示值（去掉秒）
fn dt_local(v: &str) -> String {
    if v.len() >= 16 { v[..16].to_string() } else { v.to_string() }
}

/// 24 小时制时间输入：日期 + 时(00-23) + 分(00-59)
///
/// 浏览器原生 `datetime-local` 在部分系统（如 Windows）按 12 小时制（AM/PM）
/// 显示；此组件改用 `type=date` + 时/分下拉框，强制 24 小时制。
/// `value` 约定为 `YYYY-MM-DDTHH:MM`（无秒），变更时通过 `onchange` 回传。
#[component]
fn DayTimeInput(value: String, onchange: EventHandler<String>) -> Element {
    let date = if value.len() >= 10 { value[..10].to_string() } else { String::new() };
    let hour = if value.len() >= 13 { value[11..13].to_string() } else { "00".to_string() };
    let minute = if value.len() >= 16 { value[14..16].to_string() } else { "00".to_string() };
    let hours: Vec<String> = (0..24).map(|h| format!("{h:02}")).collect();
    let minutes: Vec<String> = (0..60).map(|m| format!("{m:02}")).collect();
    // String 非 Copy，需 clone 供不同闭包捕获；
    // Callback 是 Copy，闭包内 move 即复制，无需 clone
    let h_in = hour.clone();
    let m_in = minute.clone();
    let d_hs = date.clone();
    let m_hs = minute.clone();
    let d_ms = date.clone();
    let h_ms = hour.clone();

    rsx! {
        div { style: "display:flex;gap:4px;align-items:center",
            input {
                r#type: "date",
                value: date,
                oninput: move |e| {
                    let d = e.value();
                    if d.is_empty() {
                        onchange.call(String::new());
                    } else {
                        onchange.call(format!("{d}T{h_in}:{m_in}"));
                    }
                },
            }
            select {
                value: hour,
                onchange: move |e| {
                    let h = e.value();
                    if d_hs.is_empty() {
                        onchange.call(String::new());
                    } else {
                        onchange.call(format!("{d_hs}T{h}:{m_hs}"));
                    }
                },
                for h in hours {
                    option { value: "{h}", "{h}" }
                }
            }
            span { "时" }
            span { ":" }
            select {
                value: minute,
                onchange: move |e| {
                    let m = e.value();
                    if d_ms.is_empty() {
                        onchange.call(String::new());
                    } else {
                        onchange.call(format!("{d_ms}T{h_ms}:{m}"));
                    }
                },
                for m in minutes {
                    option { value: "{m}", "{m}" }
                }
            }
            span { "分" }
        }
    }
}

/// 交易对象下拉标签：名称 (货币)
fn cp_label(c: &Counterparty, currencies: &[Currency]) -> String {
    let cur = currencies
        .iter()
        .find(|x| x.id == c.currency_id)
        .map(|x| x.name.clone())
        .unwrap_or_else(|| format!("id={}", c.currency_id));
    format!("{} ({})", c.name, cur)
}

/// 解析金额文本为定点数 Money（如 "100"、"100.50"），失败返回 None
fn parse_money(s: &str) -> Option<Money> {
    let s = s.trim();
    if s.is_empty() {
        return Some(Money::from_int(0));
    }
    let negative = s.starts_with('-');
    let digits = s.strip_prefix('-').unwrap_or(s);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let parts: Vec<&str> = digits.split('.').collect();
    if parts.len() > 2 {
        return None;
    }
    let int_part = parts[0];
    let frac_part = if parts.len() == 2 { parts[1] } else { "" };
    if int_part.is_empty() || !int_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if !frac_part.chars().all(|c| c.is_ascii_digit()) || frac_part.len() > 6 {
        return None;
    }
    let divisor = if frac_part.is_empty() { 1u32 } else { 10u32.pow(frac_part.len() as u32) };
    let int_val: i64 = int_part.parse().ok()?;
    let mut value = int_val.checked_mul(divisor as i64)?;
    if !frac_part.is_empty() {
        let frac_val: i64 = frac_part.parse().ok()?;
        value += if negative { -frac_val } else { frac_val };
    }
    if negative {
        value = -value;
    }
    Some(Money { value, divisor })
}

// ---------- 平台睡眠 ----------

async fn sleep_ms(ms: u64) {
    #[cfg(target_arch = "wasm32")]
    {
        gloo_timers::future::sleep(std::time::Duration::from_millis(ms)).await;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
    }
}

// ---------- 跨平台 HTTP ----------

async fn http_put(base: &str, path: &str, body: Option<serde_json::Value>) -> Result<serde_json::Value, String> {
    http_request("PUT", base, path, body).await
}

async fn http_post(base: &str, path: &str, body: Option<serde_json::Value>) -> Result<serde_json::Value, String> {
    http_request("POST", base, path, body).await
}

#[cfg(target_arch = "wasm32")]
async fn http_request(
    method: &str,
    base: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    use gloo_net::http::Request;

    let url = format!("{base}{path}");
    let builder = match method {
        "PUT" => Request::put(&url),
        _ => Request::post(&url),
    };
    let builder = builder.header("Content-Type", "application/json");
    // gloo_net 0.7：json/body 返回 Result<Request>，需先消费 builder
    let req = if let Some(b) = body {
        let json_str = serde_json::to_string(&b).map_err(|e| e.to_string())?;
        builder.body(json_str).map_err(|e| e.to_string())?
    } else {
        builder.build().map_err(|e| e.to_string())?
    };
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if status >= 400 {
        let err: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({ "error": text }));
        return Err(err["error"].as_str().unwrap_or("请求失败").to_string());
    }
    serde_json::from_str(&text).map_err(|e| format!("解析响应失败: {e}"))
}

#[cfg(not(target_arch = "wasm32"))]
async fn http_request(
    method: &str,
    base: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let url = format!("{base}{path}");
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true) // 自签名证书
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = match method {
        "PUT" => client.put(&url),
        _ => client.post(&url),
    };
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if status.as_u16() >= 400 {
        let err: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({ "error": text }));
        return Err(err["error"].as_str().unwrap_or("请求失败").to_string());
    }
    serde_json::from_str(&text).map_err(|e| format!("解析响应失败: {e}"))
}

async fn ping(st: &GlobalState) -> bool {
    let base = (st.base)();
    match http_put(&base, "/api/live", Some(serde_json::json!({}))).await {
        Ok(v) => v["status"].as_str() == Some("ok"),
        Err(_) => false,
    }
}

/// 秒偏移 → 时区标签："UTC"、"UTC+08"、"UTC+08:00"、"UTC-05:30"
fn utc_offset_label(offset_secs: i64) -> String {
    if offset_secs == 0 {
        return "UTC".to_string();
    }
    let sign = if offset_secs > 0 { "+" } else { "-" };
    let abs = offset_secs.unsigned_abs();
    let h = abs / 3600;
    let m = (abs % 3600) / 60;
    if m == 0 {
        format!("UTC{sign}{h}")
    } else {
        format!("UTC{sign}{h}:{m:02}")
    }
}

/// 当前本地时间（含时区标签，如 `13:50:21 UTC+08:00`）
///
/// - native：chrono::Local（系统时区）
/// - wasm：浏览器 JS Date（获取浏览器本地时区与 UTC 偏移）；取不到时显示 UTC
fn now_str() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        // wasm 下 chrono 无系统时区，用 JS Date 获取浏览器本地时间与偏移
        let date = js_sys::Date::new_0();
        let h = date.get_hours() as i32;
        let m = date.get_minutes() as i32;
        let s = date.get_seconds() as i32;
        // JS getTimezoneOffset()：本地比 UTC 快为负（如 UTC+8 返回 -480 分钟），取反转秒
        let offset_secs = -(date.get_timezone_offset() as i64) * 60;
        format!("{h:02}:{m:02}:{s:02} {}", utc_offset_label(offset_secs))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let now = chrono::Local::now();
        let t = now.format("%H:%M:%S").to_string();
        let offset_secs = now.offset().fix().local_minus_utc() as i64;
        format!("{t} {}", utc_offset_label(offset_secs))
    }
}

fn alert(msg: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window().map(|w| w.alert_with_message(msg).ok());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        log::warn!("{msg}");
    }
}

// ---------- 根组件 ----------

#[component]
pub fn App() -> Element {
    // 注意：hooks（use_signal）必须在组件函数体顶层调用，
    // 不能在 use_context_provider 的闭包内调用（违反 Rules of Hooks）
    let state = GlobalState {
        base: use_signal(|| DEFAULT_BASE.to_string()),
        connected: use_signal(|| false),
        status_msg: use_signal(String::new),
        live_ok: use_signal(|| false),
        live_at: use_signal(|| "未探测".to_string()),
        api_version: use_signal(|| "未知".to_string()),
        page: use_signal(|| Page::Records),
        counterparties: use_signal(Vec::new),
        currencies: use_signal(Vec::new),
        query_from: use_signal(String::new),
        query_to: use_signal(String::new),
        query_cp: use_signal(String::new),
        records: use_signal(Vec::new),
        new_group_name: use_signal(String::new),
        new_currency_name: use_signal(String::new),
        new_cp_name: use_signal(String::new),
        new_cp_currency: use_signal(String::new),
        new_cp_balance: use_signal(|| "0".to_string()),
        group_drafts: use_signal(|| vec![GroupDraftRow::new()]),
        stats: use_signal(Vec::new),
        stats_from: use_signal(String::new),
        stats_to: use_signal(String::new),
        stats_cp: use_signal(Vec::new),
    };
    use_context_provider(move || state);

    let st = use_context::<GlobalState>();
    if (st.connected)() {
        rsx! { MainLayout {} }
    } else {
        rsx! { ConnectPage {} }
    }
}

// ---------- 主布局（菜单栏 + 页面） ----------

#[component]
fn MainLayout() -> Element {
    let mut st = use_context::<GlobalState>();

    use_effect(move || {
        let mut st = st;
        spawn(async move {
            loop {
                let ok = ping(&st).await;
                st.live_ok.set(ok);
                st.live_at.set(now_str());
                sleep_ms(LIVE_INTERVAL_MS).await;
            }
        });
    });

    let live_ok_flag = (st.live_ok)();
    let live_text = if live_ok_flag { "● 存活" } else { "○ 离线" };
    let live_color = if live_ok_flag { "green" } else { "red" };
    let live_at_text = (st.live_at)();
    let api_version_text = (st.api_version)();

    rsx! {
        div {
            style: "display:flex;height:100vh;margin:0",
            aside { style: "width:200px;border-right:1px solid #ddd;padding:10px;display:flex;flex-direction:column",
                h2 { "记账软件" }
                div { style: "font-size:12px;color:#888;margin-bottom:8px", "版本 {api_version_text}" }
                nav { style: "display:flex;flex-direction:column;gap:8px",
                    button { onclick: move |_| st.page.set(Page::Records), "📒 交易记录" }
                    button { onclick: move |_| st.page.set(Page::Master), "🏦 对象/货币" }
                    button { onclick: move |_| st.page.set(Page::Stats), "📊 数据统计" }
                }
                div { style: "margin-top:auto;border-top:1px solid #ddd;padding-top:8px",
                    button {
                        style: "color:{live_color}",
                        onclick: move |_| st.connected.set(false),
                        "状态: {live_text} (上次 {live_at_text})"
                    }
                }
            }
            main { style: "flex:1;padding:16px;overflow:auto",
                match (st.page)() {
                    Page::Records => rsx! { RecordsPage {} },
                    Page::Master => rsx! { MasterPage {} },
                    Page::Stats => rsx! { StatsPage {} },
                }
            }
        }
    }
}

// ---------- 连接页 ----------

#[component]
fn ConnectPage() -> Element {
    let mut st = use_context::<GlobalState>();
    let base_text = (st.base)();

    rsx! {
        div { style: "height:100vh;display:flex;align-items:center;justify-content:center;flex-direction:column;gap:12px",
            h1 { "记账软件" }
            p { "输入后端地址（HTTP/2.0 HTTPS）" }
            input {
                value: base_text,
                placeholder: DEFAULT_BASE,
                oninput: move |e| st.base.set(e.value()),
            }
            button {
                onclick: move |_| {
                    let mut st = st;
                    st.status_msg.set("正在连接...".into());
                    spawn(async move {
                        let ok = ping(&st).await;
                        if ok {
                            let b = (st.base)();
                            match http_put(&b, "/api/counterparties", None).await {
                                Ok(v) => st.counterparties.set(serde_json::from_value(v).unwrap_or_default()),
                                Err(e) => { st.status_msg.set(format!("拉取交易对象失败: {e}")); return; }
                            }
                            match http_put(&b, "/api/currencies", None).await {
                                Ok(v) => st.currencies.set(serde_json::from_value(v).unwrap_or_default()),
                                Err(e) => { st.status_msg.set(format!("拉取货币失败: {e}")); return; }
                            }
                            // 拉取 API 与数据库版本号
                            match http_put(&b, "/api/version", None).await {
                                Ok(v) => {
                                    let api = v["api"].as_i64().unwrap_or(0);
                                    let dbv = v["db"].as_i64().unwrap_or(0);
                                    st.api_version.set(format!("API={api} DB={dbv}"));
                                }
                                Err(e) => { st.status_msg.set(format!("拉取版本失败: {e}")); return; }
                            }
                            st.connected.set(true);
                            st.status_msg.set("连接成功".into());
                        } else {
                            st.status_msg.set("连接失败：检查地址与证书".into());
                            alert("连接失败：后端不可达，或需先信任自签名证书");
                        }
                    });
                },
                "🔗 连接"
            }
            p { color: "#666", "{base_text}" }
            p { style: "font-size:12px;color:#999", "后端地址默认 {DEFAULT_BASE}" }
        }
    }
}

// ---------- 第一页：交易记录 ----------

#[component]
fn RecordsPage() -> Element {
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
                                Err(e) => alert(&format!("查询失败: {e}")),
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
                                    alert("组名称不能为空");
                                    return;
                                }
                                let drafts = (st.group_drafts)();
                                if drafts.is_empty() {
                                    alert("请至少添加一条记录");
                                    return;
                                }
                                let mut submits: Vec<RecordSubmit> = Vec::new();
                                for (i, d) in drafts.iter().enumerate() {
                                    if d.from_id == 0 || d.to_id == 0 || d.currency_id == 0 {
                                        alert(&format!("第 {} 行：请选择转出/转入对象与货币", i + 1));
                                        return;
                                    }
                                    if d.from_id == d.to_id {
                                        alert(&format!("第 {} 行：转出与转入不能是同一对象", i + 1));
                                        return;
                                    }
                                    let amount = match parse_money(&d.amount) {
                                        Some(m) => m,
                                        None => {
                                            alert(&format!("第 {} 行：金额格式不正确", i + 1));
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
                                        alert(&format!("组已创建 id={}", v["id"]));
                                        st.group_drafts.set(vec![GroupDraftRow::new()]);
                                        st.new_group_name.set("".into());
                                        // 余额已联动变化，刷新交易对象列表
                                        let b2 = (st.base)();
                                        if let Ok(v2) = http_put(&b2, "/api/counterparties", None).await {
                                            st.counterparties.set(serde_json::from_value(v2).unwrap_or_default());
                                        }
                                    }
                                    Err(e) => alert(&format!("创建失败: {e}")),
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
                onclick: move |_| {
                    let mut st = st;
                    let rid = rid;
                    let traded_at = traded_at.clone();
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
                            Err(e) => alert(&format!("删除失败: {e}")),
                        }
                    });
                },
                "🗑 删"
            }
            button {
                onclick: move |_| {
                    let mut st = st;
                    let gid = gid;
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
                            Err(e) => alert(&format!("删除组失败: {e}")),
                        }
                    });
                },
                "🗑 删组"
            }
        }
    }
}

// ---------- 第二页：对象/货币管理 ----------

#[component]
fn MasterPage() -> Element {
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
                                    Err(e) => alert(&format!("添加货币失败: {e}")),
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
                                    alert("初始余额格式不正确，应为数字（如 100 或 100.50）");
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
                                    Err(e) => alert(&format!("添加对象失败: {e}")),
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

    rsx! {
        div { style: "border:1px solid #ddd;padding:4px 8px;border-radius:4px;display:flex;gap:8px;align-items:center",
            span { "{cname}" }
            button {
                onclick: move |_| {
                    let mut st = st;
                    let cid = cid;
                    spawn(async move {
                        let b = (st.base)();
                        let body = serde_json::json!({ "id": cid });
                        match http_post(&b, "/api/currencies/delete", Some(body)).await {
                            Ok(_) => st.currencies.write().retain(|x| x.id != cid),
                            Err(e) => alert(&format!("删除失败: {e}")),
                        }
                    });
                },
                "✕"
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

    rsx! {
        div { style: "border:1px solid #ddd;padding:4px 8px;border-radius:4px;display:flex;gap:8px;align-items:center",
            span { "{cname}" }
            span { style: "font-size:12px;color:#888", "余额 {bal} {cur_name}" }
            button {
                onclick: move |_| {
                    let mut st = st;
                    let cid = cid;
                    spawn(async move {
                        let b = (st.base)();
                        let body = serde_json::json!({ "id": cid });
                        match http_post(&b, "/api/counterparties/delete", Some(body)).await {
                            Ok(_) => st.counterparties.write().retain(|x| x.id != cid),
                            Err(e) => alert(&format!("删除失败: {e}")),
                        }
                    });
                },
                "✕"
            }
        }
    }
}

// ---------- 第三页：数据统计 ----------

/// 统计页：按交易对象分组的汇总视图
struct ObjectGroupView {
    name: String,
    currency: String,
    balance: Money,
    /// (月份, 收入, 支出)
    monthly: Vec<(String, Option<Money>, Option<Money>)>,
}

#[component]
fn StatsPage() -> Element {
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
                                    alert("请至少选择一个交易对象");
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
                                    Err(e) => alert(&format!("查询失败: {e}")),
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
            // div { style: "border:1px solid #eee;padding:8px;border-radius:4px;display:flex;gap:8px;align-items:center",
            //     span { "更新总表统计" }
            //     button {
            //         onclick: move |_| {
            //             let st = st;
            //             spawn(async move {
            //                 let b = (st.base)();
            //                 match http_post(&b, "/api/stats/update", None).await {
            //                     Ok(v) => {
            //                         let n = v["updated"].as_array().map(|a| a.len()).unwrap_or(0);
            //                         alert(&format!("统计更新完成：{n} 个月"));
            //                     }
            //                     Err(e) => alert(&format!("更新失败: {e}")),
            //                 }
            //             });
            //         },
            //         "🔄 更新统计"
            //     }
            // }

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


// ---------- 工具 ----------
