//! 交易对象月度明细（按年分表）
//!
//! 每个交易对象 × 货币 × **有交易的月份** 一行，存放在对应年份的表：
//! `counterparty_monthly_YYYY`。
//!
//! 设计要点：
//! - **不为没有交易的月份建行**：交易对象多时不会产生「对象 × 月份」的空行。
//! - 年表按需创建；删除/回滚后若 `out == 0 && in == 0`，直接删除该行。
//! - 所有写操作由调用方在事务内调用，本模块不自行开启事务。

use super::{money_to_text, read_money};
use crate::server::models::{CounterpartyMonthlyQuery, CounterpartyMonthlyRow, Money};
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};

/// 从 `YYYY-MM` 取年份。
pub fn year_of(month: &str) -> Result<i32> {
    parse_month(month).map(|(year, _)| year)
}

/// 解析 `YYYY-MM`；非法输入返回错误。
fn parse_month(month: &str) -> Result<(i32, u32)> {
    let mut parts = month.trim().split('-');
    let year: i32 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| anyhow::anyhow!("月份格式不正确: {month}"))?;
    let month_no: u32 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| anyhow::anyhow!("月份格式不正确: {month}"))?;
    if parts.next().is_some() || !(1..=12).contains(&month_no) {
        return Err(anyhow::anyhow!("月份格式不正确: {month}"));
    }
    Ok((year, month_no))
}

/// 年份对应的年表名。
pub fn table_name(year: i32) -> String {
    format!("counterparty_monthly_{year:04}")
}

/// 创建年表（幂等）。
pub fn ensure_year_table(conn: &Connection, year: i32) -> Result<()> {
    let table = table_name(year);
    let index = format!("idx_cpm_{year:04}_cp_month");
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS `{table}` (
            counterparty_id INTEGER NOT NULL REFERENCES counterparties(id) ON DELETE CASCADE,
            month            TEXT    NOT NULL,
            currency_id      INTEGER NOT NULL REFERENCES currencies(id),
            out_value        TEXT    NOT NULL DEFAULT '0',
            out_divisor      TEXT    NOT NULL DEFAULT '1',
            in_value         TEXT    NOT NULL DEFAULT '0',
            in_divisor       TEXT    NOT NULL DEFAULT '1',
            PRIMARY KEY (counterparty_id, month, currency_id)
        );
        CREATE INDEX IF NOT EXISTS `{index}`
            ON `{table}`(counterparty_id, month);"
    ))
    .with_context(|| format!("创建年表 `{table}` 失败"))?;
    Ok(())
}

