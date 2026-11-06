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
//! - `ui/stats.rs`   — 数据统计（含导出入口）
//! - `ui/export.rs`  — 导出保存对话框（Web 下载 / 桌面另存为）
//! - `ui/date.rs`    — 中文日期/时间/年月选择组件（可手输 + 日历面板）
//! - `ui/update.rs`  — 启动时检测 GitHub 新版本
//!
//! 跨页公用的代码（全局状态、HTTP 封装、提示、确认弹窗等）保留在本文件。

use crate::models::{Counterparty, Currency, Money};
#[cfg(not(target_arch = "wasm32"))]
use chrono::Offset;
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

pub mod connect;
pub mod date;
pub mod export;
pub mod master;
pub mod records;
pub mod stats;
pub mod status_bar;
pub mod update;

use update::UpdateState;

// 让父模块（ui.rs）中的页面切换能直接引用子模块组件
pub(crate) use connect::ConnectPage;
pub(crate) use date::{DayTimeInput, MonthPicker};
pub(crate) use master::MasterPage;
pub(crate) use records::{RecordsPage, SortOrder};
pub(crate) use stats::StatsPage;
pub(crate) use status_bar::StatusBar;

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
    /// 转出对象 ID（导出时用于判断方向）
    pub from_id: i64,
    pub to: String,
    /// 转入对象 ID（导出时用于判断方向）
    pub to_id: i64,
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
    /// 当前余额
    pub balance: Money,
    /// 按月份升序的收支与月末结算余额
    pub monthly: Vec<MonthStatView>,
}

/// 统计页单月明细（月份 + 收支 + 该月月末结算余额）
pub(crate) struct MonthStatView {
    pub month: String,
    /// 该月收入（无记录为 None）
    pub income: Option<Money>,
    /// 该月支出（无记录为 None）
    pub expense: Option<Money>,
    /// 该月月末的结算余额（前端由当前余额向前倒推计算）
    pub settlement: Money,
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
    /// 记录列表排序方向（交易记录页标题右侧的切换按钮控制）
    pub records_order: Signal<SortOrder>,
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
    /// 轻提示（Toast）内容：在主界面底部状态栏 / 连接页内展示
    pub toast: Signal<Option<String>>,
    /// 更新检测状态（底部状态栏展示文案、颜色与点击行为）
    pub update_state: Signal<UpdateState>,
    /// 长任务进度（导出等），None 表示当前没有进行中的任务
    pub task: Signal<Option<TaskProgress>>,
}

/// 长任务进度（底部状态栏据此显示进度条）
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskProgress {
    /// 任务名称（如「导出交易记录」）
    pub label: String,
    /// 已完成步数
    pub done: usize,
    /// 总步数（0 表示进度未知，只显示任务名）
    pub total: usize,
}

impl TaskProgress {
    /// 新建进度（从 0 步开始）
    pub fn new(label: impl Into<String>, total: usize) -> Self {
        Self {
            label: label.into(),
            done: 0,
            total,
        }
    }

    /// 完成一步（不超过总步数）
    pub fn advance(&mut self) {
        if self.done < self.total {
            self.done += 1;
        }
    }

    /// 百分比（0~100）；总步数为 0（进度未知）时返回 0
    pub fn percent(&self) -> u8 {
        if self.total == 0 {
            return 0;
        }
        let done = self.done.min(self.total);
        ((done * 100) / self.total) as u8
    }

