//! 客户端状态与纯数据逻辑
//!
//! 这里只放**与渲染无关**的状态类型和纯函数，便于单元测试：
//! - 页面枚举、排序方向、任务进度
//! - 后端返回的视图结构（记录、对象统计、卡片行、桑基图流向）
//! - 环比/同比、月份平移、流向聚合等纯计算
//! - 重命名后对前端缓存的联动更新
//!
//! `AccountApp`（app.rs）持有本模块的类型与输入框实体。

use crate::server::export::RatioValue;
use crate::server::models::{Counterparty, Currency, Money};
use serde::{Deserialize, Serialize};

/// 三个操作页的最小宽度（px）。
///
/// 内容区本身带双向滚动条；窗口窄于该宽度时页面保持这个宽度并出现横向滚动，
/// 避免筛选行、卡片表格被压扁或右侧控件被裁掉。
pub const PAGE_MIN_WIDTH: f32 = 1100.0;

/// 主界面页面
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    Records,
    Master,
    Stats,
}

/// 交易对象选择目标（筛选 / 草稿转出 / 草稿转入）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpTarget {
    Query,
    DraftFrom(usize),
    DraftTo(usize),
}

impl CpTarget {
    /// 是否作用于草稿行（而不是筛选条件）。
    pub fn draft_index(self) -> Option<usize> {
        match self {
            Self::Query => None,
            Self::DraftFrom(i) | Self::DraftTo(i) => Some(i),
        }
    }
}

/// 草稿行「转出/转入都选中后」的货币一致性检查结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftCurrencyCheck {
    /// 尚未同时选中转出与转入。
    Pending,
    /// 两个对象货币一致：自动填入该货币。
    Matched(i64),
    /// 两个对象货币不一致：必须清空转入方。
    Mismatch,
}

/// 检查草稿行两个交易对象的货币是否一致。
///
/// 参数为两个对象的货币 ID（未选中为 `None`）。用户可能先填转出、后填转入，
/// 因此这里只看最终状态，不区分选择顺序；`Mismatch` 由调用方统一清空转入方。
pub fn check_draft_currency(from: Option<i64>, to: Option<i64>) -> DraftCurrencyCheck {
    match (from, to) {
        (Some(a), Some(b)) if a == b => DraftCurrencyCheck::Matched(a),
        (Some(_), Some(_)) => DraftCurrencyCheck::Mismatch,
        _ => DraftCurrencyCheck::Pending,
    }
}

/// 记录列表排序方向（默认从新到旧）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    #[default]
    NewestFirst,
    OldestFirst,
}

impl SortOrder {
    /// 切换方向
    pub fn toggled(self) -> Self {
        match self {
            Self::NewestFirst => Self::OldestFirst,
            Self::OldestFirst => Self::NewestFirst,
        }
    }

    /// 中文说明（按钮 title 使用）
    pub fn label(self) -> &'static str {
        match self {
            Self::NewestFirst => "从新到旧",
            Self::OldestFirst => "从旧到新",
        }
    }
}

/// 长任务进度（底部状态栏显示）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskProgress {
    pub label: String,
    pub done: usize,
    pub total: usize,
}

impl TaskProgress {
    pub fn new(label: impl Into<String>, total: usize) -> Self {
        Self {
            label: label.into(),
            done: 0,
            total,
        }
    }

    pub fn advance(&mut self) {
        if self.done < self.total {
            self.done += 1;
        }
    }

    pub fn percent(&self) -> u8 {
        if self.total == 0 {
            return 0;
        }
        ((self.done.min(self.total) * 100) / self.total) as u8
    }

    pub fn describe(&self) -> String {
        if self.total == 0 {
            self.label.clone()
        } else {
            format!(
                "{} {}/{}",
                self.label,
                self.done.min(self.total),
                self.total
            )
        }
    }
}

