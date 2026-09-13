//! 客户端 UI（Dioxus 页面）— 公共父模块
//!
//! 与平台无关：native（桌面）与 wasm（Web）共用同一份 UI 组件。
//! HTTP 层通过 cfg 分支：native 用 reqwest（跳过自签名证书验证），
//! wasm 用 gloo-net。定时 sleep 同样 cfg 分支。
//!
//! 每页 UI 分别存储于 `ui/` 文件夹（与 `db.rs + db/month.rs` 同风格）：
//! - `ui/connect.rs` — 连接页（含关闭服务器按钮）
//! - `ui/records.rs` — 交易记录浏览器
//! - `ui/master.rs`  — 交易对象/货币管理
//! - `ui/stats.rs`   — 数据统计
//!
//! 跨页公用的代码（全局状态、HTTP 封装、提示、确认弹窗等）保留在本文件。

use crate::models::{Counterparty, Currency, Money};
#[cfg(not(target_arch = "wasm32"))]
use chrono::Offset;
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

pub mod connect;
pub mod master;
pub mod records;
pub mod stats;

// 让父模块（ui.rs）中的页面切换能直接引用子模块组件
pub(crate) use connect::ConnectPage;
pub(crate) use master::MasterPage;
pub(crate) use records::RecordsPage;
pub(crate) use stats::StatsPage;

pub(crate) const DEFAULT_BASE: &str = "https://127.0.0.2:3000";
pub(crate) const LIVE_INTERVAL_MS: u64 = 300_000; // 5 分钟

// ---------- 序列化视图 ----------

/// 单条交易记录视图（后端 /api/records 返回）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordView {
    pub id: i64,
    pub group_id: i64,
    pub group_name: Option<String>,
    /// 组包含的所有月份（逗号分隔，如 `2026-08,2026-09`）
    pub group_months: Option<String>,
    pub from: String,
    pub to: String,
    pub amount: Money,
    pub currency: String,
    pub remark: Option<String>,
    pub traded_at: String,
}

/// 对象统计视图（后端 /api/stats/objects 返回）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ObjectStatView {
    pub month: String,
    pub objects: Vec<ObjectStatItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ObjectStatItem {
    pub id: i64,
    pub name: String,
    pub direction: String,
    pub currency: String,
    pub amount: Money,
}

/// 统计页分组视图
pub(crate) struct ObjectGroupView {
    pub name: String,
    pub currency: String,
    pub balance: Money,
    /// (月份, 收入, 支出)
    pub monthly: Vec<(String, Option<Money>, Option<Money>)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    Records,
    Master,
    Stats,
}

// ---------- 全局状态 ----------

#[derive(Clone, Copy)]
pub(crate) struct GlobalState {
    pub base: Signal<String>,
    /// 是否与服务器保持连接（决定关闭按钮可用性）
    pub connected: Signal<bool>,
    /// 是否显示连接页视图（与 connected 解耦，可返回连接页查看而不断开）
    pub show_connect: Signal<bool>,
    pub status_msg: Signal<String>,
    pub live_ok: Signal<bool>,
    pub live_at: Signal<String>,
    pub api_version: Signal<String>,
    pub page: Signal<Page>,
    pub counterparties: Signal<Vec<Counterparty>>,
    pub currencies: Signal<Vec<Currency>>,
    pub query_from: Signal<String>,
    pub query_to: Signal<String>,
    pub query_cp: Signal<String>,
    pub records: Signal<Vec<RecordView>>,
    pub new_group_name: Signal<String>,
    pub new_currency_name: Signal<String>,
    pub new_cp_name: Signal<String>,
    pub new_cp_currency: Signal<String>,
    pub new_cp_balance: Signal<String>,
    pub group_drafts: Signal<Vec<GroupDraftRow>>,
    pub stats: Signal<Vec<ObjectStatView>>,
    pub stats_from: Signal<String>,
    pub stats_to: Signal<String>,
    pub stats_cp: Signal<Vec<i64>>,
    /// 轻提示（Toast）内容
    pub toast: Signal<Option<String>>,
}

/// 新建组草稿行（表单编辑用）
#[derive(Debug, Clone, PartialEq)]
pub struct GroupDraftRow {
    pub from_id: i64,
    pub to_id: i64,
    pub amount: String,
    pub currency_id: i64,
    pub remark: String,
    pub traded_at: String,
}

