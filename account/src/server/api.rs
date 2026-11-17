//! HTTP API 层
//!
//! 非 RESTful 规范：
//! - `PUT` 负责所有读类操作
//! - `POST` 负责其余所有操作（增/删/改/其他）
//!
//! 通信协议可用 HTTP/2（axum-server TLS）或 HTTP/3（QUIC，见 bin 的
//! `--http-version`）；两种协议共用同一套 handler。

use crate::server::db;
use crate::server::db::month;
use crate::server::models::{
    Counterparty, CounterpartyMonthlyQuery, CounterpartySubmit, Currency, CurrencySubmit,
    DeleteRequest, GroupSubmit, Money, MonthQuery, ObjectStatsQuery, RangeQuery, Record,
    RecordSubmit, RenameRequest,
};
use anyhow::Result as AnyhowResult;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{post, put},
};
use rusqlite::Connection;
use serde_json::json;
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

/// API 当前版本（v2：新增重命名与对象月度明细接口）
pub const API_VERSION: i64 = 2;

/// 协议无关的服务器关闭句柄。
///
/// HTTP/2 使用 `axum_server::Handle`；HTTP/3（QUIC）没有对应的 handle，
/// 因此统一用 `Notify` 通知服务器主循环退出，`axum` 句柄存在时再一并触发。
#[derive(Clone)]
pub struct Shutdown {
    notify: Arc<tokio::sync::Notify>,
    axum: Option<axum_server::Handle<SocketAddr>>,
}

impl Shutdown {
    /// HTTP/2 服务器：同时持有 axum 的优雅关闭句柄。
    pub fn for_http2(handle: axum_server::Handle<SocketAddr>) -> Self {
        Self {
            notify: Arc::new(tokio::sync::Notify::new()),
            axum: Some(handle),
        }
    }

    /// HTTP/3 服务器：只使用 `Notify` 通知主循环。
    pub fn for_http3() -> Self {
        Self {
            notify: Arc::new(tokio::sync::Notify::new()),
            axum: None,
        }
    }

    /// 请求关闭（幂等）。`notify_one` 会保存一次许可，先 close 后 wait 也能收到。
    pub fn request(&self) {
        self.notify.notify_one();
        if let Some(handle) = &self.axum {
            handle.shutdown();
        }
    }

    /// 等待关闭信号（服务器主循环 `select!` 使用）。
    pub async fn wait(&self) {
        self.notify.notified().await;
    }
}

impl Default for Shutdown {
    fn default() -> Self {
        Self::for_http3()
    }
}

/// 共享数据库连接状态
#[derive(Clone)]
pub struct AppState {
    pub conn: Arc<Mutex<Connection>>,
    /// 服务器关闭句柄：`POST /api/close` 触发优雅关闭
    pub shutdown: Shutdown,
}

/// 统一错误响应
#[derive(Debug)]
pub struct ApiError(StatusCode, String);

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        log::error!("API 错误: {err:#}");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({ "error": self.1 });
        (self.0, Json(body)).into_response()
    }
}

/// 成功响应包装
fn ok_json<T: serde::Serialize>(value: T) -> Response {
    Json(value).into_response()
}