/// 单条交易记录视图（`/api/records` 返回）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordView {
    pub id: i64,
    pub group_id: i64,
    pub group_name: Option<String>,
    pub group_months: Option<String>,
    pub from: String,
    pub from_id: i64,
    pub to: String,
    pub to_id: i64,
    pub amount: Money,
    pub currency: String,
    pub currency_id: i64,
    pub remark: Option<String>,
    pub traded_at: String,
}

/// 对象统计视图（`/api/stats/objects` 返回）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectStatView {
    pub month: String,
    pub objects: Vec<ObjectStatItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectStatItem {
    pub id: i64,
    pub name: String,
    pub direction: String,
    pub currency: String,
    pub amount: Money,
}

/// 统计页分组视图（一张卡牌）
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectGroupView {
    pub name: String,
    pub currency: String,
    pub balance: Money,
    pub monthly: Vec<MonthCardRow>,
}

/// 卡片内单月的一行：收入/支出及其环比、同比与月末结算余额。
///
/// 环比、同比使用 [`RatioValue`] 表示三种状态：基准月份不在数据库范围内（留空）、
/// 基准值为 0（`N.A.`）、正常百分比。所有数值计算统一使用 [`Money`]，避免浮点误差。
#[derive(Debug, Clone, PartialEq)]
pub struct MonthCardRow {
    pub month: String,
    pub income: Option<Money>,
    pub income_mom: RatioValue,
    pub income_yoy: RatioValue,
    pub expense: Option<Money>,
    pub expense_mom: RatioValue,
    pub expense_yoy: RatioValue,
    pub settlement: Money,
}

/// 桑基图/流向图中的一条「主要对象 ↔ 次要对象」流向。
///
/// 同一对交易对象的双向流水合并为一条：`out_amount` 为主要对象转出，
/// `in_amount` 为主要对象转入。
#[derive(Debug, Clone, PartialEq)]
pub struct FlowView {
    pub counterparty_id: i64,
    pub name: String,
    pub currency: String,
    pub out_amount: Money,
    pub in_amount: Money,
}

impl FlowView {
    /// 往来合计（转出 + 转入），用作桑基图条带宽度。
    pub fn total(&self) -> Money {
        self.out_amount
            .checked_add(self.in_amount)
            .unwrap_or(self.out_amount)
    }

    /// 净额（转入 − 转出）：正数表示主要对象净流入。
    pub fn net(&self) -> Money {
        self.in_amount
            .checked_sub(self.out_amount)
            .unwrap_or(self.in_amount)
    }
}

/// 计算环比/同比百分数：`(current - previous) / previous * 100`。
///
/// 基准值为 0 时增长率没有数学定义，返回 `None`；由调用方转成
/// [`RatioValue::NotApplicable`]（界面显示 `N.A.`）。
/// 基准月份是否在数据库记录范围内由调用方判断（范围外直接留空）。
pub fn growth_percent(current: Money, previous: Money) -> Option<Money> {
    if previous.is_zero() {
        return None;
    }
    let diff = current.checked_sub(previous).ok()?;
    let ratio = diff.checked_div(previous).ok()?;
    ratio.checked_mul(Money::from(100)).ok()
}

/// 月份平移：`shift_month("2026-01", -1)` → `Some("2025-12")`；格式非法返回 `None`。
pub fn shift_month(month: &str, delta: i32) -> Option<String> {
    let (year, month) = month.trim().split_once('-')?;
    let year = year.parse::<i32>().ok()?;
    let month = month.parse::<i32>().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    let total = year
        .checked_mul(12)?
        .checked_add(month - 1)?
        .checked_add(delta)?;
    let new_year = total.div_euclid(12);
    let new_month = total.rem_euclid(12) + 1;
    Some(format!("{new_year:04}-{new_month:02}"))
}

/// 连续月份列表的上限（100 年），防止非法输入导致无限循环。
const MAX_MONTH_SPAN: usize = 1200;

