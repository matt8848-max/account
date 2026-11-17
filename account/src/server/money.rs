//! 定点数金额
//!
//! 金额使用 `(value, divisor)` 两个整数表示，实际值 = `value / divisor`。
//! 相比旧实现（`i64 / u32`），这里升级为 **`i128 / i128`**：
//! - 乘除法最容易溢出，`i128` 为业务金额提供了足够的中间精度；
//! - 分母不再要求是 10 的幂（`new(5, 2)` 即 2.5），因此 `Display`
//!   必须支持任意分母的长除法。
//!
//! 设计要点（与任务清单一致）：
//! - `new` 只校验分母为正，不做约分；`from_string` 默认按 `123/100` 解释小数。
//! - 加/减/乘/除统一走 `checked_*`，计算后调用 [`Money::normalized`] 自动约分。
//! - 约分只做两件事（完整 gcd 成本高）：
//!   1. 分子、分母同为偶数时（位运算判断）各除以 2，**只做一次 `if`**；
//!   2. 分母绝对值超过 [`REDUCE_DIVISOR_THRESHOLD`] 时做一次完整 gcd。
//! - 判等/比较按**数值**而非内部表示：`100/10` 与 `10/1` 相等，
//!   因此约分不必把 `100/10` 化成 `10/1`。

use serde::{Deserialize, Deserializer, Serialize};
use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

/// 触发完整 gcd 的分母阈值：超过该值才做一次完整约分（避免每次运算都算 gcd）。
pub const REDUCE_DIVISOR_THRESHOLD: i128 = 1_000_000_000;

/// `Display` 最多输出的小数位数。
///
/// 任意分母可能产生无限循环小数（如 `1/3`），这里截断到固定位数后去掉尾零，
/// 既保证展示稳定，也避免生成无限长的字符串。
pub const MAX_DISPLAY_DECIMALS: usize = 12;

/// 定点数金额：`value / divisor`，`divisor` 恒为正整数。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Money {
    /// 分子（带符号）
    pub value: i128,
    /// 分母（正整数）
    pub divisor: i128,
}

impl Money {
    /// 金额 0（分母为 1）。
    pub const ZERO: Self = Self {
        value: 0,
        divisor: 1,
    };

    /// 构造定点数金额；分母必须为正整数。
    ///
    /// 不在这里约分：`new(100, 10)` 保持 `100/10`，由调用方在需要时调用
    /// [`Money::normalized`]。这样构造出来的金额与输入文本/数据库内容一一对应，
    /// 便于调试与审阅。
    pub fn new(value: i128, divisor: i128) -> Result<Self, String> {
        if divisor <= 0 {
            return Err("金额分母必须为正整数".to_string());
        }
        Ok(Self { value, divisor })
    }

    /// 从字符串构造金额，默认按十进制小数解释（`"1.23"` → `123/100`）。
    ///
    /// 规则与旧 UI 的 `parse_money` 一致：
    /// - 空文本（或全空白）视为 0（“初始余额”允许留空）；
    /// - 只接受「可选负号 + ASCII 数字 + 至多一个小数点」；
    /// - 不接受正号、科学计数法、千位分隔符，小数点前必须有整数部分；
    /// - 精度由小数位数决定（`divisor = 10^n`），溢出时返回错误。
    pub fn from_string(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(Self::ZERO);
        }

