//! 客户端根视图
//!
//! `AccountApp` 持有全部页面状态与输入框实体；具体页面渲染拆分到：
//! - `connect.rs` / `records.rs` / `master.rs` / `stats.rs`
//! - `main_layout.rs` 负责侧栏 + 内容 + 状态栏布局
//!
//! 网络请求统一通过 `http` 模块在后台线程执行，结果回到 UI 线程后
//! `cx.notify()` 触发重绘。

use super::confirm::ConfirmAction;
use super::http::{self, HttpVersion};
use super::state::{
    ObjectStatView, Page, RecordView, SortOrder, TaskProgress, now_local_iso, now_minus_7d_iso,
    now_str,
};
use super::update::{self, UpdateState};
use crate::server::models::{Counterparty, Currency};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{AppContext as _, Context, Entity, IntoElement, Render, Subscription, Window};
use serde_json::json;
use std::time::Duration;

pub(crate) const DEFAULT_BASE: &str = "https://127.0.0.2:3000";
/// 自动探测间隔：5 分钟
const LIVE_INTERVAL: Duration = Duration::from_secs(300);

/// 新建组草稿行（每条记录的可编辑状态）
pub struct GroupDraftRow {
    pub from_id: i64,
    pub to_id: i64,
    pub currency_id: i64,
    pub amount: Entity<InputState>,
    pub remark: Entity<InputState>,
    pub traded_at: Entity<InputState>,
}

/// 重命名目标（内联编辑）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameTarget {
    Currency(i64),
    Counterparty(i64),
}

/// 客户端根视图。
pub struct AccountApp {
    pub(crate) address: Entity<InputState>,
    pub(crate) http_version: HttpVersion,
    pub(crate) connected: bool,
    pub(crate) connecting: bool,
    pub(crate) show_connect: bool,
    pub(crate) status_msg: String,
    pub(crate) live_ok: bool,
    pub(crate) live_at: String,
    pub(crate) api_version: String,

    pub(crate) page: Page,
    pub(crate) counterparties: Vec<Counterparty>,
    pub(crate) currencies: Vec<Currency>,

    pub(crate) query_from: Entity<InputState>,
    pub(crate) query_to: Entity<InputState>,
    pub(crate) query_cp: i64,
    pub(crate) records: Vec<RecordView>,
    pub(crate) records_order: SortOrder,
    pub(crate) new_group_name: Entity<InputState>,
    pub(crate) group_drafts: Vec<GroupDraftRow>,

    pub(crate) new_currency_name: Entity<InputState>,
    pub(crate) new_cp_name: Entity<InputState>,
    pub(crate) new_cp_currency: i64,
    pub(crate) new_cp_balance: Entity<InputState>,
    pub(crate) cur_search: Entity<InputState>,
    pub(crate) cp_search: Entity<InputState>,
    pub(crate) renaming: Option<RenameTarget>,
    pub(crate) rename_input: Option<Entity<InputState>>,

    pub(crate) stats: Vec<ObjectStatView>,
    pub(crate) stats_from: Entity<InputState>,
    pub(crate) stats_to: Entity<InputState>,
    pub(crate) stats_cp: Vec<i64>,
    pub(crate) stats_search: Entity<InputState>,

    pub(crate) toast: Option<String>,
    pub(crate) update_state: UpdateState,
    pub(crate) task: Option<TaskProgress>,
    pub(crate) detail_open: bool,

    _subscriptions: Vec<Subscription>,
}