/// 生成 `from..=to` 的连续月份列表（含两端）。
///
/// 输入必须是 `YYYY-MM`；格式非法或跨度过大时返回空列表。
/// 用于「查询了 N 个月，没有交易的月份也要显示 0」。
pub fn month_range(from: &str, to: &str) -> Vec<String> {
    let Some(mut current) = shift_month(from, 0) else {
        return Vec::new();
    };
    let Some(end) = shift_month(to, 0) else {
        return Vec::new();
    };
    let mut months = Vec::new();
    while current <= end {
        months.push(current.clone());
        if months.len() > MAX_MONTH_SPAN {
            return Vec::new();
        }
        let Some(next) = shift_month(&current, 1) else {
            return Vec::new();
        };
        current = next;
    }
    months
}

/// 从记录构建桑基图流向：只保留与已选对象相关的记录。
///
/// - 两端都在已选集合内（组内互转）或两端都无关的记录直接忽略；
/// - 同一（次要对象, 货币）的双向流水合并为一条；
/// - 结果按往来合计从大到小排序，便于阅读。
pub fn build_flows(records: &[RecordView], selected: &[i64]) -> Vec<FlowView> {
    use std::collections::BTreeMap;

    let mut map: BTreeMap<(i64, String), FlowView> = BTreeMap::new();
    for r in records {
        let from_selected = selected.contains(&r.from_id);
        let to_selected = selected.contains(&r.to_id);
        let (other_id, other_name, out) = match (from_selected, to_selected) {
            (true, false) => (r.to_id, r.to.clone(), true),
            (false, true) => (r.from_id, r.from.clone(), false),
            // 组内互转或与已选对象无关：不绘制
            _ => continue,
        };
        let key = (other_id, r.currency.clone());
        let flow = map.entry(key).or_insert_with(|| FlowView {
            counterparty_id: other_id,
            name: other_name,
            currency: r.currency.clone(),
            out_amount: Money::ZERO,
            in_amount: Money::ZERO,
        });
        let slot = if out {
            &mut flow.out_amount
        } else {
            &mut flow.in_amount
        };
        *slot = slot.checked_add(r.amount).unwrap_or(*slot);
    }

    let mut flows: Vec<FlowView> = map.into_values().collect();
    flows.sort_by(|a, b| {
        b.total()
            .cmp(&a.total())
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.currency.cmp(&b.currency))
    });
    flows
}

/// 按关键字过滤候选：标签子串匹配、忽略大小写；空关键字返回全部。
pub fn filter_candidates(candidates: &[(i64, String)], query: &str) -> Vec<(i64, String)> {
    let q = query.trim().to_lowercase();
    candidates
        .iter()
        .filter(|(_, label)| q.is_empty() || label.to_lowercase().contains(&q))
        .cloned()
        .collect()
}

/// 取当前选中项的标签；未匹配返回空串。
pub fn selected_label_of(candidates: &[(i64, String)], value: i64) -> String {
    candidates
        .iter()
        .find(|(id, _)| *id == value)
        .map(|(_, label)| label.clone())
        .unwrap_or_default()
}

/// 交易对象下拉标签：名称 (货币)
pub fn cp_label(c: &Counterparty, currencies: &[Currency]) -> String {
    let cur = currencies
        .iter()
        .find(|x| x.id == c.currency_id)
        .map(|x| x.name.clone())
        .unwrap_or_else(|| format!("id={}", c.currency_id));
    format!("{} ({})", c.name, cur)
}

/// 按交易时间排序（ISO 字符串字典序即时间序）。
pub fn sort_records(records: &mut [RecordView], order: SortOrder) {
    match order {
        SortOrder::NewestFirst => records.sort_by(|a, b| b.traded_at.cmp(&a.traded_at)),
        SortOrder::OldestFirst => records.sort_by(|a, b| a.traded_at.cmp(&b.traded_at)),
    }
}

