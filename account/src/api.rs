//! HTTP API 层
//!
//! 非 RESTful 规范：
//! - `PUT` 负责所有读类操作
//! - `POST` 负责其余所有操作（增/删/改/其他）
//!
//! 通信协议 HTTP/2.0（由 axum-server rustls TLS 协商）。

use crate::db;
use crate::db::month;
use crate::models::{
    Counterparty, CounterpartySubmit, Currency, CurrencySubmit, DeleteRequest, GroupSubmit,
    MonthMeta, MonthQuery, Money, ObjectStatsQuery, RangeQuery, Record, RecordSubmit,
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

/// API 当前版本
pub const API_VERSION: i64 = 1;

/// 共享数据库连接状态
#[derive(Clone)]
pub struct AppState {
    pub conn: Arc<Mutex<Connection>>,
    /// 服务器关闭句柄：`POST /api/close` 触发优雅关闭
    /// （泛型为监听地址类型，与 `bind_rustls` 的 Server 匹配）
    pub shutdown: axum_server::Handle<SocketAddr>,
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
        "http": "HTTP/2.0",
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
        "delete": {
            "POST /api/currencies/delete": "删除货币（须未被引用）",
            "POST /api/counterparties/delete": "删除交易对象（须未被引用）",
            "POST /api/groups/delete": "删除一个组",
            "POST /api/records/delete": "删除一条记录"
        },
        "query": {
            "PUT /api/version": "API 与数据库版本号",
            "PUT /api/stats": "查总表统计",
            "PUT /api/stats/month": "查某月统计",
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
    let handle = state.shutdown.clone();
    tokio::spawn(async move {
        // 预留 100ms 让当前响应发送完毕
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        handle.shutdown();
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
    let c = conn(&state)?;
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "交易对象名称不能为空".into(),
        ));
    }
    let balance = req.balance.unwrap_or(Money::from_int(0));
    let id = db::add_counterparty(&c, &name, req.currency_id, balance)?;
    log::info!("添加交易对象: {name} (id={id})");
    Ok(ok_json(json!({ "id": id, "name": name })))
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
async fn add_record(
    State(state): State<AppState>,
    axum::extract::Path(group_id): axum::extract::Path<i64>,
    Json(req): Json<RecordSubmit>,
) -> Result<Response, ApiError> {
    let c = conn(&state)?;
    let month = month::month_of(&req.traded_at).to_string();
    let id = month::add_record(&c, &month, group_id, &req)?;
    log::info!("添加记录: id={id}, group_id={group_id}");
    Ok(ok_json(json!({ "id": id, "group_id": group_id })))
}

// ---------- 删 ----------

/// POST /api/currencies/delete — 删除货币
async fn delete_currency(
    State(state): State<AppState>,
    Json(req): Json<DeleteRequest>,
) -> Result<Response, ApiError> {
    let c = conn(&state)?;
    match db::delete_currency(&c, req.id)? {
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
    let c = conn(&state)?;
    match db::delete_counterparty(&c, req.id)? {
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
async fn delete_group(
    State(state): State<AppState>,
    Json(req): Json<DeleteRequest>,
) -> Result<Response, ApiError> {
    let c = conn(&state)?;
    let n = month::delete_group(&c, req.id)?;
    log::info!("删除组: group_id={}, 删除 {} 条记录", req.id, n);
    Ok(ok_json(json!({ "deleted": n > 0, "count": n })))
}

/// POST /api/records/delete — 删除一条记录
///
/// 请求体需含 record_id 与 traded_at（用于确定月表）
async fn delete_record(
    State(state): State<AppState>,
    Json(req): Json<serde_json::Value>,
) -> Result<Response, ApiError> {
    let c = conn(&state)?;
    let record_id = req["id"]
        .as_i64()
        .ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "缺少记录 id".into()))?;
    let traded_at = req["traded_at"]
        .as_str()
        .ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "缺少 traded_at".into()))?;
    let month = month::month_of(traded_at).to_string();
    match month::delete_record(&c, &month, record_id)? {
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

/// PUT /api/stats — 查总表的收支统计
async fn list_stats(State(state): State<AppState>) -> Result<Response, ApiError> {
    let c = conn(&state)?;
    let list: Vec<MonthMeta> = db::list_month_stats(&c)?;
    // 将 stats 中的 (String, Money) 序列化为可读结构
    let out: Vec<serde_json::Value> = list
        .iter()
        .map(|m| {
            let income: Vec<_> = m
                .stats
                .income
                .iter()
                .map(|(n, mo)| json!({ "currency": n, "amount": mo }))
                .collect();
            let expense: Vec<_> = m
                .stats
                .expense
                .iter()
                .map(|(n, mo)| json!({ "currency": n, "amount": mo }))
                .collect();
            json!({
                "month": m.month,
                "table_name": m.table_name,
                "income": income,
                "expense": expense,
            })
        })
        .collect();
    Ok(ok_json(out))
}

/// PUT /api/stats/month — 查指定月范围的收支统计（from/to 为 `YYYY-MM`，均可省略）
async fn month_stats(
    State(state): State<AppState>,
    Json(req): Json<MonthQuery>,
) -> Result<Response, ApiError> {
    let c = conn(&state)?;
    let meta = db::list_month_stats(&c)?;
    let filtered: Vec<_> = meta
        .into_iter()
        .filter(|m| {
            let in_from = req.from.as_ref().is_none_or(|f| &m.month >= f);
            let in_to = req.to.as_ref().is_none_or(|t| &m.month <= t);
            in_from && in_to
        })
        .collect();
    let out: Vec<serde_json::Value> = filtered
        .iter()
        .map(|m| {
            let income: Vec<_> = m
                .stats
                .income
                .iter()
                .map(|(n, mo)| json!({ "currency": n, "amount": mo }))
                .collect();
            let expense: Vec<_> = m
                .stats
                .expense
                .iter()
                .map(|(n, mo)| json!({ "currency": n, "amount": mo }))
                .collect();
            json!({
                "month": m.month,
                "income": income,
                "expense": expense,
            })
        })
        .collect();
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