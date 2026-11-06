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

    /// 确定性伪随机数生成器（线性同余）。
    ///
    /// 金额运算的「无明确边界」场景需要随机覆盖取值空间，但为测试引入 `rand` 依赖
    /// 会增加编译成本；这里用固定种子的 LCG 即可得到可复现的随机序列。
    struct Lcg(u64);

    impl Lcg {
        /// 以指定种子构造（种子强制为奇数，避免 LCG 退化）
        fn new(seed: u64) -> Self {
            Self(seed | 1)
        }

        /// 生成 `[lo, hi]` 闭区间内的整数
        fn range(&mut self, lo: i64, hi: i64) -> i64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let span = (hi - lo) as u64 + 1;
            lo + ((self.0 >> 11) % span) as i64
        }
    }

    /// 按数值比较两个定点数：允许精度不同（交叉相乘避免浮点误差）
    fn same_value(a: Money, b: Money) -> bool {
        a.value as i128 * b.divisor as i128 == b.value as i128 * a.divisor as i128
    }

    /// 验证去零：公共 10 因子被约掉，覆盖正负两域，并配套「不应相等」的用例
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
        // 负数只约掉一层：(-410, 100) = -4.10 → (-41, 10) = -4.1（人工评审补充用例）
        assert_eq!(
            Money {
                value: -410,
                divisor: 100
            }
            .normalized(),
            Money {
                value: -41,
                divisor: 10
            }
        );
        // (-41, 10) 已无公共 10 因子，保持原样（评审补充用例）
        assert_eq!(
            Money {
                value: -41,
                divisor: 10
            }
            .normalized(),
            Money {
                value: -41,
                divisor: 10
            }
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

        // 不相等断言：数值不同的金额不能被约成相等（防止实现「约多了」）
        assert_ne!(
            Money {
                value: -410,
                divisor: 100
            }
            .normalized(),
            Money::from_int(-4)
        );
        assert_ne!(
            Money {
                value: 13,
                divisor: 10
            }
            .normalized(),
            Money::from_int(1)
        );
        // 不相等断言：正负号相反的两笔金额永远不相等
        assert_ne!(
            Money {
                value: -410,
                divisor: 100
            }
            .normalized(),
            Money {
                value: 410,
                divisor: 100
            }
            .normalized()
        );
    }

    /// 验证去零在随机取值（含负数域）下既不改变数值、也保持幂等，
    /// 且不会把非零金额约成零、不会丢掉符号
    #[test]
    fn normalized_should_preserve_value_and_be_idempotent() {
        let mut rng = Lcg::new(0x5eed_1234);
        for _ in 0..256 {
            // 金额精度约定为 10 的幂，故 divisor 取 10^0..10^6
            let divisor = 10u32.pow(rng.range(0, 6) as u32);
            // value 额外乘 10 的 0~2 次幂，制造「可约」与「不可约」两类输入
            let factor = 10i64.pow(rng.range(0, 2) as u32);
            let value = rng.range(-999_999, 999_999) * factor;
            let money = Money { value, divisor };
            let norm = money.normalized();

            assert!(
                same_value(money, norm),
                "去零改变了数值: {:?} -> {:?}",
                money,
                norm
            );
            assert_eq!(norm.normalized(), norm, "去零应幂等: {:?}", norm);
            assert_eq!(
                norm.value < 0,
                value < 0,
                "去零不应丢失符号: {:?} -> {:?}",
                money,
                norm
            );
            // 不相等断言：非零金额约分后不能变成 0
            if value != 0 {
                assert_ne!(norm.value, 0, "非零金额被约成 0: {:?}", money);
            }
        }
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

    /// 验证加减法在随机取值（含负数域）下满足「加后减回原值」，
    /// 且错误的金额不会被判为相等（正负号、精度差异都不影响结果）
    #[test]
    fn add_sub_should_handle_negative_and_random_values() {
        let mut rng = Lcg::new(0xabcd_9876);
        for _ in 0..256 {
            // divisor 取 10^0..10^4、value 取 ±10^6 以内：相乘后不会触发 i64 溢出
            let da = 10u32.pow(rng.range(0, 4) as u32);
            let db = 10u32.pow(rng.range(0, 4) as u32);
            let a = Money {
                value: rng.range(-1_000_000, 1_000_000),
                divisor: da,
            };
            let b = Money {
                value: rng.range(-1_000_000, 1_000_000),
                divisor: db,
            };

            let sum = a.checked_add(b).expect("合法金额相加不应失败");
            // 交换律：a + b == b + a
            let sum_swapped = b.checked_add(a).expect("合法金额相加不应失败");
            assert!(
                same_value(sum, sum_swapped),
                "加法应满足交换律: {:?} + {:?}",
                a,
                b
            );
            // 减法回退：a + b - b == a（数值相等即可，精度可能不同）
            let back = sum.checked_sub(b).expect("合法金额相减不应失败");
            assert!(
                same_value(back, a),
                "加后减回应等于原值: ({:?} + {:?}) - {:?} = {:?}",
                a,
                b,
                b,
                back
            );
            // 不相等断言：加法的结果不能被误判为只等于其中一个加数（除非另一个为 0）
            if b.value != 0 && !same_value(b, Money::from_int(0)) {
                assert!(
                    !same_value(sum, a),
                    "非零加数不应使和等于被加数: {:?} + {:?}",
                    a,
                    b
                );
            }
            // 不相等断言：a 与 -a 的数值必须不同（b 为 0 时逐笔核对符号）
            if b.value == 0 {
                assert!(
                    !same_value(
                        a,
                        Money {
                            value: -a.value,
                            divisor: a.divisor
                        }
                    ) || a.value == 0,
                    "正负金额不应相等: {:?}",
                    a
                );
            }
        }
    }

    /// 验证小数展示：负数与不足一位的整数部分都能正确补零（如 -0.05、-4.1）
    #[test]
    fn to_decimal_string_should_render_negative_values() {
        assert_eq!(
            Money {
                value: -5,
                divisor: 100
            }
            .to_decimal_string(),
            "-0.05"
        );
        assert_eq!(
            Money {
                value: -41,
                divisor: 10
            }
            .to_decimal_string(),
            "-4.1"
        );
        assert_eq!(
            Money {
                value: -1230,
                divisor: 1000
            }
            .to_decimal_string(),
            "-1.23"
        );
        // 不相等断言：负号不能丢失
        assert_ne!(
            Money {
                value: -5,
                divisor: 100
            }
            .to_decimal_string(),
            "0.05"
        );
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
