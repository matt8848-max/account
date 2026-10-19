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

    /// 去零（约分）：约去 `value` 与 `divisor` 的公共 10 因子，结果等值且更简洁。
    ///
    /// 例如 `(40, 10)` → `(4, 1)`、`(120, 100)` → `(12, 10)`。
    /// 金额除数约定为 10 的幂，因此这里只约去 10 的因子，
    /// 保证 `to_decimal_string` 的小数位补零逻辑仍然成立。
    ///
    /// 说明：数据库存储不强制去零（历史数据可保持原精度），
    /// 但所有**计算**结果（余额、月度统计、UI 结算余额等）都会经过本函数。
    #[allow(clippy::manual_is_multiple_of)] // i64 尚无稳定的 is_multiple_of
    pub fn normalized(self) -> Self {
        if self.divisor == 0 {
            // 非法除数兜底：不 panic，按整数处理（正常情况下不会出现）
            return Self {
                value: self.value,
                divisor: 1,
            };
        }
        let mut value = self.value;
        let mut divisor = self.divisor;
        while divisor > 1 && divisor % 10 == 0 && value % 10 == 0 {
            value /= 10;
            divisor /= 10;
        }
        Self { value, divisor }
    }

    /// 定点数加法：先按最小公倍数对齐精度，相加后自动去零。
    ///
    /// 例：`(13, 10) + (27, 10) = (40, 10)` → 去零后为 `(4, 1)`。
    pub fn checked_add(self, other: Self) -> Result<Self, String> {
        let (a, b, divisor) = align_pair(self, other)?;
        let value = a.checked_add(b).ok_or_else(|| "金额相加溢出".to_string())?;
        Ok(Self { value, divisor }.normalized())
    }

    /// 定点数减法：先对齐精度，相减后自动去零。
    pub fn checked_sub(self, other: Self) -> Result<Self, String> {
        let (a, b, divisor) = align_pair(self, other)?;
        let value = a.checked_sub(b).ok_or_else(|| "金额相减溢出".to_string())?;
        Ok(Self { value, divisor }.normalized())
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

/// 求最大公约数（用于两个定点数精度对齐）
fn gcd_u64(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a.max(1)
}

/// 将两个定点数对齐到同一除数（最小公倍数），返回 (a 的数值, b 的数值, 对齐后的除数)
fn align_pair(a: Money, b: Money) -> Result<(i64, i64, u32), String> {
    if a.divisor == 0 || b.divisor == 0 {
        return Err("金额除数不能为 0".into());
    }
    let da = a.divisor as u64;
    let db = b.divisor as u64;
    let g = gcd_u64(da, db);
    let lcm = (da / g)
        .checked_mul(db)
        .ok_or_else(|| "金额精度对齐溢出".to_string())?;
    if lcm > u32::MAX as u64 {
        return Err("金额精度过大".into());
    }
    let scale_a = (lcm / da) as i128;
    let scale_b = (lcm / db) as i128;
    let av = (a.value as i128)
        .checked_mul(scale_a)
        .ok_or_else(|| "金额溢出".to_string())?;
    let bv = (b.value as i128)
        .checked_mul(scale_b)
        .ok_or_else(|| "金额溢出".to_string())?;
    let av = i64::try_from(av).map_err(|_| "金额溢出".to_string())?;
    let bv = i64::try_from(bv).map_err(|_| "金额溢出".to_string())?;
    Ok((av, bv, lcm as u32))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证去零：公共 10 因子被约掉，且值与展示均不变
    #[test]
    fn normalized_removes_common_zero_factor() {
        assert_eq!(
            Money {
                value: 40,
                divisor: 10
            }
            .normalized(),
            Money::from_int(4)
        );
        assert_eq!(
            Money {
                value: 120,
                divisor: 100
            }
            .normalized(),
            Money {
                value: 12,
                divisor: 10
            }
        );
        // 负数的公共因子同样约掉
        assert_eq!(
            Money {
                value: -400,
                divisor: 100
            }
            .normalized(),
            Money::from_int(-4)
        );
        // 无法整除时保持原样
        assert_eq!(
            Money {
                value: 13,
                divisor: 10
            }
            .normalized(),
            Money {
                value: 13,
                divisor: 10
            }
        );
        // 零值约到整数 0
        assert_eq!(
            Money {
                value: 0,
                divisor: 1000
            }
            .normalized(),
            Money::from_int(0)
        );
    }

    /// 验证题述场景：1.3 + 2.7 = 4.0，计算结果自动去零为 (4, 1)
    #[test]
    fn add_auto_reduces_to_zero_remainder() {
        let sum = Money {
            value: 13,
            divisor: 10,
        }
        .checked_add(Money {
            value: 27,
            divisor: 10,
        })
        .unwrap();
        assert_eq!(sum, Money::from_int(4));
        assert_eq!(sum.to_decimal_string(), "4");
    }

    /// 验证不同精度相加：按最小公倍数对齐后再相加并去零
    #[test]
    fn add_aligns_different_divisors() {
        // 1.3 + 2.75 = 4.05
        let sum = Money {
            value: 13,
            divisor: 10,
        }
        .checked_add(Money {
            value: 275,
            divisor: 100,
        })
        .unwrap();
        assert_eq!(
            sum,
            Money {
                value: 405,
                divisor: 100
            }
        );
        assert_eq!(sum.to_decimal_string(), "4.05");
    }

    /// 验证减法：结果去零，借位/负数场景展示正确
    #[test]
    fn sub_auto_reduces_and_handles_negative() {
        let diff = Money {
            value: 40,
            divisor: 10,
        }
        .checked_sub(Money {
            value: 27,
            divisor: 10,
        })
        .unwrap();
        assert_eq!(
            diff,
            Money {
                value: 13,
                divisor: 10
            }
        );
        let negative = Money::from_int(1)
            .checked_sub(Money {
                value: 30,
                divisor: 10,
            })
            .unwrap();
        assert_eq!(negative, Money::from_int(-2));
        assert_eq!(negative.to_decimal_string(), "-2");
    }

    /// 验证非法除数返回错误而不是 panic
    #[test]
    fn arithmetic_rejects_zero_divisor() {
        assert!(
            Money {
                value: 1,
                divisor: 0
            }
            .checked_add(Money::from_int(1))
            .is_err()
        );
        assert!(
            Money::from_int(1)
                .checked_sub(Money {
                    value: 1,
                    divisor: 0
                })
                .is_err()
        );
    }
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
