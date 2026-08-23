//! 月表数据访问
//!
//! 每个月的交易记录存储在独立的月表 `transactions_YYYY_MM` 中。

use crate::models::{MonthStats, Money, ObjectStatsQuery, RangeQuery, Record, RecordSubmit};
use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use super::{adjust_counterparty_balance, ensure_month_table, month_table_name};

/// 按一条记录调整两个交易对象余额：转出方减少、转入方增加
fn apply_balance(conn: &Connection, rec: &RecordSubmit) -> Result<()> {
    adjust_counterparty_balance(conn, rec.from_id, -rec.amount.value, rec.amount.divisor)?;
    adjust_counterparty_balance(conn, rec.to_id, rec.amount.value, rec.amount.divisor)?;
    Ok(())
}

/// 从 `traded_at` 提取月份 `YYYY-MM`
pub fn month_of(traded_at: &str) -> &str {
    // traded_at 形如 2026-08-14T10:30:00，取前 7 位
    traded_at.get(..7).unwrap_or(traded_at)
}

/// 校验备注长度（最多 256 字符）
pub fn validate_remark(remark: &Option<String>) -> Result<()> {
    if let Some(r) = remark
        && r.chars().count() > 256
    {
        return Err(anyhow::anyhow!("备注长度不能超过 256 个字符"));
    }
    Ok(())
}

/// 校验组名：必填且最多 64 字符
pub fn validate_group_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        return Err(anyhow::anyhow!("组名称不能为空"));
    }
    if name.chars().count() > 64 {
        return Err(anyhow::anyhow!("组名称长度不能超过 64 个字符"));
    }
    Ok(())
}

/// 校验交易记录金额必须为正数（方向由 from/to 决定，金额本身不允许为负）
pub fn validate_positive_amount(amount: &Money) -> Result<()> {
    if amount.value <= 0 {
        return Err(anyhow::anyhow!("交易金额必须为正数"));
    }
    Ok(())
}

