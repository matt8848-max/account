//! 数据模型
//!
//! 所有模型为纯 serde 结构，供 server 与 client 共用。
//! 金额类型 [`Money`] 位于 `server/money.rs`，这里重新导出以保持
//! `server::models::Money` 的导入路径稳定。

use serde::{Deserialize, Serialize};

pub use super::money::Money;

/// 货币
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Currency {
    pub id: i64,
    pub name: String,
}

/// 交易对象（转入/转出对象）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Counterparty {
    pub id: i64,
    pub name: String,
    pub currency_id: i64,
    /// 当前资金余额（定点数）
    pub balance: Money,
    /// 该对象的起始月份（`YYYY-MM`）：月度明细从该月开始连续
    pub created_month: String,
}

/// 交易组
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Group {
    pub id: i64,
    /// 组名称（最多 64 字符）
    pub name: String,
}

/// 交易记录（组内一条）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Record {
    pub id: i64,
    /// 所属组 ID
    pub group_id: i64,
    /// 组名（所属组，最多 64 字符）
    pub group_name: Option<String>,
    /// 组包含的所有月份（逗号分隔，如 `2026-08,2026-09`）；
    /// 用于展示组跨哪些月份，跨月表中该组的每条记录都冗余存储同一份
    pub group_months: Option<String>,
    /// 转出交易对象
    pub from_id: i64,
    /// 转入交易对象
    pub to_id: i64,
    /// 金额（定点数）
    pub amount: Money,
    /// 货币
    pub currency_id: i64,
    /// 备注（最多 256 字符）
    pub remark: Option<String>,
    /// 交易时间（精确到秒）
    pub traded_at: String,
}

/// 月统计（按货币分别统计）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MonthStats {
    /// 月份，形如 `2026-08`
    pub month: String,
    /// 收入：货币 → 金额
    pub income: Vec<(String, Money)>,
    /// 支出：货币 → 金额
    pub expense: Vec<(String, Money)>,
}

/// 交易对象月度明细（`counterparty_monthly` 表的一行）
///
/// 每个交易对象、每种货币、每个月份一行，月份从对象首次创建月开始连续。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CounterpartyMonthlyRow {
    pub counterparty_id: i64,
    /// 月份，形如 `2026-08`
    pub month: String,
    pub currency_id: i64,
    /// 该月转出金额（正数）
    pub out_amount: Money,
    /// 该月转入金额（正数）
    pub in_amount: Money,
}

/// 表单提交（组 + 记录）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupSubmit {
    /// 组名称（最多 64 字符）
    pub name: String,
    /// 组内多条交易记录
    pub records: Vec<RecordSubmit>,
}

/// 单条记录提交
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordSubmit {
    pub from_id: i64,
    pub to_id: i64,
    pub amount: Money,
    pub currency_id: i64,
    pub remark: Option<String>,
    pub traded_at: String,
}

/// 货币提交
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrencySubmit {
    pub name: String,
}

/// 交易对象提交
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterpartySubmit {
    pub name: String,
    pub currency_id: i64,
    /// 初始余额
    pub balance: Option<Money>,
}

/// 重命名请求（货币 / 交易对象共用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameRequest {
    pub id: i64,
    pub name: String,
}

/// 删除请求（按 ID）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteRequest {
    pub id: i64,
}

/// 时间范围查询
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RangeQuery {
    /// 起始时间（含），ISO 格式 `YYYY-MM-DDTHH:MM:SS`
    pub from: Option<String>,
    /// 结束时间（含），ISO 格式 `YYYY-MM-DDTHH:MM:SS`
    pub to: Option<String>,
    /// 限定交易对象 ID（None 表示全部）
    pub counterparty_id: Option<i64>,
}

/// 月份统计查询（多个月份范围）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MonthQuery {
    /// 起始月份（含），形如 `2026-08`；None 表示不限
    pub from: Option<String>,
    /// 结束月份（含），形如 `2026-08`；None 表示不限
    pub to: Option<String>,
}

/// 对象统计查询：指定一个或多个交易对象，统计其在月范围内的收支
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ObjectStatsQuery {
    /// 交易对象 ID 列表（至少一个）
    pub counterparty_ids: Vec<i64>,
    /// 起始月份（含），形如 `2026-08`；None 表示不限
    pub from: Option<String>,
    /// 结束月份（含），形如 `2026-08`；None 表示不限
    pub to: Option<String>,
}

/// 交易对象月度明细查询
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CounterpartyMonthlyQuery {
    /// 交易对象 ID 列表（至少一个）
    pub counterparty_ids: Vec<i64>,
    /// 起始月份（含）；None 表示不限
    pub from: Option<String>,
    /// 结束月份（含）；None 表示不限
    pub to: Option<String>,
}