/// 构建路由
pub fn router(state: AppState) -> Router {
    use tower_http::cors::CorsLayer;

    let cors = CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods([
            axum::http::Method::PUT,
            axum::http::Method::POST,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers(tower_http::cors::Any);

    Router::new()
        // 基础接口
        .route("/api/live", put(live))
        .route("/api/help", put(help))
        // 关闭
        .route("/api/close", post(close))
        // 增（POST）
        .route("/api/currencies", post(add_currency))
        .route("/api/counterparties", post(add_counterparty))
        .route("/api/groups", post(add_group))
        .route("/api/groups/{group_id}/records", post(add_record))
        // 改（POST）
        .route("/api/currencies/rename", post(rename_currency))
        .route("/api/counterparties/rename", post(rename_counterparty))
        // 删（POST）
        .route("/api/currencies/delete", post(delete_currency))
        .route("/api/counterparties/delete", post(delete_counterparty))
        .route("/api/groups/delete", post(delete_group))
        .route("/api/records/delete", post(delete_record))
        // 查（PUT）
        .route("/api/version", put(version))
        .route("/api/stats", put(list_stats))
        .route("/api/stats/month", put(month_stats))
        .route("/api/stats/objects", put(object_stats))
        .route(
            "/api/counterparties/monthly",
            put(list_counterparty_monthly),
        )
        .route("/api/records", put(list_records))
        .route("/api/records/object", put(object_records))
        .route("/api/counterparties", put(list_counterparties))
        .route("/api/currencies", put(list_currencies))
        .layer(cors)
        .with_state(state)
}

/// 提供统一的连接访问
pub(crate) fn conn(state: &AppState) -> AnyhowResult<std::sync::MutexGuard<'_, Connection>> {
    state
        .conn
        .lock()
        .map_err(|_| anyhow::anyhow!("数据库锁获取失败"))
}

// ---------- 基础接口 ----------

/// PUT /api/live — 存活检查
async fn live() -> Response {
    ok_json(json!({ "status": "ok" }))
}

/// PUT /api/help — 返回所有接口定义
async fn help() -> Response {
    ok_json(json!({
        "api": "非 RESTful：PUT 读，POST 其余",
        "http": "HTTP/2 或 HTTP/3（由 account_server --http-version 选择）",
        "base": {
            "PUT /api/live": "存活检查",
            "PUT /api/help": "接口定义",
            "PUT /api/version": "API 与数据库版本号"
        },
        "close": {
            "POST /api/close": "关闭服务器（优雅关闭）"
        },
        "create": {
            "POST /api/currencies": "添加货币",
            "POST /api/counterparties": "添加交易对象",
            "POST /api/groups": "添加一个组（含组内多条记录）",
            "POST /api/groups/{group_id}/records": "往组内添加一条记录"
        },
        "update": {
            "POST /api/currencies/rename": "重命名货币",
            "POST /api/counterparties/rename": "重命名交易对象"
        },
        "delete": {
            "POST /api/currencies/delete": "删除货币（须未被引用）",
            "POST /api/counterparties/delete": "删除交易对象（须未被引用）",
            "POST /api/groups/delete": "删除一个组",
            "POST /api/records/delete": "删除一条记录"
        },
        "query": {
            "PUT /api/version": "API 与数据库版本号",
            "PUT /api/stats": "按月统计（按需从月表计算）",
            "PUT /api/stats/month": "查指定月份范围的统计",
            "PUT /api/stats/objects": "按交易对象统计月范围收支",
            "PUT /api/counterparties/monthly": "查交易对象月度明细",
            "PUT /api/records": "查时间范围记录",
            "PUT /api/records/object": "查指定对象的时间范围记录",
            "PUT /api/counterparties": "查交易对象",
            "PUT /api/currencies": "查货币"
        }
    }))
}

/// POST /api/close — 关闭服务器
///
/// 先向客户端返回成功响应，再延迟触发服务器优雅关闭，
/// 确保客户端能完整收到 `{ "closed": true }` 结果。
async fn close(State(state): State<AppState>) -> Response {
    log::info!("收到关闭请求，服务器将在响应后关闭");
    let shutdown = state.shutdown.clone();
    tokio::spawn(async move {
        // 预留 100ms 让当前响应发送完毕
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        shutdown.request();
    });
    ok_json(json!({ "closed": true }))
}

// ---------- 增 ----------

/// POST /api/currencies — 添加货币
async fn add_currency(
    State(state): State<AppState>,
    Json(req): Json<CurrencySubmit>,
) -> Result<Response, ApiError> {
    let c = conn(&state)?;
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError(StatusCode::BAD_REQUEST, "货币名称不能为空".into()));
    }
    let id = db::add_currency(&c, &name)?;
    log::info!("添加货币: {name} (id={id})");
    Ok(ok_json(json!({ "id": id, "name": name })))
}

/// POST /api/counterparties — 添加交易对象
async fn add_counterparty(
    State(state): State<AppState>,
    Json(req): Json<CounterpartySubmit>,
) -> Result<Response, ApiError> {
    let mut c = conn(&state)?;
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "交易对象名称不能为空".into(),
        ));
    }
    let balance = req.balance.unwrap_or(Money::ZERO);
    let id = db::add_counterparty(&mut c, &name, req.currency_id, balance)?;
    log::info!("添加交易对象: {name} (id={id})");
    Ok(ok_json(json!({ "id": id, "name": name })))
}

/// POST /api/currencies/rename — 重命名货币
async fn rename_currency(
    State(state): State<AppState>,
    Json(req): Json<RenameRequest>,
) -> Result<Response, ApiError> {
    let mut c = conn(&state)?;
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError(StatusCode::BAD_REQUEST, "货币名称不能为空".into()));
    }
    db::rename_currency(&mut c, req.id, &name).map_err(|e| {
        let msg = e.to_string();
        let status = if msg.contains("已存在") {
            StatusCode::CONFLICT
        } else if msg.contains("不存在") {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        ApiError(status, msg)
    })?;
    log::info!("重命名货币: id={}, name={name}", req.id);
    Ok(ok_json(json!({ "id": req.id, "name": name })))
}