/// 在事务中创建一个组并插入多条交易记录
///
/// 组内每条记录根据其 `traded_at` 所属月份写入对应月表（组 ID 全局唯一递增）。
pub fn add_group(conn: &mut Connection, name: &str, records: &[RecordSubmit]) -> Result<i64> {
    validate_group_name(name)?;
    if records.is_empty() {
        return Err(anyhow::anyhow!("组内至少需要一条记录"));
    }
    let tx = conn.transaction().context("开启事务失败")?;
    let group_name: Option<String> = if name.trim().is_empty() {
        None
    } else {
        Some(name.trim().to_string())
    };

    // 计算全局唯一组 ID：跨所有月表的最大 group_id + 1
    let group_id = {
        let mut max_id = 0i64;
        for table in existing_month_tables(&tx)? {
            let m: i64 = tx.query_row(
                &format!("SELECT COALESCE(MAX(group_id), 0) FROM `{table}`"),
                [],
                |r| r.get(0),
            )?;
            max_id = max_id.max(m);
        }
        max_id + 1
    };

    for rec in records {
        validate_remark(&rec.remark)?;
        validate_positive_amount(&rec.amount)?;
        // 币种一致性：转出与转入对象的货币必须相同，且金额货币一致
        let from_cur: i64 = tx.query_row(
            "SELECT currency_id FROM counterparties WHERE id = ?1",
            [rec.from_id],
            |r| r.get(0),
        ).with_context(|| format!("转出对象 id={} 不存在", rec.from_id))?;
        let to_cur: i64 = tx.query_row(
            "SELECT currency_id FROM counterparties WHERE id = ?1",
            [rec.to_id],
            |r| r.get(0),
        ).with_context(|| format!("转入对象 id={} 不存在", rec.to_id))?;
        if from_cur != to_cur {
            return Err(anyhow::anyhow!("跨币种转账不允许：转出货币与转入货币不同"));
        }
        if rec.currency_id != from_cur {
            return Err(anyhow::anyhow!("金额货币与交易对象货币不一致"));
        }
        let month = month_of(&rec.traded_at);
        let table = ensure_month_table(&tx, month)?;
        tx.execute(
            &format!(
                "INSERT INTO `{table}`
                 (group_id, group_name, from_id, to_id, amount_value, amount_divisor, currency_id, remark, traded_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
            ),
            params![
                group_id,
                group_name,
                rec.from_id,
                rec.to_id,
                rec.amount.value,
                rec.amount.divisor,
                rec.currency_id,
                rec.remark,
                rec.traded_at
            ],
        )
        .with_context(|| format!("插入月表 `{table}` 记录失败"))?;
        // 余额联动：转出方减少、转入方增加
        apply_balance(&tx, rec)?;
    }
    tx.commit().context("提交事务失败")?;
    Ok(group_id)
}

/// 往已存在的组内添加一条记录
pub fn add_record(
    conn: &Connection,
    month: &str,
    group_id: i64,
    rec: &RecordSubmit,
) -> Result<i64> {
    validate_remark(&rec.remark)?;
    validate_positive_amount(&rec.amount)?;
    let table = ensure_month_table(conn, month)?;
    // 从已有记录中取组名（无则 NULL）
    let group_name: Option<String> = conn
        .query_row(
            &format!("SELECT group_name FROM `{table}` WHERE group_id = ?1 LIMIT 1"),
            [group_id],
            |r| r.get(0),
        )
        .unwrap_or(None);
    conn.execute(
        &format!(
            "INSERT INTO `{table}`
             (group_id, group_name, from_id, to_id, amount_value, amount_divisor, currency_id, remark, traded_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
        ),
        params![
            group_id,
            group_name,
            rec.from_id,
            rec.to_id,
            rec.amount.value,
            rec.amount.divisor,
            rec.currency_id,
            rec.remark,
            rec.traded_at
        ],
    )
    .with_context(|| format!("插入月表 `{table}` 记录失败"))?;
    // 余额联动
    apply_balance(conn, rec)?;
    Ok(conn.last_insert_rowid())
}

/// 删除一个组及其全部记录（跨所有月表）
pub fn delete_group(conn: &Connection, group_id: i64) -> Result<usize> {
    let mut deleted = 0usize;
    for table in existing_month_tables(conn)? {
        // 删除前取出记录，用于反转余额
        let mut stmt = conn.prepare(&format!(
            "SELECT from_id, to_id, amount_value, amount_divisor
             FROM `{table}` WHERE group_id = ?1"
        ))?;
        let rows = stmt.query_map([group_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, u32>(3)?))
        })?;
        for row in rows {
            let (from_id, to_id, value, divisor) = row?;
            // 反转：转出方加回、转入方减回
            adjust_counterparty_balance(conn, from_id, value, divisor)?;
            adjust_counterparty_balance(conn, to_id, -value, divisor)?;
        }
        let n = conn.execute(
            &format!("DELETE FROM `{table}` WHERE group_id = ?1"),
            [group_id],
        )?;
        deleted += n;
    }
    Ok(deleted)
}