/// 列出已存在的年表年份（升序）。
pub fn existing_years(conn: &Connection) -> Result<Vec<i32>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master
         WHERE type='table' AND name LIKE 'counterparty_monthly_%' ORDER BY name",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for row in rows {
        let name = row?;
        if let Some(suffix) = name.strip_prefix("counterparty_monthly_")
            && let Ok(year) = suffix.parse::<i32>()
        {
            out.push(year);
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

/// 在某月的转出/转入上累加金额（增量为负即冲销）。
///
/// - 该月第一次写入时创建年表与行；
/// - 冲销后 `out` 与 `in` 均为 0 时删除该行（无交易不保留记录）。
pub fn add_amounts(
    conn: &Connection,
    counterparty_id: i64,
    currency_id: i64,
    month: &str,
    out_delta: Money,
    in_delta: Money,
) -> Result<()> {
    let year = year_of(month)?;
    ensure_year_table(conn, year)?;
    let table = table_name(year);

    let current: Option<(Money, Money)> = conn
        .query_row(
            &format!(
                "SELECT out_value, out_divisor, in_value, in_divisor
                 FROM `{table}`
                 WHERE counterparty_id = ?1 AND month = ?2 AND currency_id = ?3"
            ),
            rusqlite::params![counterparty_id, month, currency_id],
            |r| Ok((read_money(r, 0)?, read_money(r, 2)?)),
        )
        .optional()?;
    let (out, incoming) = current.unwrap_or((Money::ZERO, Money::ZERO));
    let out = out.checked_add(out_delta).map_err(|e| anyhow::anyhow!(e))?;
    let incoming = incoming
        .checked_add(in_delta)
        .map_err(|e| anyhow::anyhow!(e))?;

    if out.is_zero() && incoming.is_zero() {
        conn.execute(
            &format!(
                "DELETE FROM `{table}`
                 WHERE counterparty_id = ?1 AND month = ?2 AND currency_id = ?3"
            ),
            rusqlite::params![counterparty_id, month, currency_id],
        )?;
        return Ok(());
    }

    let (out_value, out_divisor) = money_to_text(out);
    let (in_value, in_divisor) = money_to_text(incoming);
    conn.execute(
        &format!(
            "INSERT INTO `{table}`
                (counterparty_id, month, currency_id, out_value, out_divisor, in_value, in_divisor)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(counterparty_id, month, currency_id) DO UPDATE SET
                out_value = excluded.out_value,
                out_divisor = excluded.out_divisor,
                in_value = excluded.in_value,
                in_divisor = excluded.in_divisor"
        ),
        rusqlite::params![
            counterparty_id,
            month,
            currency_id,
            out_value,
            out_divisor,
            in_value,
            in_divisor
        ],
    )
    .with_context(|| format!("写入年表 `{table}` 失败"))?;
    Ok(())
}

/// 查询若干交易对象的月度明细（跨年表，按对象、月份升序）。
pub fn list(
    conn: &Connection,
    query: &CounterpartyMonthlyQuery,
) -> Result<Vec<CounterpartyMonthlyRow>> {
    if query.counterparty_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut years = existing_years(conn)?;
    if let Some(from) = &query.from {
        let from_year = year_of(from)?;
        years.retain(|y| *y >= from_year);
    }
    if let Some(to) = &query.to {
        let to_year = year_of(to)?;
        years.retain(|y| *y <= to_year);
    }
    if years.is_empty() {
        return Ok(Vec::new());
    }

    let ids = query
        .counterparty_ids
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let mut selects = Vec::new();
    let mut args: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    for year in years {
        let table = table_name(year);
        let mut sql = format!(
            "SELECT counterparty_id, month, currency_id,
                    out_value, out_divisor, in_value, in_divisor
             FROM `{table}`
             WHERE counterparty_id IN ({ids})"
        );
        if let Some(from) = &query.from {
            sql.push_str(" AND month >= ?");
            args.push(Box::new(from.clone()));
        }
        if let Some(to) = &query.to {
            sql.push_str(" AND month <= ?");
            args.push(Box::new(to.clone()));
        }
        selects.push(sql);
    }
    let sql = format!(
        "{} ORDER BY counterparty_id, month",
        selects.join(" UNION ALL ")
    );

    let mut stmt = conn.prepare(&sql)?;
    let arg_refs: Vec<&dyn rusqlite::types::ToSql> = args.iter().map(|a| a.as_ref()).collect();
    let rows = stmt.query_map(arg_refs.as_slice(), |r| {
        Ok(CounterpartyMonthlyRow {
            counterparty_id: r.get(0)?,
            month: r.get(1)?,
            currency_id: r.get(2)?,
            out_amount: read_money(r, 3)?,
            in_amount: read_money(r, 5)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// 货币是否仍被任意年表引用。
pub fn currency_used(conn: &Connection, currency_id: i64) -> Result<bool> {
    for year in existing_years(conn)? {
        let table = table_name(year);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::db;

    fn setup() -> (Connection, i64, i64) {
        let mut conn = Connection::open_in_memory().expect("打开内存库");
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        db::init_schema(&conn).unwrap();
        let cid = db::add_currency(&conn, "RMB").unwrap();
        let cp = db::add_counterparty(&mut conn, "微信", cid, Money::ZERO).unwrap();
        (conn, cid, cp)
    }

    /// 验证只给有交易的月份建行：1 月与 3 月有行，2 月没有。
    #[test]
    fn add_amounts_should_only_create_transaction_months() {
        let (conn, cid, cp) = setup();
        add_amounts(&conn, cp, cid, "2026-01", Money::from(10), Money::ZERO).unwrap();
        add_amounts(&conn, cp, cid, "2026-03", Money::ZERO, Money::from(5)).unwrap();

        let rows = list(
            &conn,
            &CounterpartyMonthlyQuery {
                counterparty_ids: vec![cp],
                from: None,
                to: None,
            },
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].month, "2026-01");
        assert_eq!(rows[1].month, "2026-03");
        // 不相等断言：没有交易的 2 月不能出现
        assert!(!rows.iter().any(|r| r.month == "2026-02"));
    }

    /// 验证按年分表：2025-12 和 2026-01 落在不同年表。
    #[test]
    fn add_amounts_should_split_by_year() {
        let (conn, cid, cp) = setup();
        add_amounts(&conn, cp, cid, "2025-12", Money::from(1), Money::ZERO).unwrap();
        add_amounts(&conn, cp, cid, "2026-01", Money::from(2), Money::ZERO).unwrap();
        assert_eq!(existing_years(&conn).unwrap(), vec![2025, 2026]);
        let rows = list(
            &conn,
            &CounterpartyMonthlyQuery {
                counterparty_ids: vec![cp],
                from: None,
                to: None,
            },
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        // 不相等断言：两张年表不能合并成一张
        assert_ne!(existing_years(&conn).unwrap().len(), 1);
    }

    /// 验证冲销到 0 后删除该行（无交易不保留记录）。
    #[test]
    fn add_amounts_should_delete_zero_row() {
        let (conn, cid, cp) = setup();
        let ten = Money::from(10);
        add_amounts(&conn, cp, cid, "2026-05", ten, Money::ZERO).unwrap();
        add_amounts(
            &conn,
            cp,
            cid,
            "2026-05",
            ten.checked_neg().unwrap(),
            Money::ZERO,
        )
        .unwrap();
        let rows = list(
            &conn,
            &CounterpartyMonthlyQuery {
                counterparty_ids: vec![cp],
                from: None,
                to: None,
            },
        )
        .unwrap();
        assert!(rows.is_empty());
        // 不相等断言：不能留下零金额行
        assert_ne!(rows.len(), 1);
    }

    /// 验证跨年查询与月份过滤。
    #[test]
    fn list_should_filter_across_year_tables() {
        let (conn, cid, cp) = setup();
        add_amounts(&conn, cp, cid, "2025-12", Money::from(1), Money::ZERO).unwrap();
        add_amounts(&conn, cp, cid, "2026-01", Money::from(2), Money::ZERO).unwrap();
        let rows = list(
            &conn,
            &CounterpartyMonthlyQuery {
                counterparty_ids: vec![cp],
                from: Some("2026-01".into()),
                to: Some("2026-01".into()),
            },
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].month, "2026-01");
        assert_eq!(rows[0].out_amount, Money::from(2));
        // 不相等断言：2025-12 必须被过滤掉
        assert_ne!(rows[0].month, "2025-12");
    }

    /// 验证货币引用检测会扫描所有年表。
    #[test]
    fn currency_used_should_check_year_tables() {
        let (conn, cid, cp) = setup();
        assert!(!currency_used(&conn, cid).unwrap());
        add_amounts(&conn, cp, cid, "2026-01", Money::from(1), Money::ZERO).unwrap();
        assert!(currency_used(&conn, cid).unwrap());
        // 不相等断言：被引用货币与不存在的货币必须得到不同结果
        assert_ne!(
            currency_used(&conn, cid).unwrap(),
            currency_used(&conn, 999).unwrap()
        );
    }
}