/// POST /api/counterparties/rename — 重命名交易对象
async fn rename_counterparty(
    State(state): State<AppState>,
    Json(req): Json<RenameRequest>,
) -> Result<Response, ApiError> {
    let mut c = conn(&state)?;
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "交易对象名称不能为空".into(),
        ));
    }
    db::rename_counterparty(&mut c, req.id, &name).map_err(|e| {
        let msg = e.to_string();
        let status = if msg.contains("已存在") {
            StatusCode::CONFLICT
        } else if msg.contains("不存在") {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        ApiError(status, msg)
    })?;
    log::info!("重命名交易对象: id={}, name={name}", req.id);
    Ok(ok_json(json!({ "id": req.id, "name": name })))
}

/// POST /api/groups — 添加一个组
async fn add_group(
    State(state): State<AppState>,
    Json(req): Json<GroupSubmit>,
) -> Result<Response, ApiError> {
    let mut c = conn(&state)?;
    let name = req.name.trim().to_string();
    // 币种一致性校验（转出/转入对象货币必须相同且与金额货币一致）
    month::validate_no_cross_currency(&c, &req.records)?;
    let group_id = month::add_group(&mut c, &name, &req.records)?;
    log::info!(
        "添加交易组: group_id={group_id}, {} 条记录",
        req.records.len()
    );
    Ok(ok_json(json!({ "id": group_id, "name": name })))
}

/// POST /api/groups/{group_id}/records — 往组内添加一条记录
///
/// 请求体为 `RecordSubmit` 字段 + 可选 `group_months`（该组当前包含的月份，
/// 逗号分隔，用于定位组实例；新方案下同 group_id 可在不同月份的组间复用）。
async fn add_record(
    State(state): State<AppState>,
    axum::extract::Path(group_id): axum::extract::Path<i64>,
    Json(req): Json<serde_json::Value>,
) -> Result<Response, ApiError> {
    let mut c = conn(&state)?;
    let traded_at = req["traded_at"]
        .as_str()
        .ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "缺少 traded_at".into()))?;
    let month = month::month_of(traded_at).to_string();
    let group_months = req
        .get("group_months")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let rec: RecordSubmit = serde_json::from_value(req.clone())
        .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "记录格式不正确".into()))?;
    let id = month::add_record(&mut c, &month, group_id, group_months.as_deref(), &rec)?;
    log::info!("添加记录: id={id}, group_id={group_id}");
    Ok(ok_json(json!({ "id": id, "group_id": group_id })))
}

// ---------- 删 ----------

/// POST /api/currencies/delete — 删除货币
async fn delete_currency(
    State(state): State<AppState>,
    Json(req): Json<DeleteRequest>,
) -> Result<Response, ApiError> {
    let mut c = conn(&state)?;
    match db::delete_currency(&mut c, req.id)? {
        true => {
            log::info!("删除货币: id={}", req.id);
            Ok(ok_json(json!({ "deleted": true })))
        }
        false => Err(ApiError(
            StatusCode::CONFLICT,
            "货币仍被交易对象或记录引用，无法删除".into(),
        )),
    }
}

/// POST /api/counterparties/delete — 删除交易对象
async fn delete_counterparty(
    State(state): State<AppState>,
    Json(req): Json<DeleteRequest>,
) -> Result<Response, ApiError> {
    let mut c = conn(&state)?;
    match db::delete_counterparty(&mut c, req.id)? {
        true => {
            log::info!("删除交易对象: id={}", req.id);
            Ok(ok_json(json!({ "deleted": true })))
        }
        false => Err(ApiError(
            StatusCode::CONFLICT,
            "交易对象仍被记录使用，无法删除".into(),
        )),
    }
}

/// POST /api/groups/delete — 删除一个组
///
/// 请求体：`{ "id": group_id, "group_months": "2026-08,2026-09" }`。
/// `group_months` 为该组当前包含的月份（逗号分隔），用于定位组实例——
/// 新方案下同 group_id 可在不同月份的组间复用，必须按月份删除，避免误删同号组。
async fn delete_group(
    State(state): State<AppState>,
    Json(req): Json<serde_json::Value>,
) -> Result<Response, ApiError> {
    let mut c = conn(&state)?;
    let id = req["id"]
        .as_i64()
        .ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "缺少组 id".into()))?;
    let group_months = req
        .get("group_months")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let n = month::delete_group(&mut c, id, group_months.as_deref())?;
    log::info!("删除组: group_id={id}, 删除 {} 条记录", n);
    Ok(ok_json(json!({ "deleted": n > 0, "count": n })))
}