/// 删除一条交易记录
pub fn delete_record(conn: &Connection, month: &str, record_id: i64) -> Result<bool> {
    let table = match ensure_month_table(conn, month) {
        Ok(t) => t,
        Err(_) => return Ok(false),
    };
    // 删除前取出记录，用于反转余额
    let rec: Option<(i64, i64, i64, u32)> = conn
        .query_row(
            &format!(
                "SELECT from_id, to_id, amount_value, amount_divisor
                 FROM `{table}` WHERE id = ?1"
            ),
            [record_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .ok();
    let n = conn.execute(
        &format!("DELETE FROM `{table}` WHERE id = ?1"),
        [record_id],
    )?;
    if n > 0
        && let Some((from_id, to_id, value, divisor)) = rec
    {
        // 反转：转出方加回、转入方减回
        adjust_counterparty_balance(conn, from_id, value, divisor)?;
        adjust_counterparty_balance(conn, to_id, -value, divisor)?;
    }
    Ok(n > 0)
}

/// 查询指定时间范围（可选交易对象过滤）的所有记录，跨月表
pub fn list_records(conn: &Connection, query: &RangeQuery) -> Result<Vec<Record>> {
    let tables = existing_month_tables(conn)?;
    let mut out = Vec::new();
    for table in tables {
        let mut sql = format!(
            "SELECT id, group_id, group_name, from_id, to_id, amount_value, amount_divisor,
                    currency_id, remark, traded_at FROM `{table}` WHERE 1=1"
        );
        let mut conds: Vec<String> = Vec::new();
        let mut args: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        if let Some(from) = &query.from {
            conds.push("traded_at >= ?".into());
            args.push(Box::new(from.clone()));
        }
        if let Some(to) = &query.to {
            conds.push("traded_at <= ?".into());
            args.push(Box::new(to.clone()));
        }
        if let Some(id) = query.counterparty_id {
            conds.push("(from_id = ? OR to_id = ?)".into());
            args.push(Box::new(id));
            args.push(Box::new(id));
        }
        if !conds.is_empty() {
            sql.push_str(" AND ");
            sql.push_str(&conds.join(" AND "));
        }
        sql.push_str(" ORDER BY traded_at");

        let mut stmt = conn.prepare(&sql)?;
        let arg_refs: Vec<&dyn rusqlite::types::ToSql> =
            args.iter().map(|a| a.as_ref()).collect();
        let rows = stmt.query_map(arg_refs.as_slice(), |r| {
            Ok(Record {
                id: r.get(0)?,
                group_id: r.get(1)?,
                group_name: r.get(2)?,
                from_id: r.get(3)?,
                to_id: r.get(4)?,
                amount: Money {
                    value: r.get(5)?,
                    divisor: r.get(6)?,
                },
                currency_id: r.get(7)?,
                remark: r.get(8)?,
                traded_at: r.get(9)?,
            })
        })?;
        for row in rows {
            out.push(row?);
        }
    }
    Ok(out)
}

/// 校验一个组提交的记录来源（供 API 层调用，跨币种转账拒绝在 add_group 内进行）
pub fn validate_no_cross_currency(conn: &Connection, records: &[RecordSubmit]) -> Result<()> {
    for rec in records {
        validate_positive_amount(&rec.amount)?;
        let from_cur: i64 = conn.query_row(
            "SELECT currency_id FROM counterparties WHERE id = ?1",
            [rec.from_id],
            |r| r.get(0),
        ).with_context(|| format!("转出对象 id={} 不存在", rec.from_id))?;
        let to_cur: i64 = conn.query_row(
            "SELECT currency_id FROM counterparties WHERE id = ?1",
            [rec.to_id],
            |r| r.get(0),
        ).with_context(|| format!("转入对象 id={} 不存在", rec.to_id))?;
        if from_cur != to_cur {
            return Err(anyhow::anyhow!(
                "跨币种转账不允许：转出货币 id={from_cur} 与转入货币 id={to_cur} 不同"
            ));
        }
        if rec.currency_id != from_cur {
            return Err(anyhow::anyhow!("金额货币与交易对象货币不一致"));
        }
    }
    Ok(())
}

/// 对象统计条目： (对象ID, 对象名, "income:RMB" 或 "expense:RMB", 金额)
pub type ObjectStatRow = (i64, String, String, Money);
/// 月份 → 对象统计条目列表
pub type ObjectStatMonths = Vec<(String, Vec<ObjectStatRow>)>;

/// 按指定交易对象集合统计月范围的收支。
///
/// 对每个涉及这些对象（作为转出或转入方）的记录，
/// 转出侧记支出、转入侧记收入，按 (对象 ID, 货币名) 分组。
/// 返回月份 → 按对象分别的收入/支出列表。
pub fn compute_object_stats(conn: &Connection, query: &ObjectStatsQuery) -> Result<ObjectStatMonths> {
    let tables = existing_month_tables(conn)?;
    let ids = &query.counterparty_ids;
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    // 构建月份过滤
    let mut months: Vec<String> = tables
        .iter()
        .filter_map(|t| t.strip_prefix("transactions_").map(|s| s.replace('_', "-")))
        .collect();
    months.retain(|m| {
        let in_from = query.from.as_ref().is_none_or(|f| m >= f);
        let in_to = query.to.as_ref().is_none_or(|t| m <= t);
        in_from && in_to
    });
    months.sort();

    let id_list = ids
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");

    // 对象 + 货币名
    let mut cp_names: std::collections::HashMap<i64, (String, String)> = std::collections::HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT c.id, c.name, cu.name FROM counterparties c
             JOIN currencies cu ON cu.id = c.currency_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
        })?;
        for row in rows {
            let (id, name, cur) = row?;
            cp_names.insert(id, (name, cur));
        }
    }

    let mut out: ObjectStatMonths = Vec::new();
    for month in months {
        let table = month_table_name(&month);
        let sql = format!(
            "SELECT from_id, to_id, amount_value, amount_divisor FROM `{table}`
             WHERE from_id IN ({id_list}) OR to_id IN ({id_list})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                // 金额语义为正：方向由 from/to 决定（转出=支出、转入=收入），取绝对值避免符号反
                Money { value: r.get::<_, i64>(2)?.abs(), divisor: r.get(3)? },
            ))
        })?;
        // (object_id, 方向 income/expense, 货币名, 金额累计)
        let mut by_obj: std::collections::HashMap<i64, Vec<(String, String, Money)>> =
            std::collections::HashMap::new();
        for row in rows {
            let (from_id, to_id, amount) = row?;
            // 转出侧记支出
            if ids.contains(&from_id)
                && let Some((_, cur)) = cp_names.get(&from_id)
            {
                by_obj.entry(from_id).or_default().push(("expense".into(), cur.clone(), amount));
            }
            // 转入侧记收入
            if ids.contains(&to_id)
                && let Some((_, cur)) = cp_names.get(&to_id)
            {
                by_obj.entry(to_id).or_default().push(("income".into(), cur.clone(), amount));
            }
        }
        let mut obj_vec: Vec<(i64, String, String, Money)> = Vec::new();
        for (obj_id, items) in by_obj {
            // 汇总同方向同货币金额
            let mut income_map: std::collections::HashMap<String, Money> = std::collections::HashMap::new();
            let mut expense_map: std::collections::HashMap<String, Money> = std::collections::HashMap::new();
            for (dir, cur, m) in items {
                let map = if dir == "income" { &mut income_map } else { &mut expense_map };
                if let Some(existing) = map.get_mut(&cur) {
                    // 按最大 divisor 对齐累加
                    let divisor = existing.divisor.max(m.divisor);
                    let scale = |v: i64, from: u32, to: u32| -> i64 {
                        if from == to { v } else { v * (to / from) as i64 }
                    };
                    let new_value = scale(existing.value, existing.divisor, divisor)
                        + scale(m.value, m.divisor, divisor);
                    *existing = Money { value: new_value, divisor };
                } else {
                    map.insert(cur.clone(), m);
                }
            }
            let name = cp_names.get(&obj_id).map(|(n, _)| n.clone()).unwrap_or_else(|| format!("#{obj_id}"));
            // 先输出收入方向货币汇总，再支出方向
            for (cur, m) in income_map {
                obj_vec.push((obj_id, name.clone(), format!("income:{cur}"), m));
            }
            for (cur, m) in expense_map {
                obj_vec.push((obj_id, name.clone(), format!("expense:{cur}"), m));
            }
        }
        obj_vec.sort_by_key(|a| (a.0, a.2.clone()));
        out.push((month, obj_vec));
    }
    Ok(out)
}