    /// 底栏文案：有总步数时形如「导出交易记录 3/5」
    pub fn describe(&self) -> String {
        if self.total == 0 {
            self.label.clone()
        } else {
            let done = self.done.min(self.total);
            format!("{} {}/{}", self.label, done, self.total)
        }
    }
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

impl Default for GroupDraftRow {
    fn default() -> Self {
        Self::new()
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

/// 取当前选中项的标签；未匹配时返回空串。
///
/// `value = 0` 且候选中含 `(0, empty_label)` 时返回该标签（如「全部」）；
/// 返回空串表示"没有可展示的选中项"，调用方应回退到传入的 placeholder。
pub(crate) fn selected_label_of(candidates: &[(i64, String)], value: i64) -> String {
    candidates
        .iter()
        .find(|(id, _)| *id == value)
        .map(|(_, label)| label.clone())
        .unwrap_or_default()
}

/// 按关键字过滤候选：标签子串匹配、忽略大小写；关键字为空（或全空白）时返回全部候选。
///
/// 只匹配标签不匹配数值：用户按对象名/货币名搜索（如「微信」）。
pub(crate) fn filter_candidates(candidates: &[(i64, String)], query: &str) -> Vec<(i64, String)> {
    let q = query.trim().to_lowercase();
    candidates
        .iter()
        .filter(|(_, label)| q.is_empty() || label.to_lowercase().contains(&q))
        .cloned()
        .collect()
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
///
/// 文字颜色：**已选项为黑色**（叠加层展示，便于阅读），**未选择时占位文本保持灰色**。
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

    // 当前选中项标签：仅用于 placeholder 展示（绝不写回输入框 value）
    let selected_label = selected_label_of(&all_candidates, value);

    // 过滤候选：按标签子串匹配（忽略大小写）
    let filtered: Vec<(i64, String)> = filter_candidates(&all_candidates, &query());
    // 输入框是否为空：为空时用黑字叠加展示已选项（灰色留给占位文本）
    let query_is_empty = (query)().trim().is_empty();

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
        div {
            style: "position:relative;display:inline-block;min-width:150px;vertical-align:middle",
            // 焦点离开本组件（点击页面其他位置）→ 收起候选并丢弃临时搜索词
            // （历史 bug：此前点击别处候选栏不会关闭）
            onfocusout: move |_| {
                open.set(false);
                query.set(String::new());
            },
            input {
                // value 只绑定用户输入，禁止回填已选标签：
                // 旧实现把选中标签写进 value，用户删除文本后又被渲染填回（历史 bug）
                value: query(),
                // 占位文本只在「没有已选项」时使用，保持灰色（浏览器默认 ::placeholder 颜色）；
                // 已选项改用下方叠加的黑字展示——placeholder 无法单独改色
                placeholder: if selected_label.is_empty() {
                    placeholder
                } else {
                    String::new()
                },
                style: "color:#000",
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
                    } else if code == "Escape" {
                        // Esc：收起候选并清空临时搜索词
                        open.set(false);
                        query.set(String::new());
                    }
                },
            }
            // 已选项以黑字叠加在输入框上：placeholder 只能是灰字，无法单独改色；
            // 只在「输入框为空」时显示，用户开始输入即让位（pointer-events:none 不挡输入）
            if query_is_empty && !selected_label.is_empty() {
                span {
                    style: "position:absolute;left:7px;top:50%;transform:translateY(-50%);color:#000;pointer-events:none;white-space:nowrap;max-width:calc(100% - 14px);overflow:hidden;text-overflow:ellipsis",
                    "{selected_label}"
                }
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
                            // 用 mousedown 而非 click：click 之前输入框会先失焦，
                            // 候选列表可能已被卸载导致本次点击丢失；prevent_default 阻止失焦
                            onmousedown: move |e| {
                                e.prevent_default();
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

/// 解析金额文本为定点数 Money（如 `100`、`100.50`、`-1.23`），失败返回 `None`
///
/// 规则：
/// - 空文本（或全空白）视为 0：`初始余额` 允许留空
/// - 只接受「可选负号 + ASCII 数字 + 至多一个小数点」，不接受正号、科学计数法、千位分隔符
/// - **不限制小数位数**：精度由文本本身决定（`divisor = 10^位数`），
///   只在 `divisor`（u32）或金额（i64）溢出时判定失败——业务上金额虽限定为正数，
///   但解析器不该用业务上限掩盖溢出检测（人工评审要求）
/// - 负数必须解析正确：先按绝对值累加，最后统一取负。
///   （早期实现先减小数部分再取负，导致 `-1.23` 被算成 `-0.77`）
pub(crate) fn parse_money(s: &str) -> Option<Money> {
    let s = s.trim();
    if s.is_empty() {
        return Some(Money::from_int(0));
    }
    // 先剥离负号：符号不参与数值累加，避免「先减小数再取负」的旧 bug
    let (negative, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    // 负号后必须有内容，且只允许数字与小数点
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let mut parts = digits.split('.');
    let int_part = parts.next().unwrap_or("");
    let frac_part = parts.next().unwrap_or("");
    // 多于一个小数点（如 `1.2.3`）非法
    if parts.next().is_some() {
        return None;
    }
    // 小数点前必须有整数部分：`.5` 不接受，界面统一要求写成 `0.5`
    if int_part.is_empty() || !int_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    // 精度：小数位数 → 10^n；n 过大导致 divisor 溢出 u32 时视为非法输入
    let divisor = if frac_part.is_empty() {
        1u32
    } else {
        10u32.checked_pow(frac_part.len() as u32)?
    };
    let int_val: i64 = int_part.parse().ok()?;
    let mut value = int_val.checked_mul(divisor as i64)?;
    if !frac_part.is_empty() {
        let frac_val: i64 = frac_part.parse().ok()?;
        value = value.checked_add(frac_val)?;
    }
    if negative {
        value = -value;
    }
    Some(Money { value, divisor })
}

// ---------- 会话状态（跨 bin 共享） ----------

/// 当前已连接的后端地址（供桌面端关闭窗口时联动关闭后端）
///
/// 桌面端 bin 需要在窗口关闭事件里调用 `POST /api/close`，但该回调不在 Dioxus
/// 组件树内、拿不到 `GlobalState`；因此由 UI 侧（根组件 `App` 的 effect）
/// 把「已连接的后端地址」写入此模块，bin 侧只读，避免两边各存一份状态。
pub mod session {
    use std::sync::Mutex;

    /// 已连接的后端地址；`None` 表示当前未连接（或已断开）
    static ACTIVE_BASE: Mutex<Option<String>> = Mutex::new(None);

    /// 取锁：互斥量被毒化（持锁线程 panic）时取回内部值，UI 状态不值得因此崩溃
    fn lock() -> std::sync::MutexGuard<'static, Option<String>> {
        ACTIVE_BASE.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 记录当前已连接的后端地址（连接成功 / 地址变化时调用）
    pub fn set_active(base: &str) {
        *lock() = Some(base.trim().to_string());
    }

    /// 当前已连接的后端地址；未连接时为 `None`
    pub fn active_base() -> Option<String> {
        lock().clone()
    }

    /// 清除会话（断开连接 / 服务器已关闭后调用）
    pub fn clear() {
        *lock() = None;
    }
}

#[cfg(test)]
mod session_tests {
    use super::session;

    /// 验证会话缓存的写入、读取与清除：
    /// UI 侧写入后 bin 侧能读到同一地址；清除后必须读不到（避免退出时误发关闭请求）
    #[test]
    fn session_should_store_and_clear_active_base() {
        session::clear();
        assert_eq!(session::active_base(), None, "初始应为未连接");

        session::set_active("https://127.0.0.2:3000");
        assert_eq!(
            session::active_base().as_deref(),
            Some("https://127.0.0.2:3000")
        );
        // 前后空白被裁剪（用户手输地址常见）
        session::set_active("  https://127.0.0.1:3001  ");
        assert_eq!(
            session::active_base().as_deref(),
            Some("https://127.0.0.1:3001")
        );

        session::clear();
        assert_eq!(session::active_base(), None);
        // 不相等断言：清除后不能残留上一次的地址
        assert_ne!(
            session::active_base().as_deref(),
            Some("https://127.0.0.1:3001")
        );
    }
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

/// GET 绝对 URL（用于 GitHub Releases 等外部接口）。
///
/// - wasm：gloo-net（浏览器禁止自定义 `User-Agent`，故只在桌面端设置）
/// - native：reqwest，按 GitHub 要求带 `User-Agent`
pub(crate) async fn http_get(url: &str) -> Result<serde_json::Value, String> {
    http_get_impl(url).await
}

#[cfg(target_arch = "wasm32")]
async fn http_get_impl(url: &str) -> Result<serde_json::Value, String> {
    use gloo_net::http::Request;

    let req = Request::get(url).header("Accept", "application/vnd.github+json");
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if status >= 400 {
        return Err(format!("HTTP {status}: {text}"));
    }
    serde_json::from_str(&text).map_err(|e| format!("解析响应失败: {e}"))
}

#[cfg(not(target_arch = "wasm32"))]
async fn http_get_impl(url: &str) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true) // 与 PUT/POST 保持一致（自签名证书场景）
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(url)
        // GitHub API 要求必须带 User-Agent
        .header(
            "User-Agent",
            concat!("account-client/", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if status.as_u16() >= 400 {
        return Err(format!("HTTP {status}: {text}"));
    }
    serde_json::from_str(&text).map_err(|e| format!("解析响应失败: {e}"))
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

/// 界面提示：统一走自建 UI（主界面底部状态栏 / 连接页内提示），3 秒后自动消失。
///
/// 不再调用浏览器原生 `alert`：原生弹窗会阻塞页面、弹出「URL says …」这类
/// 与软件无关的标题，且与自建提示重复（见任务清单「web 的 UI 问题」）。
pub(crate) fn push_toast(st: &GlobalState, msg: &str) {
    let mut st = *st; // GlobalState 是 Copy，解引用复制一份供闭包移动
    st.toast.set(Some(msg.to_string()));
    spawn(async move {
        sleep_ms(3000).await;
        st.toast.set(None);
    });
}

// ---------- 外部链接 ----------

/// 是否允许打开的外部链接：仅 `https://`，且不含空白（发布页地址来自 GitHub API，
/// 这里做白名单校验，避免把任意字符串交给系统命令行执行）。
pub(crate) fn is_safe_external_url(url: &str) -> bool {
    let url = url.trim();
    url.len() > "https://".len()
        && url.starts_with("https://")
        && !url.contains(char::is_whitespace)
}

/// 打开外部页面：桌面用系统默认浏览器，Web 开新标签页。
pub(crate) fn open_external(url: &str) -> Result<(), String> {
    open_external_impl(url)
}

#[cfg(target_arch = "wasm32")]
fn open_external_impl(url: &str) -> Result<(), String> {
    if !is_safe_external_url(url) {
        return Err("链接不合法，仅支持 https".to_string());
    }
    let window = web_sys::window().ok_or_else(|| "无法访问页面窗口".to_string())?;
    window
        .open_with_url_and_target(url, "_blank")
        .map_err(|e| format!("打开链接失败: {e:?}"))?
        .ok_or_else(|| "浏览器拦截了新窗口，请允许弹出窗口".to_string())?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn open_external_impl(url: &str) -> Result<(), String> {
    if !is_safe_external_url(url) {
        return Err("链接不合法，仅支持 https".to_string());
    }
    #[cfg(target_os = "windows")]
    let spawned = {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW：避免打开浏览器时闪出一个黑色控制台窗口
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
    };
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("open").arg(url).spawn();
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let spawned = std::process::Command::new("xdg-open").arg(url).spawn();
    spawned
        .map(|_| ())
        .map_err(|e| format!("打开浏览器失败: {e}"))
}

// ---------- 后台阻塞任务 ----------

/// 在阻塞线程池执行 CPU / IO 密集任务（xlsx 生成、写文件），避免卡住 UI 线程。
///
/// - native：`tokio::task::spawn_blocking`（Dioxus 桌面端是多线程 tokio 运行时）
/// - wasm：浏览器只有单线程，直接同步执行
pub(crate) async fn run_blocking<F, T>(task: F) -> Result<T, String>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    run_blocking_impl(task).await
}

#[cfg(not(target_arch = "wasm32"))]
async fn run_blocking_impl<F, T>(task: F) -> Result<T, String>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|e| format!("后台任务失败: {e}"))
}

#[cfg(target_arch = "wasm32")]
async fn run_blocking_impl<F, T>(task: F) -> Result<T, String>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    Ok(task())
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
    // 全局 CSS 重置（人工排查修复的 UI bug）：
    // 浏览器默认给 html/body 加 8px margin，叠加 height:100% 后造成错误的高度占用，
    // 表现为页面四周空白 + 多余滚动条，故在根组件统一清除默认边距并改用 border-box 盒模型。
    // 连接页与主布局是两个互斥返回的 rsx 分支，需各自注入一次，否则漏掉的分支仍会残留默认边距。
    const GLOBAL_CSS: &str = "
        html, body { margin: 0; padding: 0; height: 100%; overflow: hidden; }
        *, *::before, *::after { box-sizing: border-box; }
    ";

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
        // 默认从新到旧：最新记录排在最上面
        records_order: use_signal(|| SortOrder::NewestFirst),
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
        update_state: use_signal(|| UpdateState::Idle),
        task: use_signal(|| None),
    };
    use_context_provider(move || state);

    let st = use_context::<GlobalState>();
    // 启动时检测一次 GitHub 是否发布了新版本；use_hook 只在 App 挂载时执行一次，
    // 之后无论怎么切换页面/重渲染都不会自动重试（重试只能由状态栏点击触发）
    use_hook(move || update::spawn_check(st));
    // 把「连接状态 + 后端地址」同步到跨 bin 的会话缓存：
    // 桌面端窗口关闭时 bin 侧据此决定是否调用 POST /api/close
    use_effect(move || {
        if (st.connected)() {
            session::set_active(&(st.base)());
        } else {
            session::clear();
        }
    });

    rsx! {
        style { {GLOBAL_CSS} }
        // 站点图标（图标资产由 tools/make_icons.py 生成，dx 会把 public/ 拷到输出根）
        document::Link { rel: "icon", href: "/favicon.ico" }
        document::Link { rel: "apple-touch-icon", href: "/apple-touch-icon-180.png" }
        // 显示连接页的情形：尚未连接，或用户主动从主界面切回连接页（保持 connected 不变）
        if (st.show_connect)() || !(st.connected)() {
            ConnectPage {}
        } else {
            MainLayout {}
        }
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

    rsx! {
        div {
            // 使用 100dvh 避免桌面端/网页端出现多余滚动条；
            // 最外层纵向布局：内容行（侧栏 + main）占满剩余高度，底部状态栏固定高度
            style: "display:flex;flex-direction:column;height:100dvh;margin:0;overflow:hidden",
            div { style: "display:flex;flex:1;min-height:0;overflow:hidden",
                aside { style: "width:200px;border-right:1px solid #ddd;padding:10px;display:flex;flex-direction:column;gap:8px;flex-shrink:0",
                    h2 { style: "margin:0", "菜单栏" }
                    // 版本（API/DB）不在此显示：底部状态栏的「后端连接详情」里已有，避免重复
                    nav { style: "display:flex;flex-direction:column;gap:8px",
                        button { onclick: move |_| st.page.set(Page::Records), "📒 交易记录" }
                        button { onclick: move |_| st.page.set(Page::Master), "🏦 对象/货币" }
                        button { onclick: move |_| st.page.set(Page::Stats), "📊 数据统计" }
                    }
                    // 页脚按钮区：margin-top:auto 贴到菜单栏最下方，
                    // column-reverse 让按钮从下往上排（与顶部菜单拉开距离，避免误触）
                    div { style: "margin-top:auto;display:flex;flex-direction:column-reverse;gap:8px",
                        button {
                            style: "background:#f0f0f0;border:1px solid #ccc;color:#333;padding:6px 10px;border-radius:4px;cursor:pointer;text-align:left",
                            title: "返回连接页（不断开连接）",
                            onclick: move |_| st.show_connect.set(true),
                            "↩ 返回连接页"
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
            }
            // 底部状态栏：后端连接状态 / 更新检测 / 提示信息与任务进度
            StatusBar {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TaskProgress, filter_candidates, is_safe_external_url, parse_money, selected_label_of,
    };
    use crate::models::Money;

    /// 候选项集合（含「全部」项，值 0）
    fn candidates() -> Vec<(i64, String)> {
        vec![
            (0, "全部".to_string()),
            (1, "支付宝 (RMB)".to_string()),
            (2, "微信钱包 (RMB)".to_string()),
            (3, "Bank (USD)".to_string()),
        ]
    }

    /// 验证选中项标签：`value = 0` 取「全部」，未匹配返回空串（调用方回退 placeholder）
    #[test]
    fn selected_label_should_return_matched_label_or_empty() {
        let all = candidates();
        assert_eq!(selected_label_of(&all, 0), "全部");
        assert_eq!(selected_label_of(&all, 2), "微信钱包 (RMB)");
        // 未匹配：空串，前端回退到传入的 placeholder，不会显示错误标签
        assert_eq!(selected_label_of(&all, 999), "");
        assert_eq!(selected_label_of(&[], 0), "");
    }

    /// 验证关键字过滤：空/全空白关键字返回全部候选（下拉默认展示所有项）
    #[test]
    fn filter_candidates_should_return_all_on_empty_query() {
        let all = candidates();
        assert_eq!(filter_candidates(&all, "").len(), all.len());
        assert_eq!(filter_candidates(&all, "   ").len(), all.len());
    }

    /// 验证关键字过滤：标签子串匹配且忽略大小写（中英文混排标签同样适用）
    #[test]
    fn filter_candidates_should_match_label_case_insensitively() {
        let all = candidates();
        let hit = filter_candidates(&all, "微信");
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].0, 2);

        // 大小写不敏感：bank / BANK 均命中 (3, "Bank (USD)")
        assert_eq!(filter_candidates(&all, "bank")[0].0, 3);
        assert_eq!(filter_candidates(&all, "BANK")[0].0, 3);
        // 关键字带空格也能命中（trim 后按子串匹配）
        assert_eq!(filter_candidates(&all, " 支付宝 ")[0].0, 1);
    }

    /// 验证关键字过滤：无匹配时返回空集合（下拉展示「无匹配候选」）
    #[test]
    fn filter_candidates_should_return_empty_when_no_match() {
        let all = candidates();
        assert!(filter_candidates(&all, "不存在的对象").is_empty());
        assert!(filter_candidates(&[], "支付宝").is_empty());
    }

    /// 验证任务进度：百分比按步数计算，总数未知时为 0，步数不会超过总数
    #[test]
    fn task_progress_should_report_percent_and_steps() {
        let mut task = TaskProgress::new("导出交易记录", 4);
        assert_eq!(task.percent(), 0);
        assert_eq!(task.describe(), "导出交易记录 0/4");
        task.advance();
        task.advance();
        assert_eq!(task.percent(), 50);
        assert_eq!(task.describe(), "导出交易记录 2/4");
        // 超额推进不会让百分比超过 100
        for _ in 0..10 {
            task.advance();
        }
        assert_eq!(task.percent(), 100);
        assert_eq!(task.describe(), "导出交易记录 4/4");

        // 总步数为 0（进度未知）：只显示任务名，不出现 NaN/除零
        let unknown = TaskProgress::new("生成文件", 0);
        assert_eq!(unknown.percent(), 0);
        assert_eq!(unknown.describe(), "生成文件");
    }

    /// 验证外部链接白名单：仅接受非空 https 地址，拒绝 http/javascript/带空白等可疑输入
    #[test]
    fn external_url_should_only_allow_https() {
        assert!(is_safe_external_url(
            "https://github.com/a/b/releases/tag/0.5.0"
        ));
        assert!(is_safe_external_url("  https://example.com/x  "));
        assert!(!is_safe_external_url(""));
        assert!(!is_safe_external_url("https://"));
        assert!(!is_safe_external_url("http://example.com"));
        assert!(!is_safe_external_url("javascript:alert(1)"));
        assert!(!is_safe_external_url("https://example.com/a b"));
    }

    /// 验证金额文本解析的合法性：正负整数/小数、空文本视为 0，且不限制小数位数
    /// （人工评审要求：金额业务上限定为正数，但解析器必须能正确解析负号与长小数）
    #[test]
    fn parse_money_should_parse_sign_and_decimals() {
        assert_eq!(parse_money("100"), Some(Money::from_int(100)));
        assert_eq!(parse_money("-100"), Some(Money::from_int(-100)));
        assert_eq!(parse_money("1.23"), Some(Money::new(123, 100).unwrap()));
        assert_eq!(parse_money("-1.23"), Some(Money::new(-123, 100).unwrap()));
        // 整数部分为 0 的小额负数：符号与补零都必须正确（旧实现会算成 -0.77 这类错误值）
        assert_eq!(parse_money("-0.05"), Some(Money::new(-5, 100).unwrap()));
        // 6 位以上小数不再被拒绝：精度由文本决定
        assert_eq!(
            parse_money("0.0000001"),
            Some(Money::new(1, 10_000_000).unwrap())
        );
        // 空文本 / 全空白视为 0（「初始余额」允许留空）
        assert_eq!(parse_money(""), Some(Money::from_int(0)));
        assert_eq!(parse_money("   "), Some(Money::from_int(0)));
        // 前后空白会被裁剪
        assert_eq!(parse_money("  -12.5 "), Some(Money::new(-125, 10).unwrap()));

        // 不相等断言：解析结果不能与错误的金额相等（防止符号/精度实现错误）
        assert_ne!(parse_money("-1.23"), Some(Money::new(-77, 100).unwrap()));
        assert_ne!(parse_money("1.23"), Some(Money::new(1230, 100).unwrap()));
        assert_ne!(parse_money("-100"), Some(Money::from_int(100)));
    }

    /// 验证金额文本解析的非法输入：非数字字符、多个小数点、缺整数部分、
    /// 以及精度/数值溢出（不靠业务上限拒绝，只靠溢出检测拒绝）
    #[test]
    fn parse_money_should_reject_invalid_and_overflowing_input() {
        assert_eq!(parse_money("abc"), None);
        assert_eq!(parse_money("1.2.3"), None);
        assert_eq!(parse_money("-"), None);
        assert_eq!(parse_money("."), None);
        // 缺整数部分：界面统一要求写成 0.5
        assert_eq!(parse_money(".5"), None);
        assert_eq!(parse_money("1,000"), None);
        assert_eq!(parse_money("+1"), None);
        assert_eq!(parse_money("1e3"), None);
        // 小数位数过多导致 divisor（u32）溢出
        assert_eq!(parse_money("0.0000000001"), None);
        // 整数部分超出 i64
        assert_eq!(parse_money("99999999999999999999"), None);
        // 数值乘以精度后溢出 i64
        assert_eq!(parse_money("92233720368547758.08"), None);
    }
}