/// POST /api/records/delete — 删除一条记录
///
/// 请求体需含 record_id 与 traded_at（用于确定月表）
async fn delete_record(
    State(state): State<AppState>,
    Json(req): Json<serde_json::Value>,
) -> Result<Response, ApiError> {
    let mut c = conn(&state)?;
    let record_id = req["id"]
        .as_i64()
        .ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "缺少记录 id".into()))?;
    let traded_at = req["traded_at"]
        .as_str()
        .ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "缺少 traded_at".into()))?;
    let month = month::month_of(traded_at).to_string();
    match month::delete_record(&mut c, &month, record_id)? {
        true => {
            log::info!("删除记录: id={}", record_id);
            Ok(ok_json(json!({ "deleted": true })))
        }
        false => Err(ApiError(StatusCode::NOT_FOUND, "记录不存在".into())),
    }
}

// ---------- 查 ----------

/// PUT /api/version — 返回 API 与数据库当前版本号
async fn version(State(state): State<AppState>) -> Result<Response, ApiError> {
    let c = conn(&state)?;
    let db_ver = db::db_version(&c)?;
    Ok(ok_json(json!({
        "api": API_VERSION,
        "db": db_ver,
    })))
}

/// 按需从月表计算统计（`total_stats` 已删除，不再保存冗余汇总）。
fn collect_month_stats(
    conn: &Connection,
    from: Option<&str>,
    to: Option<&str>,
) -> AnyhowResult<Vec<crate::server::models::MonthStats>> {
    let mut out = Vec::new();
    for table in db::existing_transaction_tables(conn)? {
        let month = db::month_from_table_name(&table);
        let in_from = from.is_none_or(|f| month.as_str() >= f);
        let in_to = to.is_none_or(|t| month.as_str() <= t);
        if in_from && in_to {
            out.push(month::compute_month_stats(conn, &month)?);
        }
    }
    Ok(out)
}

/// 将月度统计序列化为 API 返回结构。
fn stats_json(stats: &crate::server::models::MonthStats) -> serde_json::Value {
    let income: Vec<_> = stats
        .income
        .iter()
        .map(|(n, mo)| json!({ "currency": n, "amount": mo }))
        .collect();
    let expense: Vec<_> = stats
        .expense
        .iter()
        .map(|(n, mo)| json!({ "currency": n, "amount": mo }))
        .collect();
    json!({
        "month": stats.month,
        "table_name": db::month_table_name(&stats.month),
        "income": income,
        "expense": expense,
    })
}

/// PUT /api/stats — 查所有月份的收支统计（按需从月表计算）
async fn list_stats(State(state): State<AppState>) -> Result<Response, ApiError> {
    let c = conn(&state)?;
    let list = collect_month_stats(&c, None, None)?;
    let out: Vec<serde_json::Value> = list.iter().map(stats_json).collect();
    Ok(ok_json(out))
}

/// PUT /api/stats/month — 查指定月范围的收支统计（from/to 为 `YYYY-MM`，均可省略）
async fn month_stats(
    State(state): State<AppState>,
    Json(req): Json<MonthQuery>,
) -> Result<Response, ApiError> {
    let c = conn(&state)?;
    let list = collect_month_stats(&c, req.from.as_deref(), req.to.as_deref())?;
    let out: Vec<serde_json::Value> = list.iter().map(stats_json).collect();
    Ok(ok_json(out))
}