/// 统计某个月的收支（按货币分别统计）
///
/// 收入 = 转入方向金额合计，支出 = 转出方向金额合计。
pub fn compute_month_stats(conn: &Connection, month: &str) -> Result<MonthStats> {
    let table = month_table_name(month);
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [&table],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(MonthStats {
            month: month.to_string(),
            income: Vec::new(),
            expense: Vec::new(),
        });
    }

    // 查询：转入对象的货币作为收入，转出对象的货币作为支出
    // 一条记录：from 转出 (支出) → to 转入 (收入)
    let mut stmt = conn.prepare(&format!(
        "SELECT icu.name, ocu.name, t.amount_value, t.amount_divisor
         FROM `{table}` t
         JOIN counterparties ic ON ic.id = t.to_id
         JOIN currencies icu ON icu.id = ic.currency_id
         JOIN counterparties oc ON oc.id = t.from_id
         JOIN currencies ocu ON ocu.id = oc.currency_id"
    ))?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?, // 收入货币名（转入对象的货币）
            r.get::<_, String>(1)?, // 支出货币名（转出对象的货币）
            // 金额语义为正：方向由 from/to 决定，取绝对值避免符号反
            Money {
                value: r.get::<_, i64>(2)?.abs(),
                divisor: r.get(3)?,
            },
        ))
    })?;

    let mut income_map: Vec<(String, i64, u32)> = Vec::new(); // (货币名, value, divisor)
    let mut expense_map: Vec<(String, i64, u32)> = Vec::new();

    for row in rows {
        let (income_currency, expense_currency, money) = row?;
        // 转入方的货币为收入
        upsert_sum(&mut income_map, income_currency, money);
        // 转出方的货币为支出
        upsert_sum(&mut expense_map, expense_currency, money);
    }

    Ok(MonthStats {
        month: month.to_string(),
        income: income_map
            .into_iter()
            .map(|(n, v, d)| (n, Money { value: v, divisor: d }))
            .collect(),
        expense: expense_map
            .into_iter()
            .map(|(n, v, d)| (n, Money { value: v, divisor: d }))
            .collect(),
    })
}

