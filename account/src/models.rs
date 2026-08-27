//! 数据模型
//!
//! 所有模型为纯 serde 结构，供 server 与 client 共用。
//!
//! 金额使用定点数 `(value, divisor)` 两个整数表示，实际值 = value / divisor，
//! 避免浮点运算误差。例如 `(1000, 10)` 表示 100.0，`(1000, 100)` 表示 10.00。

use serde::{Deserialize, Serialize};

/// 定点数金额：value / divisor
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money {
    /// 数额（整数）
    pub value: i64,
    /// 除数（正整数，表示精度）
    pub divisor: u32,
}

impl Money {
    /// 构造定点数金额，除数必须为正
    pub fn new(value: i64, divisor: u32) -> Result<Self, String> {
        if divisor == 0 {
            return Err("金额除数不能为 0".into());
        }
        Ok(Self { value, divisor })
    }

    /// 整数金额
    pub fn from_int(value: i64) -> Self {
        Self { value, divisor: 1 }
    }

    /// 实际值（字符串形式避免浮点误差展示）
    pub fn to_decimal_string(&self) -> String {
        let sign = if self.value < 0 { "-" } else { "" };
        let abs = self.value.unsigned_abs();
        let divisor = self.divisor as u64;
        let int_part = abs / divisor;
        let frac_part = abs % divisor;
        if frac_part == 0 {
            format!("{sign}{int_part}")
        } else {
            let frac = frac_part.to_string();
            // let padded = format!("{frac:0>width$}", width = self.divisor.to_string().len());  手工bug修复，小数部分位数计算错误
            let padded = format!(
                "{frac:0>width$}",
                width = (self.divisor as f64).log10() as usize
            );
            format!("{sign}{int_part}.{}", padded.trim_end_matches('0'))
        }
    }
}

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

/// 总表行：月份统计与月表名
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MonthMeta {
    pub month: String,
    pub table_name: String,
    pub stats: MonthStats,
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