        // 先剥离负号：符号不参与数值累加，避免“先减小数再取负”的历史 bug。
        let (negative, digits) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return Err(format!("金额格式不正确: {text}"));
        }

        let mut parts = digits.split('.');
        let int_part = parts.next().unwrap_or("");
        let frac_part = parts.next().unwrap_or("");
        if parts.next().is_some() {
            return Err(format!("金额格式不正确（多于一个小数点）: {text}"));
        }
        if int_part.is_empty() || !int_part.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!("金额格式不正确（缺少整数部分）: {text}"));
        }
        if !frac_part.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!("金额格式不正确（小数部分非法）: {text}"));
        }

        // 精度：小数位数 → 10^n；n 过大导致 i128 溢出时视为非法输入。
        let divisor = if frac_part.is_empty() {
            1
        } else {
            10i128
                .checked_pow(frac_part.len() as u32)
                .ok_or_else(|| format!("金额精度过大: {text}"))?
        };
        let int_val: i128 = int_part
            .parse()
            .map_err(|_| format!("金额整数部分溢出: {text}"))?;
        let mut value = int_val
            .checked_mul(divisor)
            .ok_or_else(|| format!("金额溢出: {text}"))?;
        if !frac_part.is_empty() {
            let frac_val: i128 = frac_part
                .parse()
                .map_err(|_| format!("金额小数部分溢出: {text}"))?;
            value = value
                .checked_add(frac_val)
                .ok_or_else(|| format!("金额溢出: {text}"))?;
        }
        if negative {
            value = value
                .checked_neg()
                .ok_or_else(|| format!("金额溢出: {text}"))?;
        }
        Self::new(value, divisor)
    }

    /// 是否为零。
    pub fn is_zero(&self) -> bool {
        self.value == 0
    }

    /// 取负；`i128::MIN` 无法取负时返回错误（业务金额不会触及该边界）。
    pub fn checked_neg(self) -> Result<Self, String> {
        Ok(Self {
            value: self
                .value
                .checked_neg()
                .ok_or_else(|| "金额取负溢出".to_string())?,
            divisor: self.divisor,
        })
    }

    /// 自动约分（成本受控）。
    ///
    /// - 分子、分母同时为偶数 → 各除以 2（位运算判断，只做一次）；
    /// - 分母超过 [`REDUCE_DIVISOR_THRESHOLD`] → 做一次完整 gcd。
    ///
    /// 注意：`100/10` 按该规则只会变成 `50/5`，但通过 [`PartialEq`] 的数值比较
    /// 它与 `10/1` 相等，因此不违背“最小化”的业务语义。
    pub fn normalized(self) -> Self {
        if self.divisor <= 0 {
            // 兜底：非法分母（正常构造不会出现）按整数处理，避免除零 panic。
            return Self {
                value: self.value,
                divisor: 1,
            };
        }
        let mut value = self.value;
        let mut divisor = self.divisor;

        // 位运算比取模更快；对负数同样成立（-4 & 1 == 0）。
        if value & 1 == 0 && divisor & 1 == 0 {
            value /= 2;
            divisor /= 2;
        }
        if divisor > REDUCE_DIVISOR_THRESHOLD {
            let g = gcd_u128(value.unsigned_abs(), divisor as u128);
            if g > 1 {
                value /= g as i128;
                divisor /= g as i128;
            }
        }
        Self { value, divisor }
    }

    /// 定点数加法：先对齐分母（最小公倍数），相加后自动约分。
    pub fn checked_add(self, other: Self) -> Result<Self, String> {
        let (a, b, divisor) = align_pair(self, other)?;
        let value = a.checked_add(b).ok_or_else(|| "金额相加溢出".to_string())?;
        Ok(Self { value, divisor }.normalized())
    }

    /// 定点数减法：先对齐分母，相减后自动约分。
    pub fn checked_sub(self, other: Self) -> Result<Self, String> {
        let (a, b, divisor) = align_pair(self, other)?;
        let value = a.checked_sub(b).ok_or_else(|| "金额相减溢出".to_string())?;
        Ok(Self { value, divisor }.normalized())
    }

    /// 定点数乘法：分子乘分子、分母乘分母，结果自动约分。
    pub fn checked_mul(self, other: Self) -> Result<Self, String> {
        let value = self
            .value
            .checked_mul(other.value)
            .ok_or_else(|| "金额相乘溢出".to_string())?;
        let divisor = self
            .divisor
            .checked_mul(other.divisor)
            .ok_or_else(|| "金额精度相乘溢出".to_string())?;
        Ok(Self { value, divisor }.normalized())
    }

    /// 定点数除法：`self / other`，结果自动约分。
    ///
    /// 除以 0 金额返回错误；若 `other.value` 为负，结果分子分母同时取反，
    /// 保证构造出的金额分母恒为正。
    pub fn checked_div(self, other: Self) -> Result<Self, String> {
        if other.value == 0 {
            return Err("金额不能除以 0".to_string());
        }
        let mut value = self
            .value
            .checked_mul(other.divisor)
            .ok_or_else(|| "金额相除溢出".to_string())?;
        let mut divisor = self
            .divisor
            .checked_mul(other.value)
            .ok_or_else(|| "金额精度相除溢出".to_string())?;
        if divisor < 0 {
            value = value
                .checked_neg()
                .ok_or_else(|| "金额相除溢出".to_string())?;
            divisor = divisor
                .checked_neg()
                .ok_or_else(|| "金额精度相除溢出".to_string())?;
        }
        if divisor == 0 {
            return Err("金额分母必须为正整数".to_string());
        }
        Ok(Self { value, divisor }.normalized())
    }

    /// 数值比较：交叉相乘，避免浮点误差。
    ///
    /// 极端情况下（|value| 接近 `i128::MAX` 且分母很大）交叉相乘会溢出，
    /// 此时退回 `f64` 比值比较。业务金额远小于该量级，不影响正常使用；
    /// 该兜底保证 `Ord` 在所有输入下都有确定结果，不会 panic。
    fn compare_numeric(&self, other: &Self) -> Ordering {
        let left = self.value.checked_mul(other.divisor);
        let right = other.value.checked_mul(self.divisor);
        if let (Some(left), Some(right)) = (left, right) {
            return left.cmp(&right);
        }
        let left = self.value as f64 / self.divisor as f64;
        let right = other.value as f64 / other.divisor as f64;
        left.partial_cmp(&right).unwrap_or(Ordering::Equal)
    }
}