/// 当前本地时间 `HH:MM:SS UTC±H[:MM]`（状态栏上次探测时间）。
pub fn now_str() -> String {
    use chrono::Offset;
    let now = chrono::Local::now();
    let t = now.format("%H:%M:%S").to_string();
    let offset = now.offset().fix().local_minus_utc();
    let sign = if offset < 0 { '-' } else { '+' };
    let abs = offset.unsigned_abs();
    let h = abs / 3600;
    let m = (abs % 3600) / 60;
    let zone = if m == 0 {
        format!("UTC{sign}{h}")
    } else {
        format!("UTC{sign}{h}:{m:02}")
    };
    format!("{t} {zone}")
}

/// 重命名货币后更新前端缓存：货币列表 + 记录里的货币名。
///
/// 统计接口只返回货币名（没有 currency_id），由调用方在刷新统计时自然更新；
/// 这里保留参数是为了让调用点统一。
pub fn apply_currency_rename(
    currencies: &mut [Currency],
    records: &mut [RecordView],
    stats: &mut [ObjectStatView],
    id: i64,
    new_name: &str,
) {
    let old_name = currencies
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.name.clone());
    if let Some(c) = currencies.iter_mut().find(|c| c.id == id) {
        c.name = new_name.to_string();
    }
    for r in records.iter_mut() {
        if r.currency_id == id {
            r.currency = new_name.to_string();
        }
    }
    // 统计接口只返回货币名，没有 currency_id：按旧名替换。
    if let Some(old) = old_name {
        for month in stats.iter_mut() {
            for item in month.objects.iter_mut() {
                if item.currency == old {
                    item.currency = new_name.to_string();
                }
            }
        }
    }
}