/// 定点数累加，按最大 divisor 对齐
fn upsert_sum(map: &mut Vec<(String, i64, u32)>, name: String, money: Money) {
    if let Some(entry) = map.iter_mut().find(|(n, _, _)| *n == name) {
        let divisor = entry.2.max(money.divisor);
        let scale = |value: i64, from: u32, to: u32| -> i64 {
            if from == to {
                value
            } else {
                value * (to / from) as i64
            }
        };
        entry.0.clone_from(&name);
        entry.1 = scale(entry.1, entry.2, divisor) + scale(money.value, money.divisor, divisor);
        entry.2 = divisor;
    } else {
        map.push((name, money.value, money.divisor));
    }
}

/// 列出所有已存在的月表名
fn existing_month_tables(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master
         WHERE type='table' AND name LIKE 'transactions_%' ORDER BY name",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// 检查货币是否被任何月表记录使用
pub fn currency_used(conn: &Connection, currency_id: i64) -> Result<bool> {
    for table in existing_month_tables(conn)? {
        let used: bool = conn.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM `{table}` WHERE currency_id = ?1)"),
            [currency_id],
            |r| r.get(0),
        )?;
        if used {
            return Ok(true);
        }
    }
    Ok(false)
}

/// 检查交易对象是否被任何月表记录使用
pub fn counterparty_used(conn: &Connection, counterparty_id: i64) -> Result<bool> {
    for table in existing_month_tables(conn)? {
        let used: bool = conn.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM `{table}` WHERE from_id = ?1 OR to_id = ?1)"
            ),
            [counterparty_id],
            |r| r.get(0),
        )?;
        if used {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CounterpartySubmit, CurrencySubmit};

    fn setup() -> Connection {
        // 内存库：避免产生临时文件垃圾，且无并行锁冲突
        let conn = Connection::open_in_memory().expect("打开内存库");
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS total_stats (
                 month      TEXT PRIMARY KEY,
                 table_name TEXT NOT NULL,
                 stats      TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS currencies (
                 id   INTEGER PRIMARY KEY AUTOINCREMENT,
                 name TEXT NOT NULL UNIQUE
             );
             CREATE TABLE IF NOT EXISTS counterparties (
                 id            INTEGER PRIMARY KEY AUTOINCREMENT,
                 name          TEXT NOT NULL,
                 currency_id   INTEGER NOT NULL REFERENCES currencies(id),
                 balance_value INTEGER NOT NULL DEFAULT 0,
                 balance_divisor INTEGER NOT NULL DEFAULT 1,
                 UNIQUE(name, currency_id)
             );",
        )
        .expect("初始化内存库 schema");
        // 添加基础数据
        let cid = crate::db::add_currency(&conn, "RMB").unwrap();
        let cid2 = crate::db::add_currency(&conn, "HK").unwrap();
        let _p1 = crate::db::add_counterparty(&conn, "微信", cid, Money::from_int(0)).unwrap();
        let _p2 = crate::db::add_counterparty(&conn, "银行卡", cid, Money::from_int(0)).unwrap();
        let _p3 = crate::db::add_counterparty(&conn, "港币钱包", cid2, Money::from_int(0)).unwrap();
        let _ = CounterpartySubmit {
            name: "x".into(),
            currency_id: cid,
            balance: None,
        };
        let _ = CurrencySubmit { name: "x".into() };
        conn
    }

    #[test]
    fn month_of_parses() {
        assert_eq!(month_of("2026-08-14T10:30:00"), "2026-08");
    }

    #[test]
    fn validate_remark_length() {
        assert!(validate_remark(&None).is_ok());
        assert!(validate_remark(&Some("短备注".into())).is_ok());
        let long = "长".repeat(257);
        assert!(validate_remark(&Some(long)).is_err());
    }

    #[test]
    fn group_name_required() {
        // 组名必填：空名与纯空白名均被拒绝
        assert!(validate_group_name("").is_err());
        assert!(validate_group_name("   ").is_err());
        assert!(validate_group_name("测试组").is_ok());
        // 65 个字符超限；64 个正好通过（按字符数而非字节数统计）
        let long = "组".repeat(65);
        assert!(validate_group_name(&long).is_err());
        let ok_name = "组".repeat(64);
        assert!(validate_group_name(&ok_name).is_ok());
    }

    #[test]
    fn add_group_inserts_records() {
        let mut conn = setup();
        // 注意：内存库不可行，setup 用的临时文件；此处确保表存在
        let _ = ensure_month_table(&conn, "2026-08").unwrap();
        let recs = vec![RecordSubmit {
            from_id: 1,
            to_id: 2,
            amount: Money::from_int(100),
            currency_id: 1,
            remark: Some("测试".into()),
            traded_at: "2026-08-14T10:30:00".into(),
        }];
        let gid = add_group(&mut conn, "测试组", &recs).unwrap();
        assert!(gid > 0);
        let list = list_records(
            &conn,
            &RangeQuery {
                from: None,
                to: None,
                counterparty_id: None,
            },
        )
        .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].group_id, gid);
    }

    #[test]
    fn cross_currency_transfer_rejected() {
        let mut conn = setup();
        // 微信(id=1) RMB, 港币钱包(id=3) HK：跨币种转账应被拒绝
        let recs = vec![RecordSubmit {
            from_id: 1,
            to_id: 3,
            amount: Money::from_int(100),
            currency_id: 1,
            remark: None,
            traded_at: "2026-08-02T08:00:00".into(),
        }];
        let err = add_group(&mut conn, "跨币种", &recs).unwrap_err();
        assert!(err.to_string().contains("跨币种"));
    }

    #[test]
    fn compute_stats_by_currency() {
        let mut conn = setup();
        let _ = ensure_month_table(&conn, "2026-08").unwrap();
        let recs = vec![RecordSubmit {
            from_id: 1,
            to_id: 2,
            amount: Money::from_int(300),
            currency_id: 1,
            remark: None,
            traded_at: "2026-08-02T08:00:00".into(),
        }];
        add_group(&mut conn, "测试组", &recs).unwrap();
        let stats = compute_month_stats(&conn, "2026-08").unwrap();
        assert_eq!(stats.income.len(), 1);
        assert_eq!(stats.income[0].0, "RMB");
        assert_eq!(stats.income[0].1, Money::from_int(300));
        assert_eq!(stats.expense, stats.income);
    }

    /// 交易金额必须为正：负数金额应被拒绝
    #[test]
    fn negative_amount_rejected() {
        let mut conn = setup();
        let recs = vec![rec(1, 2, Money::from_int(-100), "2026-08-14T10:30:00")];
        let err = add_group(&mut conn, "负数组", &recs).unwrap_err();
        assert!(err.to_string().contains("正数"));
        // 初始余额允许负数（交易对象初始化场景）
        let cid = crate::db::add_currency(&conn, "USD").unwrap();
        assert!(crate::db::add_counterparty(
            &conn,
            "负债对象",
            cid,
            Money::from_int(-50),
        )
        .is_ok());
    }

    /// 金额语义为正：统计时对历史负数金额取绝对值（防御历史脏数据），
    /// 方向由 from（转出=支出）/ to（转入=收入）决定
    #[test]
    fn stats_with_negative_amount_are_positive() {
        let conn = setup();
        let table = ensure_month_table(&conn, "2026-08").unwrap();
        // 手工 SQL 插入负数金额记录，模拟早期无校验时的历史数据：微信(1)→银行卡(2)，金额 -100
        conn.execute(
            &format!(
                "INSERT INTO `{table}`
                 (group_id, group_name, from_id, to_id, amount_value, amount_divisor, currency_id, remark, traded_at)
                 VALUES (1, NULL, 1, 2, -100, 1, 1, NULL, '2026-08-02T08:00:00')"
            ),
            [],
        )
        .unwrap();

        // 月度统计：收入/支出都应取绝对值 100
        let stats = compute_month_stats(&conn, "2026-08").unwrap();
        assert_eq!(stats.income[0].1, Money::from_int(100));
        assert_eq!(stats.expense[0].1, Money::from_int(100));

        // 对象统计：微信支出 100、银行卡收入 100（均为正）
        let obj = compute_object_stats(
            &conn,
            &ObjectStatsQuery {
                counterparty_ids: vec![1, 2],
                from: None,
                to: None,
            },
        )
        .unwrap();
        let (_, rows) = &obj[0];
        let find = |tag: &str| {
            rows.iter()
                .find(|(_, _, t, _)| t == tag)
                .map(|(_, _, _, m)| m.value)
                .unwrap()
        };
        assert_eq!(find(&format!("income:RMB")), 100);
        assert_eq!(find(&format!("expense:RMB")), 100);
    }

    // ---------- 余额自动联动测试 ----------
    //
    // 验证问题清单第 1 项：增删记录/组时交易对象余额会自动更新，
    // 因此前端「更新总表统计」按钮冗余、已被注释掉。
    // 以下测试确保联动逻辑正确：转出减、转入加；删除时反向回滚。

    /// 读取指定交易对象的当前余额
    fn balance_of(conn: &Connection, id: i64) -> Money {
        crate::db::list_counterparties(conn)
            .unwrap()
            .into_iter()
            .find(|c| c.id == id)
            .expect("交易对象存在")
            .balance
    }

    /// 构造一条 RMB 记录（货币 id=1）
    fn rec(from_id: i64, to_id: i64, amount: Money, traded_at: &str) -> RecordSubmit {
        RecordSubmit {
            from_id,
            to_id,
            amount,
            currency_id: 1,
            remark: None,
            traded_at: traded_at.into(),
        }
    }

    #[test]
    fn add_group_updates_balances() {
        let mut conn = setup();
        let recs = vec![rec(1, 2, Money::from_int(100), "2026-08-14T10:30:00")];
        add_group(&mut conn, "测试组", &recs).unwrap();
        // 微信(1) 转出 100 → -100；银行卡(2) 转入 100 → +100
        assert_eq!(balance_of(&conn, 1), Money::from_int(-100));
        assert_eq!(balance_of(&conn, 2), Money::from_int(100));
        // 无关对象（港币钱包）余额不受影响
        assert_eq!(balance_of(&conn, 3), Money::from_int(0));
    }

    #[test]
    fn add_record_updates_balances() {
        let mut conn = setup();
        let recs = vec![rec(1, 2, Money::from_int(50), "2026-08-14T10:30:00")];
        let gid = add_group(&mut conn, "测试组", &recs).unwrap();
        // 往组内追加一条反向记录
        add_record(
            &conn,
            "2026-08",
            gid,
            &rec(2, 1, Money::from_int(30), "2026-08-20T12:00:00"),
        )
        .unwrap();
        // 微信: -50 + 30 = -20；银行卡: +50 - 30 = +20
        assert_eq!(balance_of(&conn, 1), Money::from_int(-20));
        assert_eq!(balance_of(&conn, 2), Money::from_int(20));
    }

    #[test]
    fn delete_record_restores_balances() {
        let mut conn = setup();
        let recs = vec![rec(1, 2, Money::from_int(100), "2026-08-14T10:30:00")];
        add_group(&mut conn, "测试组", &recs).unwrap();
        let list = list_records(&conn, &RangeQuery::default()).unwrap();
        let rid = list[0].id;
        assert!(delete_record(&conn, "2026-08", rid).unwrap());
        // 删除后余额回滚为初始值 0
        assert_eq!(balance_of(&conn, 1), Money::from_int(0));
        assert_eq!(balance_of(&conn, 2), Money::from_int(0));
    }

    #[test]
    fn delete_group_restores_balances() {
        let mut conn = setup();
        let recs = vec![
            rec(1, 2, Money::from_int(100), "2026-08-14T10:30:00"),
            rec(2, 1, Money::from_int(25), "2026-08-15T11:00:00"),
        ];
        let gid = add_group(&mut conn, "测试组", &recs).unwrap();
        // 中间状态：微信 -75、银行卡 +75
        assert_eq!(balance_of(&conn, 1), Money::from_int(-75));
        assert_eq!(balance_of(&conn, 2), Money::from_int(75));
        let n = delete_group(&conn, gid).unwrap();
        assert_eq!(n, 2);
        // 组删除后余额全部回滚为 0
        assert_eq!(balance_of(&conn, 1), Money::from_int(0));
        assert_eq!(balance_of(&conn, 2), Money::from_int(0));
    }

    #[test]
    fn delete_group_across_months_restores_balances() {
        let mut conn = setup();
        // 同一组两条记录分属 2026-07 与 2026-08 两个月表
        let recs = vec![
            rec(1, 2, Money::from_int(10), "2026-07-31T23:00:00"),
            rec(1, 2, Money::from_int(20), "2026-08-01T08:00:00"),
        ];
        let gid = add_group(&mut conn, "跨月组", &recs).unwrap();
        assert_eq!(balance_of(&conn, 1), Money::from_int(-30));
        assert_eq!(balance_of(&conn, 2), Money::from_int(30));
        let n = delete_group(&conn, gid).unwrap();
        assert_eq!(n, 2);
        // 跨两个月表的记录都要回滚
        assert_eq!(balance_of(&conn, 1), Money::from_int(0));
        assert_eq!(balance_of(&conn, 2), Money::from_int(0));
    }

    #[test]
    fn balances_with_mixed_precision() {
        let mut conn = setup();
        // 小数金额（100.50 → 10050/100）
        let recs = vec![rec(
            1,
            2,
            Money { value: 10050, divisor: 100 },
            "2026-08-14T10:30:00",
        )];
        add_group(&mut conn, "小数组", &recs).unwrap();
        assert_eq!(balance_of(&conn, 1), Money { value: -10050, divisor: 100 });
        // 再转整数 3：对齐到 divisor=100 后 -10050 - 300 = -10350（即 -103.50）
        let recs2 = vec![rec(1, 2, Money::from_int(3), "2026-08-15T10:30:00")];
        add_group(&mut conn, "整数组", &recs2).unwrap();
        assert_eq!(balance_of(&conn, 1), Money { value: -10350, divisor: 100 });
        assert_eq!(balance_of(&conn, 2), Money { value: 10350, divisor: 100 });
    }
}
