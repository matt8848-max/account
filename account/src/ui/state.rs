//! 客户端状态与纯数据逻辑
//!
//! 这里只放**与渲染无关**的状态类型和纯函数，便于单元测试：
//! - 页面枚举、排序方向、任务进度
//! - 后端返回的视图结构
//! - 重命名后对前端缓存的联动更新
//!
//! `AccountApp`（app.rs）持有本模块的类型与输入框实体。

use crate::server::models::{Counterparty, Currency, Money};
use serde::{Deserialize, Serialize};

/// 主界面页面
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    Records,
    Master,
    Stats,
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

/// 统计页分组视图
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectGroupView {
    pub name: String,
    pub currency: String,
    pub balance: Money,
    pub monthly: Vec<MonthStatView>,
}

/// 统计页单月明细
#[derive(Debug, Clone, PartialEq)]
pub struct MonthStatView {
    pub month: String,
    pub income: Option<Money>,
    pub expense: Option<Money>,
    pub settlement: Money,
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

/// 当前本地时间 `YYYY-MM-DDTHH:MM`（交易记录表单默认值）。
pub fn now_local_iso() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M").to_string()
}

/// 当前时间往前 7 天（记录页默认查询起点）。
pub fn now_minus_7d_iso() -> String {
    (chrono::Local::now() - chrono::Duration::days(7))
        .format("%Y-%m-%dT%H:%M")
        .to_string()
}

/// `datetime-local` 值补秒为后端 ISO 字符串。
pub fn build_iso(dt: &str) -> String {
    let dt = dt.trim();
    if dt.len() == 16 {
        format!("{dt}:00")
    } else {
        dt.to_string()
    }
}

/// 后端 ISO 时间转显示值（去掉秒）。
pub fn dt_local(v: &str) -> String {
    if v.len() >= 16 {
        v[..16].to_string()
    } else {
        v.to_string()
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

    /// 验证任务进度百分比与超界保护。
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