/// 重命名交易对象后更新前端缓存（对象列表、记录、统计）。
pub fn apply_counterparty_rename(
    counterparties: &mut [Counterparty],
    records: &mut [RecordView],
    stats: &mut [ObjectStatView],
    id: i64,
    new_name: &str,
) {
    let old_name = counterparties
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.name.clone());
    if let Some(c) = counterparties.iter_mut().find(|c| c.id == id) {
        c.name = new_name.to_string();
    }
    for r in records.iter_mut() {
        if r.from_id == id {
            r.from = new_name.to_string();
        }
        if r.to_id == id {
            r.to = new_name.to_string();
        }
    }
    if let Some(old) = old_name {
        for month in stats.iter_mut() {
            for item in month.objects.iter_mut() {
                if item.id == id || item.name == old {
                    item.name = new_name.to_string();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cp(id: i64, name: &str, currency_id: i64) -> Counterparty {
        Counterparty {
            id,
            name: name.into(),
            currency_id,
            balance: Money::ZERO,
            created_month: "2026-01".into(),
        }
    }

    fn rec(id: i64, from: &str, to: &str) -> RecordView {
        RecordView {
            id,
            group_id: 1,
            group_name: None,
            group_months: None,
            from: from.into(),
            from_id: 1,
            to: to.into(),
            to_id: 2,
            amount: Money::from(100),
            currency: "RMB".into(),
            currency_id: 1,
            remark: None,
            traded_at: "2026-11-01T08:00:00".into(),
        }
    }

    /// 验证排序：默认从新到旧，切换后反向；两个方向结果必须不同。
    #[test]
    fn sort_records_should_order_both_ways() {
        let mut records = vec![
            RecordView {
                traded_at: "2026-11-01T08:00:00".into(),
                ..rec(1, "a", "b")
            },
            RecordView {
                traded_at: "2026-11-03T08:00:00".into(),
                ..rec(2, "a", "b")
            },
        ];
        sort_records(&mut records, SortOrder::NewestFirst);
        assert_eq!(records[0].id, 2);
        sort_records(&mut records, SortOrder::OldestFirst);
        assert_eq!(records[0].id, 1);
        // 不相等断言：两个方向不能得到同一顺序
        assert_ne!(SortOrder::NewestFirst.toggled(), SortOrder::NewestFirst);
    }

    /// 验证候选过滤：空关键字全显示；关键字忽略大小写；无匹配为空。
    #[test]
    fn filter_candidates_should_match_substring() {
        let all = vec![(0, "全部".to_string()), (1, "微信 (RMB)".to_string())];
        assert_eq!(filter_candidates(&all, "").len(), 2);
        assert_eq!(filter_candidates(&all, "微信").len(), 1);
        assert_eq!(filter_candidates(&all, "rmb")[0].0, 1);
        assert!(filter_candidates(&all, "不存在").is_empty());
        assert_eq!(selected_label_of(&all, 1), "微信 (RMB)");
        assert_eq!(selected_label_of(&all, 99), "");
    }

    /// 验证重命名联动：对象名与货币名同步更新到记录/统计缓存。
    #[test]
    fn rename_should_update_caches() {
        let mut cps = vec![cp(1, "微信", 1), cp(2, "银行卡", 1)];
        let mut records = vec![rec(1, "微信", "银行卡")];
        let mut stats = vec![ObjectStatView {
            month: "2026-11".into(),
            objects: vec![ObjectStatItem {
                id: 1,
                name: "微信".into(),
                direction: "expense".into(),
                currency: "RMB".into(),
                amount: Money::from(10),
            }],
        }];
        apply_counterparty_rename(&mut cps, &mut records, &mut stats, 1, "微信钱包");
        assert_eq!(cps[0].name, "微信钱包");
        assert_eq!(records[0].from, "微信钱包");
        assert_eq!(stats[0].objects[0].name, "微信钱包");
        // 不相等断言：无关对象不能被改名
        assert_ne!(cps[1].name, "微信钱包");

        let mut currencies = vec![Currency {
            id: 1,
            name: "RMB".into(),
        }];
        apply_currency_rename(&mut currencies, &mut records, &mut stats, 1, "CNY");
        assert_eq!(currencies[0].name, "CNY");
        assert_eq!(records[0].currency, "CNY");
        assert_eq!(stats[0].objects[0].currency, "CNY");
        // 不相等断言：旧名不能残留
        assert_ne!(records[0].currency, "RMB");
    }

    /// 验证草稿货币检查：只选一侧为 Pending；同币种返回 Matched；异币种返回 Mismatch。
    #[test]
    fn draft_currency_check_should_cover_all_combinations() {
        assert_eq!(
            check_draft_currency(None, Some(1)),
            DraftCurrencyCheck::Pending
        );
        assert_eq!(
            check_draft_currency(Some(1), None),
            DraftCurrencyCheck::Pending
        );
        assert_eq!(
            check_draft_currency(Some(2), Some(2)),
            DraftCurrencyCheck::Matched(2)
        );
        assert_eq!(
            check_draft_currency(Some(2), Some(3)),
            DraftCurrencyCheck::Mismatch
        );
        // 不相等断言：同币种不能被判为不一致
        assert_ne!(
            check_draft_currency(Some(2), Some(2)),
            DraftCurrencyCheck::Mismatch
        );
    }

    /// 验证月份平移：跨年、向前/向后、非法格式。
    #[test]
    fn shift_month_should_cross_year_boundaries() {
        assert_eq!(shift_month("2026-01", -1).as_deref(), Some("2025-12"));
        assert_eq!(shift_month("2026-12", 1).as_deref(), Some("2027-01"));
        assert_eq!(shift_month("2026-11", -12).as_deref(), Some("2025-11"));
        assert_eq!(shift_month("2026-11", 0).as_deref(), Some("2026-11"));
        assert_eq!(shift_month("2026-13", 1), None);
        assert_eq!(shift_month("abc", 1), None);
        // 不相等断言：不能返回未平移的月份
        assert_ne!(shift_month("2026-01", -1).as_deref(), Some("2026-01"));
    }

    /// 验证环比计算：增长、下降；基准为 0 时返回 None（由界面显示 N.A.）。
    #[test]
    fn growth_percent_should_compute_signed_ratio() {
        let growth = growth_percent(Money::from(120), Money::from(100)).unwrap();
        assert_eq!(growth.to_string(), "20");
        let decline = growth_percent(Money::from(90), Money::from(100)).unwrap();
        assert_eq!(decline.to_string(), "-10");
        // 基准为 0：无法计算，返回 None（界面/导出显示 N.A.）
        assert_eq!(growth_percent(Money::ZERO, Money::ZERO), None);
        assert_eq!(growth_percent(Money::from(5), Money::ZERO), None);
        // 不相等断言：增长与下降不能相同
        assert_ne!(growth, decline);
    }

    /// 验证连续月份列表：包含两端、跨年、非法输入返回空。
    #[test]
    fn month_range_should_include_both_ends() {
        assert_eq!(
            month_range("2027-10", "2027-12"),
            vec!["2027-10", "2027-11", "2027-12"]
        );
        assert_eq!(
            month_range("2027-11", "2028-01"),
            vec!["2027-11", "2027-12", "2028-01"]
        );
        assert_eq!(month_range("2027-12", "2027-10"), Vec::<String>::new());
        assert!(month_range("abc", "2027-10").is_empty());
        // 不相等断言：单月区间不能少掉该月
        assert_ne!(month_range("2027-10", "2027-10").len(), 0);
    }

    /// 验证流向构建：合并同一对象的双向流水、忽略组内互转与无关记录。
    #[test]
    fn build_flows_should_merge_directions_and_skip_internal() {
        let mut out = rec(1, "钱包", "商店");
        out.from_id = 10;
        out.to_id = 20;
        out.amount = Money::from(30);
        let mut back = rec(2, "商店", "钱包");
        back.from_id = 20;
        back.to_id = 10;
        back.amount = Money::from(12);
        // 组内互转：两个对象都在已选集合内
        let mut internal = rec(3, "钱包", "钱包2");
        internal.from_id = 10;
        internal.to_id = 11;
        // 与已选对象无关
        let mut unrelated = rec(4, "a", "b");
        unrelated.from_id = 30;
        unrelated.to_id = 31;

        let flows = build_flows(&[out, back, internal, unrelated], &[10, 11]);
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].counterparty_id, 20);
        assert_eq!(flows[0].out_amount, Money::from(30));
        assert_eq!(flows[0].in_amount, Money::from(12));
        assert_eq!(flows[0].total(), Money::from(42));
        assert_eq!(flows[0].net(), Money::from(-18));
        // 不相等断言：无关记录不能混入
        assert_ne!(flows[0].counterparty_id, 30);
    }

    /// 验证流向排序：往来合计大的排在前面。
    #[test]
    fn build_flows_should_sort_by_total_desc() {
        let mut small = rec(1, "钱包", "小商店");
        small.from_id = 10;
        small.to_id = 20;
        small.amount = Money::from(5);
        let mut big = rec(2, "钱包", "大商店");
        big.from_id = 10;
        big.to_id = 21;
        big.amount = Money::from(80);
        let flows = build_flows(&[small, big], &[10]);
        assert_eq!(flows.len(), 2);
        assert_eq!(flows[0].counterparty_id, 21);
        assert_ne!(flows[0].counterparty_id, 20);
    }
    #[test]
    fn task_progress_should_clamp() {
        let mut t = TaskProgress::new("导出", 4);
        assert_eq!(t.percent(), 0);
        for _ in 0..10 {
            t.advance();
        }
        assert_eq!(t.percent(), 100);
        assert_eq!(t.describe(), "导出 4/4");
        // 总数为 0 时不出现除零
        assert_eq!(TaskProgress::new("未知", 0).percent(), 0);
    }
}