impl Default for Money {
    /// 默认金额为 0。
    fn default() -> Self {
        Self::ZERO
    }
}

impl From<i128> for Money {
    /// 整数金额（分母为 1）。
    fn from(value: i128) -> Self {
        Self { value, divisor: 1 }
    }
}

impl FromStr for Money {
    type Err = String;

    /// 字符串解析，行为与 [`Money::from_string`] 一致。
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_string(s)
    }
}

impl fmt::Display for Money {
    /// 按数值输出十进制字符串（最多 [`MAX_DISPLAY_DECIMALS`] 位小数，去掉尾零）。
    ///
    /// 使用长除法而不是浮点转换：`5/2` → `2.5`、`1/3` → `0.333333333333`。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 正常构造的分母恒为正；这里仍做一次兜底，避免异常数据导致除零。
        let (value, divisor) = if self.divisor > 0 {
            (self.value, self.divisor)
        } else {
            (
                self.value.wrapping_neg(),
                self.divisor.wrapping_neg().max(1),
            )
        };
        let sign = if value < 0 { "-" } else { "" };
        let abs = value.unsigned_abs();
        let divisor = divisor as u128;
        let int_part = abs / divisor;
        let mut rem = abs % divisor;
        if rem == 0 {
            return write!(f, "{sign}{int_part}");
        }

        let mut frac = String::new();
        for _ in 0..MAX_DISPLAY_DECIMALS {
            let Some(next) = rem.checked_mul(10) else {
                break;
            };
            let digit = next / divisor;
            rem = next % divisor;
            // digit 一定小于 10（余数 < 分母）。
            frac.push(char::from(b'0' + digit as u8));
            if rem == 0 {
                break;
            }
        }
        while frac.ends_with('0') {
            frac.pop();
        }
        if frac.is_empty() {
            write!(f, "{sign}{int_part}")
        } else {
            write!(f, "{sign}{int_part}.{frac}")
        }
    }
}

impl PartialEq for Money {
    /// 按数值判等：`100/10 == 10/1`。
    fn eq(&self, other: &Self) -> bool {
        self.compare_numeric(other) == Ordering::Equal
    }
}

impl Eq for Money {}

impl PartialOrd for Money {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Money {
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare_numeric(other)
    }
}

impl<'de> Deserialize<'de> for Money {
    /// 反序列化时复用 [`Money::new`] 校验分母，拒绝 0/负数。
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            value: i128,
            divisor: i128,
        }
        let repr = Repr::deserialize(deserializer)?;
        Money::new(repr.value, repr.divisor).map_err(serde::de::Error::custom)
    }
}

