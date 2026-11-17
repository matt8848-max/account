//! 交易对象月度明细（`counterparty_monthly` 表）
//!
//! 每个交易对象 × 货币 × 月份一行，记录该月转出/转入金额。
//! 月份从对象的 `created_month` 开始**连续**，中间没有交易的月份写 0。
//!
//! 所有写操作都要求传入 `start_month`（对象起始月），由调用方在事务内调用；
//! 本模块不自行开启事务，避免与月表/余额的增删事务嵌套。

use super::{money_to_text, read_money};
use crate::server::models::{CounterpartyMonthlyQuery, CounterpartyMonthlyRow, Money};
use anyhow::{Context, Result};
use rusqlite::Connection;

/// 月份加一：`2026-12` → `2027-01`。
///
/// 仅接受 `YYYY-MM`；解析失败返回错误（不 panic），便于暴露脏数据。
pub fn next_month(month: &str) -> Result<String> {
    let (year, month_no) = parse_month(month)?;
    let (year, month_no) = if month_no == 12 {
        (year + 1, 1)
    } else {
        (year, month_no + 1)
    };
    Ok(format!("{year:04}-{month_no:02}"))
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

/// 保证 `[start_month, through_month]` 内每个月都有行（金额为 0 的空行）。
///
/// 使用 `INSERT OR IGNORE`，重复调用是幂等的；已有月份不会被覆盖。
/// `start_month > through_month` 时不做任何事。
pub fn ensure_month_rows(
    conn: &Connection,
    counterparty_id: i64,
    currency_id: i64,
    start_month: &str,
    through_month: &str,
) -> Result<()> {
    parse_month(start_month)?;
    parse_month(through_month)?;
    if start_month > through_month {
        return Ok(());
    }
    let mut month = start_month.to_string();
    loop {
        conn.execute(
            "INSERT OR IGNORE INTO counterparty_monthly
                (counterparty_id, month, currency_id, out_value, out_divisor, in_value, in_divisor)
             VALUES (?1, ?2, ?3, '0', '1', '0', '1')",
            rusqlite::params![counterparty_id, month, currency_id],
        )
        .with_context(|| format!("初始化对象月度明细失败: id={counterparty_id}, month={month}"))?;
        if month == through_month {
            break;
        }
        month = next_month(&month)?;
    }
    Ok(())
}

/// 在某月的转出/转入上累加金额（增量为负即冲销）。
///
/// `start_month` 为对象起始月；若交易月份早于它（脏数据），则把起始月
/// 临时前移到交易月份，保证该行仍可写入。
pub fn add_amounts(
    conn: &Connection,
    counterparty_id: i64,
    currency_id: i64,
    start_month: &str,
    month: &str,
    out_delta: Money,
    in_delta: Money,
) -> Result<()> {
    let start = if month < start_month {
        month
    } else {
        start_month
    };
    ensure_month_rows(conn, counterparty_id, currency_id, start, month)?;

    let (out, incoming) = {
        let mut stmt = conn.prepare(
            "SELECT out_value, out_divisor, in_value, in_divisor
             FROM counterparty_monthly
             WHERE counterparty_id = ?1 AND month = ?2 AND currency_id = ?3",
        )?;
        let mut rows = stmt.query(rusqlite::params![counterparty_id, month, currency_id])?;
        let row = rows
            .next()?
            .ok_or_else(|| anyhow::anyhow!("对象月度明细行不存在"))?;
        (read_money(row, 0)?, read_money(row, 2)?)
    };

    let out = out.checked_add(out_delta).map_err(|e| anyhow::anyhow!(e))?;
    let incoming = incoming
        .checked_add(in_delta)
        .map_err(|e| anyhow::anyhow!(e))?;
    let (out_value, out_divisor) = money_to_text(out);
    let (in_value, in_divisor) = money_to_text(incoming);
    conn.execute(
        "UPDATE counterparty_monthly
         SET out_value = ?4, out_divisor = ?5, in_value = ?6, in_divisor = ?7
         WHERE counterparty_id = ?1 AND month = ?2 AND currency_id = ?3",
        rusqlite::params![
            counterparty_id,
            month,
            currency_id,
            out_value,
            out_divisor,
            in_value,
            in_divisor
        ],
    )?;
    Ok(())
}

/// 查询若干交易对象的月度明细（按对象、月份升序）。
pub fn list(
    conn: &Connection,
    query: &CounterpartyMonthlyQuery,
) -> Result<Vec<CounterpartyMonthlyRow>> {
    if query.counterparty_ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids = query
        .counterparty_ids
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let mut sql = format!(
        "SELECT counterparty_id, month, currency_id,
                out_value, out_divisor, in_value, in_divisor
         FROM counterparty_monthly
         WHERE counterparty_id IN ({ids})"
    );
    let mut conds: Vec<String> = Vec::new();
    let mut args: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    if let Some(from) = &query.from {
        conds.push("month >= ?".into());
        args.push(Box::new(from.clone()));
    }
    if let Some(to) = &query.to {
        conds.push("month <= ?".into());
        args.push(Box::new(to.clone()));
    }
    if !conds.is_empty() {
        sql.push_str(" AND ");
        sql.push_str(&conds.join(" AND "));
    }
    sql.push_str(" ORDER BY counterparty_id, month");

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::db;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("打开内存库");
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        db::init_schema(&conn).unwrap();
        conn
    }

    /// 验证月份加一跨年正确：12 月 → 次年 1 月；非法月份报错。
    #[test]
    fn next_month_should_cross_year_boundary() {
        assert_eq!(next_month("2026-12").unwrap(), "2027-01");
        assert_eq!(next_month("2026-01").unwrap(), "2026-02");
        assert_ne!(next_month("2026-12").unwrap(), "2026-13");
        assert!(next_month("2026").is_err());
        assert!(next_month("2026-13").is_err());
        assert!(next_month("abc").is_err());
    }

    /// 验证空行补齐：从起始月到目标月每个月都有一行，金额为 0。
    #[test]
    fn ensure_month_rows_should_fill_every_month() {
        let mut conn = test_conn();
        let cid = db::add_currency(&conn, "RMB").unwrap();
        let cp = db::add_counterparty(&mut conn, "微信", cid, Money::ZERO).unwrap();
        ensure_month_rows(&conn, cp, cid, "2026-10", "2026-12").unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM counterparty_monthly
                 WHERE counterparty_id = ?1 AND month BETWEEN '2026-10' AND '2026-12'",
                [cp],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 3);
        // 不相等断言：不能多补到区间外
        assert_ne!(count, 2);
    }

    /// 验证累加：转出/转入分别累计，负数增量可冲销。
    #[test]
    fn add_amounts_should_accumulate_and_reverse() {
        let mut conn = test_conn();
        let cid = db::add_currency(&conn, "RMB").unwrap();
        let cp = db::add_counterparty(&mut conn, "微信", cid, Money::ZERO).unwrap();
        let ten = Money::from(10);
        add_amounts(&conn, cp, cid, "2026-10", "2026-10", ten, Money::ZERO).unwrap();
        add_amounts(&conn, cp, cid, "2026-10", "2026-10", ten, Money::from(5)).unwrap();

        let october = CounterpartyMonthlyQuery {
            counterparty_ids: vec![cp],
            from: Some("2026-10".into()),
            to: Some("2026-10".into()),
        };
        let rows = list(&conn, &october).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].out_amount, Money::from(20));
        assert_eq!(rows[0].in_amount, Money::from(5));

        add_amounts(
            &conn,
            cp,
            cid,
            "2026-10",
            "2026-10",
            ten.checked_neg().unwrap(),
            Money::ZERO,
        )
        .unwrap();
        let rows = list(&conn, &october).unwrap();
        assert_eq!(rows[0].out_amount, Money::from(10));
        // 不相等断言：冲销后不能仍是累计值
        assert_ne!(rows[0].out_amount, Money::from(20));
    }
}