/// PUT /api/stats/objects — 按指定一个/多个交易对象统计月范围收支
///
/// 返回结构：{ month, objects: [{ id, name, direction, currency, amount }] }
/// direction 为 `income`（收入）/ `expense`（支出）/ `balance`（当前余额）。
async fn object_stats(
    State(state): State<AppState>,
    Json(req): Json<ObjectStatsQuery>,
) -> Result<Response, ApiError> {
    if req.counterparty_ids.is_empty() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "至少需要选择一个交易对象".into(),
        ));
    }
    let c = conn(&state)?;
    let data = month::compute_object_stats(&c, &req)?;
    // 所选对象当前余额：id → (名称, 货币名, 余额)
    let cps = db::list_counterparties(&c)?;
    let currencies = db::list_currencies(&c)?;
    let mut balances: std::collections::HashMap<i64, (String, String, Money)> =
        std::collections::HashMap::new();
    for x in &cps {
        if !req.counterparty_ids.contains(&x.id) {
            continue;
        }
        let cur = currencies
            .iter()
            .find(|c| c.id == x.currency_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| format!("id={}", x.currency_id));
        balances.insert(x.id, (x.name.clone(), cur, x.balance));
    }
    let out: Vec<serde_json::Value> = data
        .into_iter()
        .map(|(month, items)| {
            let mut objects: Vec<serde_json::Value> = items
                .into_iter()
                .map(|(id, name, tag, money)| {
                    let (dir, cur) = tag.split_once(':').unwrap_or(("unknown", &tag));
                    json!({
                        "id": id,
                        "name": name,
                        "direction": dir,
                        "currency": cur,
                        "amount": money,
                    })
                })
                .collect();
            // 每个选中对象附加一行当前余额
            for (id, (name, cur, bal)) in &balances {
                objects.push(json!({
                    "id": id,
                    "name": name,
                    "direction": "balance",
                    "currency": cur,
                    "amount": bal,
                }));
            }
            json!({ "month": month, "objects": objects })
        })
        .collect();
    Ok(ok_json(out))
}

/// 通用：查询记录（跨月表）
async fn query_records(state: &AppState, query: &RangeQuery) -> Result<Response, ApiError> {
    let c = conn(state)?;
    let list: Vec<Record> = month::list_records(&c, query)?;
    // 关联名称
    let cp = db::list_counterparties(&c)?;
    let currencies = db::list_currencies(&c)?;
    let cp_map: std::collections::HashMap<i64, &Counterparty> =
        cp.iter().map(|x| (x.id, x)).collect();
    let cur_map: std::collections::HashMap<i64, &Currency> =
        currencies.iter().map(|x| (x.id, x)).collect();
    let out: Vec<serde_json::Value> = list
        .iter()
        .map(|r| {
            json!({
                "id": r.id,
                "group_id": r.group_id,
                "group_name": r.group_name,
                "group_months": r.group_months,
                "from": cp_map.get(&r.from_id).map_or_else(|| "?".to_string(), |c| c.name.clone()),
                "from_id": r.from_id,
                "to": cp_map.get(&r.to_id).map_or_else(|| "?".to_string(), |c| c.name.clone()),
                "to_id": r.to_id,
                "amount": r.amount,
                "currency": cur_map.get(&r.currency_id).map_or_else(|| "?".to_string(), |c| c.name.clone()),
                "currency_id": r.currency_id,
                "remark": r.remark,
                "traded_at": r.traded_at,
            })
        })
        .collect();
    Ok(ok_json(out))
}

/// PUT /api/records — 查一定时间范围的所有记录
async fn list_records(
    State(state): State<AppState>,
    Json(query): Json<RangeQuery>,
) -> Result<Response, ApiError> {
    query_records(&state, &query).await
}

/// PUT /api/records/object — 查指定交易对象的时间范围记录
async fn object_records(
    State(state): State<AppState>,
    Json(query): Json<RangeQuery>,
) -> Result<Response, ApiError> {
    if query.counterparty_id.is_none() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "缺少 counterparty_id".into(),
        ));
    }
    query_records(&state, &query).await
}

/// PUT /api/counterparties/monthly — 查交易对象月度明细
///
/// 返回 `[{ counterparty_id, month, currency_id, out_amount, in_amount }]`，
/// 月份从对象首次创建月开始连续（无交易的月份金额为 0）。
async fn list_counterparty_monthly(
    State(state): State<AppState>,
    Json(query): Json<CounterpartyMonthlyQuery>,
) -> Result<Response, ApiError> {
    if query.counterparty_ids.is_empty() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "至少需要选择一个交易对象".into(),
        ));
    }
    let c = conn(&state)?;
    let rows = db::counterparty_monthly::list(&c, &query)?;
    Ok(ok_json(rows))
}

/// PUT /api/counterparties — 查有哪些交易对象
async fn list_counterparties(State(state): State<AppState>) -> Result<Response, ApiError> {
    let c = conn(&state)?;
    let list: Vec<Counterparty> = db::list_counterparties(&c)?;
    Ok(ok_json(list))
}

/// PUT /api/currencies — 查有哪些货币
async fn list_currencies(State(state): State<AppState>) -> Result<Response, ApiError> {
    let c = conn(&state)?;
    let list: Vec<Currency> = db::list_currencies(&c)?;
    Ok(ok_json(list))
}