/// 求最大公约数（`u128` 版本，避免 `i128::MIN` 取绝对值溢出）。
fn gcd_u128(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a.max(1)
}

/// 将两个定点数对齐到同一分母（最小公倍数）。
///
/// 返回 `(a 的分子, b 的分子, 对齐后的分母)`；任何一步溢出都返回错误，
/// 由调用方的 `checked_*` 统一向上传递。
fn align_pair(a: Money, b: Money) -> Result<(i128, i128, i128), String> {
    if a.divisor <= 0 || b.divisor <= 0 {
        return Err("金额分母必须为正整数".to_string());
    }
    let da = a.divisor as u128;
    let db = b.divisor as u128;
    let g = gcd_u128(da, db);
    let lcm = (da / g)
        .checked_mul(db)
        .ok_or_else(|| "金额精度对齐溢出".to_string())?;
    let lcm_i = i128::try_from(lcm).map_err(|_| "金额精度过大".to_string())?;
    let scale_a = i128::try_from(lcm / da).map_err(|_| "金额精度过大".to_string())?;
    let scale_b = i128::try_from(lcm / db).map_err(|_| "金额精度过大".to_string())?;
    let av = a
        .value
        .checked_mul(scale_a)
        .ok_or_else(|| "金额溢出".to_string())?;
    let bv = b
        .value
        .checked_mul(scale_b)
        .ok_or_else(|| "金额溢出".to_string())?;
    Ok((av, bv, lcm_i))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证构造校验：分母为 0 或负数必须报错，正分母正常构造且不约分。
    #[test]
    fn new_should_reject_non_positive_divisor() {
        assert!(Money::new(1, 0).is_err());
        assert!(Money::new(1, -2).is_err());
        let m = Money::new(100, 10).unwrap();
        assert_eq!(m.value, 100);
        assert_eq!(m.divisor, 10);
        // 不相等断言：构造阶段不应偷偷约分
        assert_ne!((m.value, m.divisor), (10, 1));
    }

    /// 验证字符串解析：默认按 123/100 解释小数，负数/空串/非法输入都正确。
    #[test]
    fn from_string_should_parse_decimal_amounts() {
        assert_eq!(
            Money::from_string("100").unwrap(),
            Money::new(100, 1).unwrap()
        );
        assert_eq!(
            Money::from_string("-1.23").unwrap(),
            Money::new(-123, 100).unwrap()
        );
        assert_eq!(
            Money::from_string("-0.05").unwrap(),
            Money::new(-5, 100).unwrap()
        );
        assert_eq!(Money::from_string("").unwrap(), Money::ZERO);
        assert_eq!(Money::from_string("   ").unwrap(), Money::ZERO);
        assert_eq!(
            Money::from_string("  -12.5 ").unwrap(),
            Money::new(-125, 10).unwrap()
        );
        // 不相等断言：负数不能解析成正数
        assert_ne!(
            Money::from_string("-1.23").unwrap(),
            Money::new(123, 100).unwrap()
        );

        for bad in ["abc", "1.2.3", "-", ".5", "1,000", "+1", "1e3"] {
            assert!(Money::from_string(bad).is_err(), "应拒绝: {bad}");
        }
    }

    /// 验证 Display：任意分母长除法、负数、整数与循环小数截断。
    #[test]
    fn display_should_render_any_divisor() {
        assert_eq!(Money::new(5, 2).unwrap().to_string(), "2.5");
        assert_eq!(Money::new(50, 5).unwrap().to_string(), "10");
        assert_eq!(Money::new(-5, 100).unwrap().to_string(), "-0.05");
        assert_eq!(Money::new(-41, 10).unwrap().to_string(), "-4.1");
        assert_eq!(Money::new(123, 100).unwrap().to_string(), "1.23");
        assert_eq!(Money::new(1, 3).unwrap().to_string(), "0.333333333333");
        // 不相等断言：负号不能丢失
        assert_ne!(Money::new(-5, 100).unwrap().to_string(), "0.05");
    }

    /// 验证数值判等：不同表示但数值相同应相等；数值不同必须不相等。
    #[test]
    fn equality_should_compare_numeric_value() {
        assert_eq!(Money::new(100, 10).unwrap(), Money::new(10, 1).unwrap());
        assert_eq!(Money::new(-100, 10).unwrap(), Money::new(-10, 1).unwrap());
        assert_ne!(Money::new(1, 3).unwrap(), Money::new(333, 1000).unwrap());
        assert_ne!(Money::new(1, 1).unwrap(), Money::new(-1, 1).unwrap());
    }

    /// 验证大小比较：正负数、不同分母都按数值排序。
    #[test]
    fn ordering_should_follow_numeric_value() {
        let mut values = vec![
            Money::new(1, 2).unwrap(),
            Money::new(-3, 2).unwrap(),
            Money::new(3, 4).unwrap(),
            Money::ZERO,
        ];
        values.sort();
        assert_eq!(
            values,
            vec![
                Money::new(-3, 2).unwrap(),
                Money::ZERO,
                Money::new(1, 2).unwrap(),
                Money::new(3, 4).unwrap(),
            ]
        );
        assert!(Money::new(-3, 2).unwrap() < Money::new(-1, 1).unwrap());
        // 不相等断言：大的数值不能被判为小于小的数值
        assert_ne!(
            Money::new(3, 4).unwrap().cmp(&Money::new(1, 2).unwrap()),
            Ordering::Less
        );
    }

    /// 验证约分规则：偶数只除一次 2；分母超过阈值时做一次完整 gcd。
    #[test]
    fn normalized_should_follow_cheap_rules() {
        // 4/2 → 2/1（一次偶数约分）
        let m = Money::new(4, 2).unwrap().normalized();
        assert_eq!(m, Money::new(2, 1).unwrap());
        // 100/10 → 50/5（只除一次 2），但数值上等于 10/1
        let m = Money::new(100, 10).unwrap().normalized();
        assert_eq!(m, Money::new(10, 1).unwrap());
        assert_eq!((m.value, m.divisor), (50, 5));
        // 分母超阈值：3_000_000_003 / 1_000_000_001 → 3/1
        let m = Money::new(3_000_000_003, 1_000_000_001)
            .unwrap()
            .normalized();
        assert_eq!((m.value, m.divisor), (3, 1));
        // 不相等断言：不能被约成 1
        assert_ne!(m, Money::new(1, 1).unwrap());
    }

    /// 验证四则运算与溢出保护：加减乘除结果正确，极大值溢出返回错误。
    #[test]
    fn checked_arithmetic_should_compute_and_guard_overflow() {
        let a = Money::new(13, 10).unwrap();
        let b = Money::new(27, 10).unwrap();
        assert_eq!(a.checked_add(b).unwrap(), Money::new(4, 1).unwrap());
        assert_eq!(b.checked_sub(a).unwrap(), Money::new(14, 10).unwrap());

        let half = Money::new(5, 2).unwrap();
        let two = Money::new(2, 1).unwrap();
        assert_eq!(half.checked_mul(two).unwrap(), Money::new(5, 1).unwrap());
        assert_eq!(half.checked_div(two).unwrap(), Money::new(5, 4).unwrap());

        let huge = Money::new(i128::MAX, 1).unwrap();
        assert!(huge.checked_add(Money::from(1)).is_err());
        assert!(huge.checked_mul(Money::from(2)).is_err());
        assert!(Money::from(1).checked_div(Money::ZERO).is_err());
    }

    /// 验证反序列化拒绝非法分母（避免 JSON 绕过构造校验）。
    #[test]
    fn deserialize_should_reject_zero_divisor() {
        let ok: Money = serde_json::from_str(r#"{"value":123,"divisor":100}"#).unwrap();
        assert_eq!(ok, Money::new(123, 100).unwrap());
        assert!(serde_json::from_str::<Money>(r#"{"value":1,"divisor":0}"#).is_err());
        assert!(serde_json::from_str::<Money>(r#"{"value":1,"divisor":-1}"#).is_err());
    }
}