impl GroupDraftRow {
    pub fn new() -> Self {
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
pub(crate) fn now_local_iso() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        chrono::Utc::now().format("%Y-%m-%dT%H:%M").to_string()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        chrono::Local::now().format("%Y-%m-%dT%H:%M").to_string()
    }
}

/// 当前时间往前 7 天（datetime-local 格式，无秒）
///
/// 用作交易记录页默认查询起始时间（近 7 天范围）。
pub(crate) fn now_minus_7d_iso() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        (chrono::Utc::now() - chrono::Duration::days(7))
            .format("%Y-%m-%dT%H:%M")
            .to_string()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        (chrono::Local::now() - chrono::Duration::days(7))
            .format("%Y-%m-%dT%H:%M")
            .to_string()
    }
}

/// datetime-local 值转后端 ISO 字符串（补秒）
pub(crate) fn build_iso(dt: &str) -> String {
    let dt = dt.trim();
    if dt.len() == 16 {
        format!("{dt}:00")
    } else {
        dt.to_string()
    }
}

/// 后端 ISO 时间转为 datetime-local 显示值（去掉秒）
pub(crate) fn dt_local(v: &str) -> String {
    if v.len() >= 16 {
        v[..16].to_string()
    } else {
        v.to_string()
    }
}

/// 24 小时制时间输入：日期 + 时(00-23) + 分(00-59)
///
/// 浏览器原生 `datetime-local` 在部分系统（如 Windows）按 12 小时制（AM/PM）
/// 显示；此组件改用 `type=date` + 时/分下拉框，强制 24 小时制。
/// `value` 约定为 `YYYY-MM-DDTHH:MM`（无秒），变更时通过 `onchange` 回传。
#[component]
pub(crate) fn DayTimeInput(value: String, onchange: EventHandler<String>) -> Element {
    let date = if value.len() >= 10 {
        value[..10].to_string()
    } else {
        String::new()
    };
    let hour = if value.len() >= 13 {
        value[11..13].to_string()
    } else {
        "00".to_string()
    };
    let minute = if value.len() >= 16 {
        value[14..16].to_string()
    } else {
        "00".to_string()
    };
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

/// 搜索 + 候选选择组件
///
/// 原生 `<select>` 不支持搜索，此组件提供输入框关键字过滤 + 候选列表选择：
/// - 点击/聚焦输入框展开候选下拉
/// - 输入关键字实时过滤候选（按标签子串匹配，忽略大小写）
/// - 支持键盘操作：`↑`/`↓` 移动高亮，`Enter` 确认选择
/// - 点击候选选中并回填，关闭下拉
/// - 未匹配时显示「无匹配候选」
///
/// `candidates` 为 `(值, 标签)` 列表；`value` 为当前选中值；
/// `empty_label` 提供时，候选顶部插入 `(0, empty_label)` 项（用于「全部」筛选）；
/// 选择后回调 `onchange`。
#[component]
pub(crate) fn SearchSelect(
    candidates: Vec<(i64, String)>,
    value: i64,
    placeholder: String,
    empty_label: Option<String>,
    onchange: EventHandler<i64>,
) -> Element {
    // 下拉是否展开
    let mut open = use_signal(|| false);
    // 搜索文本
    let mut query = use_signal(String::new);
    // 键盘高亮索引
    let mut highlight = use_signal(|| 0usize);

    // 完整候选：可选在顶部插入「全部」项（值 0）
    let mut all_candidates: Vec<(i64, String)> = Vec::new();
    if let Some(label) = empty_label {
        all_candidates.push((0, label));
    }
    all_candidates.extend(candidates);

    // 当前选中项标签（用于输入框展示；搜索中则展示搜索词）
    let selected_label = all_candidates
        .iter()
        .find(|(id, _)| *id == value)
        .map(|(_, label)| label.clone())
        .unwrap_or_default();

    // 过滤候选：按标签子串匹配（忽略大小写）
    let q = query().to_lowercase();
    let filtered: Vec<(i64, String)> = all_candidates
        .iter()
        .filter(|(_, label)| q.is_empty() || label.to_lowercase().contains(&q))
        .cloned()
        .collect();

    // 高亮索引（渲染时收敛到有效范围，避免过滤后越界）
    let highlight_idx = {
        let h = (highlight)();
        if filtered.is_empty() {
            0
        } else {
            h.min(filtered.len() - 1)
        }
    };
    let filtered_len = filtered.len();
    // 键盘闭包需要 filtered 的副本：读取选中项
    let filtered_for_keys = filtered.clone();

    rsx! {
        div { style: "position:relative;display:inline-block;min-width:150px;vertical-align:middle",
            input {
                value: if query().is_empty() { selected_label } else { query() },
                placeholder: placeholder,
                oninput: move |e| {
                    query.set(e.value());
                    open.set(true);
                },
                onclick: move |_| open.set(true),
                onfocus: move |_| open.set(true),
                onkeydown: move |e| {
                    let code = e.code().to_string();
                    if code == "ArrowDown" || code == "ArrowUp" {
                        e.prevent_default();
                        if filtered_len == 0 { return; }
                        let cur = (highlight)().min(filtered_len - 1);
                        let next = if code == "ArrowDown" {
                            (cur + 1) % filtered_len
                        } else {
                            (cur + filtered_len - 1) % filtered_len
                        };
                        highlight.set(next);
                    } else if code == "Enter" {
                        e.prevent_default();
                        if filtered_len == 0 { return; }
                        let cur = (highlight)().min(filtered_len - 1);
                        if let Some((id, _)) = filtered_for_keys.get(cur) {
                            onchange.call(*id);
                        }
                        query.set(String::new());
                        open.set(false);
                    }
                },
            }
            if open() {
                div {
                    style: "position:absolute;top:100%;left:0;right:0;background:#fff;border:1px solid #ccc;border-radius:4px;max-height:200px;overflow:auto;z-index:500;box-shadow:0 2px 8px rgba(0,0,0,0.15)",
                    for (idx, (id, label)) in filtered.iter().cloned().enumerate() {
                        div {
                            style: format!(
                                "padding:4px 8px;cursor:pointer;background:{}",
                                if idx == highlight_idx || id == value { "#eef" } else { "#fff" }
                            ),
                            onclick: move |_| {
                                onchange.call(id);
                                query.set(String::new());
                                open.set(false);
                            },
                            onmouseover: move |_| highlight.set(idx),
                            "{label}"
                        }
                    }
                    if filtered.is_empty() {
                        div { style: "padding:4px 8px;color:#999", "无匹配候选" }
                    }
                }
            }
        }
    }
}

/// 交易对象下拉标签：名称 (货币)
pub(crate) fn cp_label(c: &Counterparty, currencies: &[Currency]) -> String {
    let cur = currencies
        .iter()
        .find(|x| x.id == c.currency_id)
        .map(|x| x.name.clone())
        .unwrap_or_else(|| format!("id={}", c.currency_id));
    format!("{} ({})", c.name, cur)
}

/// 解析金额文本为定点数 Money（如 "100"、"100.50"），失败返回 None
pub(crate) fn parse_money(s: &str) -> Option<Money> {
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
    let divisor = if frac_part.is_empty() {
        1u32
    } else {
        10u32.pow(frac_part.len() as u32)
    };
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

pub(crate) async fn sleep_ms(ms: u64) {
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
//
// 平台差异（web vs 桌面）通过 #[cfg(target_arch = "wasm32")] 条件编译封装：
// - native（桌面）：reqwest（跳过自签名证书验证）
// - wasm（Web）：gloo-net
// 上层 UI 无需感知差异，统一调用 http_put / http_post。

pub(crate) async fn http_put(
    base: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    http_request("PUT", base, path, body).await
}

pub(crate) async fn http_post(
    base: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
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

pub(crate) async fn ping(st: &GlobalState) -> bool {
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
pub(crate) fn now_str() -> String {
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

/// 界面提示：桌面端与 Web 端都能看到。
///
/// - Web 端：额外调用浏览器原生 alert（与原行为一致）
/// - 桌面端：通过 Toast 浮层展示（原生对话框需额外依赖，统一走 UI 内提示）
pub(crate) fn push_toast(st: &GlobalState, msg: &str) {
    let mut st = *st; // GlobalState 是 Copy，解引用复制一份供闭包移动
    st.toast.set(Some(msg.to_string()));
    spawn(async move {
        sleep_ms(3000).await;
        st.toast.set(None);
    });
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window().map(|w| w.alert_with_message(msg).ok());
    }
}

// ---------- 通用确认弹窗 ----------
//
// 防呆设计：删除记录/删除组/关闭服务器等危险操作，先弹出确认框，
// 列出将要删除的内容，用户确认后才真正执行。

/// 确认弹窗组件：`open` 控制显隐；确认后回调 `on_confirm`
#[component]
pub(crate) fn ConfirmDialog(
    title: String,
    message: String,
    mut open: Signal<bool>,
    on_confirm: EventHandler<()>,
) -> Element {
    if !(open)() {
        return rsx! {};
    }
    rsx! {
        div {
            style: "position:fixed;inset:0;background:rgba(0,0,0,0.45);display:flex;align-items:center;justify-content:center;z-index:1000",
            div {
                style: "background:#fff;padding:16px 20px;border-radius:8px;max-width:80vw;max-height:80vh;overflow:auto;box-shadow:0 4px 16px rgba(0,0,0,0.3)",
                h4 { style: "margin:0 0 8px 0", "{title}" }
                div { style: "white-space:pre-wrap;color:#333;margin-bottom:12px", "{message}" }
                div { style: "display:flex;gap:8px;justify-content:flex-end",
                    button {
                        onclick: move |_| open.set(false),
                        "取消"
                    }
                    button {
                        style: "background:#c0392b;color:#fff;border:none;padding:4px 12px;border-radius:4px;cursor:pointer",
                        onclick: move |_| {
                            open.set(false);
                            on_confirm.call(());
                        },
                        "确认"
                    }
                }
            }
        }
    }
}

// ---------- 根组件 ----------

#[component]
pub fn App() -> Element {
    const GLOBAL_CSS: &str = "
        html, body { margin: 0; padding: 0; height: 100%; overflow: hidden; }
        *, *::before, *::after { box-sizing: border-box; }
    ";// 人工修复了一个UIbug,用于修复错误的高度占用所导致的UI bug。之前没有清除浏览器默认的 html 和 body 标签的 8px margin

    // 注意：hooks（use_signal）必须在组件函数体顶层调用，
    // 不能在 use_context_provider 的闭包内调用（违反 Rules of Hooks）
    let state = GlobalState {
        base: use_signal(|| DEFAULT_BASE.to_string()),
        connected: use_signal(|| false),
        show_connect: use_signal(|| false),
        status_msg: use_signal(String::new),
        live_ok: use_signal(|| false),
        live_at: use_signal(|| "未探测".to_string()),
        api_version: use_signal(|| "未知".to_string()),
        page: use_signal(|| Page::Records),
        counterparties: use_signal(Vec::new),
        currencies: use_signal(Vec::new),
        // 默认查询范围：近 7 天（起始 = 当前时间往前 7 天，结束 = 当前时间）
        query_from: use_signal(now_minus_7d_iso),
        query_to: use_signal(now_local_iso),
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
        toast: use_signal(|| None),
    };
    use_context_provider(move || state);

    let st = use_context::<GlobalState>();
    // 显示连接页的情形：尚未连接，或用户主动从主界面切回连接页（保持 connected 不变）
    if (st.show_connect)() || !(st.connected)() {
        rsx! {
        style { {GLOBAL_CSS} }
            ConnectPage {}
        }
    } else {
        rsx! { style { {GLOBAL_CSS} }
        MainLayout {} }
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
    let live_text = if live_ok_flag {
        "● 存活"
    } else {
        "○ 离线"
    };
    let live_color = if live_ok_flag { "green" } else { "red" };
    let live_at_text = (st.live_at)();
    let api_version_text = (st.api_version)();
    let toast_msg = (st.toast)();

    rsx! {
        div {
            // 使用 100dvh 避免桌面端/网页端出现多余滚动条；
            // 最外层禁止滚动，内容区由 main 自行滚动
            style: "display:flex;height:100dvh;margin:0;overflow:hidden",
            aside { style: "width:200px;border-right:1px solid #ddd;padding:10px;display:flex;flex-direction:column;flex-shrink:0",
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
                        // 仅切回连接页视图，不断开与服务器的连接
                        onclick: move |_| st.show_connect.set(true),
                        "状态: {live_text} (上次 {live_at_text})"
                    }
                }
            }
            main { style: "flex:1;padding:16px;overflow:auto;min-width:0",
                match (st.page)() {
                    Page::Records => rsx! { RecordsPage {} },
                    Page::Master => rsx! { MasterPage {} },
                    Page::Stats => rsx! { StatsPage {} },
                }
            }
            if let Some(msg) = &toast_msg {
                div {
                    style: "position:fixed;top:16px;left:50%;transform:translateX(-50%);background:#333;color:#fff;padding:10px 18px;border-radius:6px;z-index:2000;box-shadow:0 2px 8px rgba(0,0,0,0.3)",
                    "{msg}"
                }
            }
        }
    }
}