impl AccountApp {
    /// 构造根视图并创建所有输入框实体。
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let address = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(DEFAULT_BASE)
                .placeholder("后端地址")
        });
        let query_from = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(now_minus_7d_iso())
                .placeholder("起始时间")
        });
        let query_to = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(now_local_iso())
                .placeholder("结束时间")
        });
        let new_group_name =
            cx.new(|cx| InputState::new(window, cx).placeholder("组名称（≤64 字符）"));
        let new_currency_name =
            cx.new(|cx| InputState::new(window, cx).placeholder("货币名称，如 RMB"));
        let new_cp_name = cx.new(|cx| InputState::new(window, cx).placeholder("交易对象名称"));
        let new_cp_balance =
            cx.new(|cx| InputState::new(window, cx).placeholder("初始余额（可空）"));
        let cur_search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索货币"));
        let cp_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("搜索交易对象（名称或货币）"));
        let stats_from =
            cx.new(|cx| InputState::new(window, cx).placeholder("起始月份 YYYY-MM（可空）"));
        let stats_to =
            cx.new(|cx| InputState::new(window, cx).placeholder("结束月份 YYYY-MM（可空）"));
        let stats_search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索交易对象"));

        let mut subscriptions = Vec::new();
        for input in [
            &address,
            &query_from,
            &query_to,
            &new_group_name,
            &new_currency_name,
            &new_cp_name,
            &new_cp_balance,
            &cur_search,
            &cp_search,
            &stats_from,
            &stats_to,
            &stats_search,
        ] {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                |_: &mut Self, _: &Entity<InputState>, _: &InputEvent, _: &mut Window, cx| {
                    cx.notify();
                },
            ));
        }

        let mut app = Self {
            address,
            http_version: HttpVersion::Http2,
            connected: false,
            connecting: false,
            show_connect: true,
            status_msg: String::new(),
            live_ok: false,
            live_at: "未探测".to_string(),
            api_version: "未知".to_string(),
            page: Page::Records,
            counterparties: Vec::new(),
            currencies: Vec::new(),
            query_from,
            query_to,
            query_cp: 0,
            records: Vec::new(),
            records_order: SortOrder::NewestFirst,
            new_group_name,
            group_drafts: Vec::new(),
            new_currency_name,
            new_cp_name,
            new_cp_currency: 0,
            new_cp_balance,
            cur_search,
            cp_search,
            renaming: None,
            rename_input: None,
            stats: Vec::new(),
            stats_from,
            stats_to,
            stats_cp: Vec::new(),
            stats_search,
            toast: None,
            update_state: UpdateState::Idle,
            task: None,
            detail_open: false,
            _subscriptions: subscriptions,
        };
        app.spawn_live_loop(cx);
        app.spawn_update_check(cx);
        app
    }

    /// 当前地址（输入框值）。
    pub fn base(&self, cx: &gpui_kit::App) -> String {
        self.address.read(cx).value().trim().to_string()
    }

    /// 当前协议。
    pub fn version(&self) -> HttpVersion {
        self.http_version
    }

    /// 是否已连接。
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// 统一提示：写入状态栏，3 秒后清除。
    pub fn push_toast(&mut self, cx: &mut Context<Self>, msg: impl Into<String>) {
        let msg = msg.into();
        log::debug!("提示: {msg}");
        self.toast = Some(msg);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(3)).await;
            this.update(cx, |this, cx| {
                this.toast = None;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 断开连接：清空所有缓存，保留地址。
    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.connected = false;
        self.counterparties.clear();
        self.currencies.clear();
        self.records.clear();
        self.stats.clear();
        self.live_ok = false;
        self.live_at = "未探测".to_string();
        self.api_version = "未知".to_string();
        self.group_drafts.clear();
        self.stats_cp.clear();
        http::session::clear();
        cx.notify();
    }

    /// 启动后台更新检测（阻塞请求放在后台线程）。
    pub fn spawn_update_check(&mut self, cx: &mut Context<Self>) {
        if matches!(self.update_state, UpdateState::Checking) {
            return;
        }
        self.update_state = UpdateState::Checking;
        cx.spawn(async move |this, cx| {
            let current = env!("CARGO_PKG_VERSION").to_string();
            let state = cx
                .background_spawn(async move { update::check_now(&current) })
                .await;
            this.update(cx, |this, cx| {
                this.update_state = state;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 5 分钟存活探测循环。
    fn spawn_live_loop(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(LIVE_INTERVAL).await;
                let info = this
                    .update(cx, |this, cx| {
                        (this.connected, this.base(cx), this.http_version)
                    })
                    .ok();
                let Some((connected, base, version)) = info else {
                    break;
                };
                if !connected || base.is_empty() {
                    continue;
                }
                let ok = cx
                    .background_spawn(async move { http::ping(&base, version) })
                    .await;
                this.update(cx, |this, cx| {
                    this.live_ok = ok;
                    this.live_at = now_str();
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// 执行确认弹窗中的操作。
    pub fn perform_confirm(&mut self, action: ConfirmAction, cx: &mut Context<Self>) {
        let base = self.base(cx);
        let version = self.http_version;
        match action {
            ConfirmAction::CloseServer => {
                if base.is_empty() {
                    return;
                }
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_spawn(async move {
                            http::post(&base, "/api/close", Some(json!({})), version)
                        })
                        .await;
                    this.update(cx, |this, cx| {
                        match result {
                            Ok(_) => {
                                this.disconnect(cx);
                                this.status_msg = "服务器已关闭".to_string();
                                this.push_toast(cx, "服务器已关闭");
                            }
                            Err(e) => this.push_toast(cx, format!("关闭服务器失败: {e}")),
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            ConfirmAction::DeleteCurrency { id } => {
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_spawn(async move {
                            http::post(
                                &base,
                                "/api/currencies/delete",
                                Some(json!({ "id": id })),
                                version,
                            )
                        })
                        .await;
                    this.update(cx, |this, cx| {
                        match result {
                            Ok(_) => this.currencies.retain(|c| c.id != id),
                            Err(e) => this.push_toast(cx, format!("删除失败: {e}")),
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            ConfirmAction::DeleteCounterparty { id } => {
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_spawn(async move {
                            http::post(
                                &base,
                                "/api/counterparties/delete",
                                Some(json!({ "id": id })),
                                version,
                            )
                        })
                        .await;
                    this.update(cx, |this, cx| {
                        match result {
                            Ok(_) => this.counterparties.retain(|c| c.id != id),
                            Err(e) => this.push_toast(cx, format!("删除失败: {e}")),
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            ConfirmAction::DeleteRecord { id, traded_at } => {
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_spawn(async move {
                            http::post(
                                &base,
                                "/api/records/delete",
                                Some(json!({ "id": id, "traded_at": traded_at })),
                                version,
                            )
                        })
                        .await;
                    this.update(cx, |this, cx| {
                        match result {
                            Ok(_) => {
                                this.records.retain(|r| r.id != id);
                                this.refresh_counterparties(cx);
                            }
                            Err(e) => this.push_toast(cx, format!("删除失败: {e}")),
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            ConfirmAction::DeleteGroup { id, group_months } => {
                let filter_months = group_months.clone();
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_spawn(async move {
                            http::post(
                                &base,
                                "/api/groups/delete",
                                Some(json!({ "id": id, "group_months": group_months })),
                                version,
                            )
                        })
                        .await;
                    this.update(cx, |this, cx| {
                        match result {
                            Ok(_) => {
                                this.records.retain(|r| {
                                    r.group_id != id
                                        || r.group_months.as_deref() != filter_months.as_deref()
                                });
                                this.refresh_counterparties(cx);
                            }
                            Err(e) => this.push_toast(cx, format!("删除组失败: {e}")),
                        }
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
            ConfirmAction::Export => {
                self.start_export(cx);
            }
        }
    }

    /// 连接后端：探测 + 拉取对象/货币/版本，成功后进入主界面。
    pub fn connect(&mut self, cx: &mut Context<Self>) {
        let base = self.base(cx);
        if base.is_empty() {
            self.status_msg = "请输入后端地址".to_string();
            self.push_toast(cx, "请输入后端地址（如 https://127.0.0.2:3000）");
            return;
        }
        if self.connected {
            if http::session::active()
                .map(|(b, v)| b == base && v == self.http_version)
                .unwrap_or(false)
            {
                self.show_connect = false;
                self.status_msg = "已连接，已返回主页面".to_string();
                cx.notify();
                return;
            }
            self.disconnect(cx);
        }

        self.status_msg = "正在连接...".to_string();
        self.connecting = true;
        let version = self.http_version;
        let session_base = base.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if !http::ping(&base, version) {
                        return Err("连接失败：后端不可达，或需先信任自签名证书".to_string());
                    }
                    let cps = http::put(&base, "/api/counterparties", None, version)?;
                    let curs = http::put(&base, "/api/currencies", None, version)?;
                    let ver = http::put(&base, "/api/version", None, version)?;
                    Ok::<_, String>((cps, curs, ver))
                })
                .await;

            this.update(cx, |this, cx| {
                this.connecting = false;
                match result {
                    Ok((cps, curs, ver)) => {
                        this.counterparties = serde_json::from_value(cps).unwrap_or_default();
                        this.currencies = serde_json::from_value(curs).unwrap_or_default();
                        let api = ver["api"].as_i64().unwrap_or(0);
                        let dbv = ver["db"].as_i64().unwrap_or(0);
                        this.api_version =
                            format!("v{} API={api} DB={dbv}", env!("CARGO_PKG_VERSION"));
                        this.connected = true;
                        this.show_connect = false;
                        this.live_ok = true;
                        this.live_at = now_str();
                        this.status_msg = "已连接".to_string();
                        http::session::set(&session_base, version);
                    }
                    Err(e) => {
                        this.status_msg = e.clone();
                        this.push_toast(cx, e);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 重新拉取交易对象（余额可能随记账联动变化）。
    pub fn refresh_counterparties(&mut self, cx: &mut Context<Self>) {
        let base = self.base(cx);
        let version = self.http_version;
        if base.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(
                    async move { http::put(&base, "/api/counterparties", None, version) },
                )
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(v) => {
                        this.counterparties = serde_json::from_value(v).unwrap_or_default();
                    }
                    Err(e) => this.push_toast(cx, format!("刷新交易对象失败: {e}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 重新拉取货币。
    pub fn refresh_currencies(&mut self, cx: &mut Context<Self>) {
        let base = self.base(cx);
        let version = self.http_version;
        if base.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { http::put(&base, "/api/currencies", None, version) })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(v) => this.currencies = serde_json::from_value(v).unwrap_or_default(),
                    Err(e) => this.push_toast(cx, format!("刷新货币失败: {e}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 按当前筛选条件重新拉取记录。
    pub fn refresh_records(&mut self, cx: &mut Context<Self>) {
        let base = self.base(cx);
        let version = self.http_version;
        if base.is_empty() {
            return;
        }
        let from = self.query_from.read(cx).value().trim().to_string();
        let to = self.query_to.read(cx).value().trim().to_string();
        let cp = self.query_cp;
        let body = json!({
            "from": if from.is_empty() { None } else { Some(super::state::build_iso(&from)) },
            "to": if to.is_empty() { None } else { Some(super::state::build_iso(&to)) },
            "counterparty_id": if cp == 0 { None } else { Some(cp) },
        });
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(
                    async move { http::put(&base, "/api/records", Some(body), version) },
                )
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(v) => this.records = serde_json::from_value(v).unwrap_or_default(),
                    Err(e) => this.push_toast(cx, format!("查询失败: {e}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// 按当前月份范围与已选对象刷新统计。
    pub fn refresh_stats(&mut self, cx: &mut Context<Self>) {
        let base = self.base(cx);
        let version = self.http_version;
        if base.is_empty() {
            return;
        }
        if self.stats_cp.is_empty() {
            self.push_toast(cx, "请至少选择一个交易对象");
            return;
        }
        let from = self.stats_from.read(cx).value().trim().to_string();
        let to = self.stats_to.read(cx).value().trim().to_string();
        let ids = self.stats_cp.clone();
        let body = json!({
            "counterparty_ids": ids,
            "from": if from.is_empty() { None } else { Some(from) },
            "to": if to.is_empty() { None } else { Some(to) },
        });
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    http::put(&base, "/api/stats/objects", Some(body), version)
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(v) => this.stats = serde_json::from_value(v).unwrap_or_default(),
                    Err(e) => this.push_toast(cx, format!("统计失败: {e}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for AccountApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.show_connect || !self.connected {
            self.connect_page(window, cx).into_any_element()
        } else {
            self.main_layout(window, cx).into_any_element()
        }
    }
}

impl Drop for AccountApp {
    /// 窗口/应用退出时联动关闭后端。
    ///
    /// 关闭请求在独立线程执行，当前线程最多等待 1.5s：后端已失联时也不会卡住退出。
    fn drop(&mut self) {
        let Some((base, version)) = http::session::active() else {
            return;
        };
        log::info!("退出前关闭后端服务器: {base}");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = http::post(&base, "/api/close", Some(json!({})), version);
            let _ = tx.send(());
        });
        if rx.recv_timeout(Duration::from_millis(1500)).is_err() {
            log::warn!("关闭后端超时，直接退出");
        }
        http::session::clear();
    }
}
