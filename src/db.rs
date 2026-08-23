//! SQLite 数据访问层
//!
//! 数据库按月分表：
//! - 总表 `total_stats` — 每月汇总与月表名
//! - 月表 `transactions_YYYY_MM` — 该月交易记录（组与记录同表）
//! - `currencies` — 货币表
//! - `counterparties` — 交易对象表

use crate::models::{MonthMeta, MonthStats};
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;

pub mod month;

/// 数据库文件名
pub const DB_FILE: &str = "account.db";

/// 数据库 Schema 当前版本
pub const DB_VERSION: i64 = 1;

/// 打开（或创建）数据库并初始化表结构
pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path).context("打开数据库失败")?;
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;",
    )
    .context("设置数据库参数失败")?;
    init_schema(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

/// 数据库版本迁移。
///
/// 早期数据库没有版本控制，视为版本 0（即"最开始忘了版本控制的版本"）。
/// 依次执行各版本迁移，最终将 `PRAGMA user_version` 更新为当前版本号。
pub fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .context("读取数据库版本失败")?;
    // 版本 0 → 1：无实际结构变更，仅补齐版本号
    if version < DB_VERSION {
        conn.execute_batch(&format!("PRAGMA user_version = {DB_VERSION};"))
            .context("写入数据库版本号失败")?;
        log::info!("数据库版本迁移: {version} → {DB_VERSION}");
    }
    Ok(())
}

/// 查询当前数据库 schema 版本
pub fn db_version(conn: &Connection) -> Result<i64> {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .context("读取数据库版本失败")
}

/// 初始化表结构
fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS total_stats (
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
    .context("初始化表结构失败")?;
    Ok(())
}

/// 返回某月份的月表名
pub fn month_table_name(month: &str) -> String {
    format!("transactions_{}", month.replace('-', "_"))
}

/// 确保某月份的月表存在
pub fn ensure_month_table(conn: &Connection, month: &str) -> Result<String> {
    let table = month_table_name(month);
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS `{table}` (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            group_id       INTEGER NOT NULL,
            group_name     TEXT,
            from_id        INTEGER NOT NULL REFERENCES counterparties(id),
            to_id          INTEGER NOT NULL REFERENCES counterparties(id),
            amount_value   INTEGER NOT NULL,
            amount_divisor INTEGER NOT NULL,
            currency_id    INTEGER NOT NULL REFERENCES currencies(id),
            remark         TEXT,
            traded_at      TEXT NOT NULL
        );"
    ))
    .with_context(|| format!("创建月表 `{table}` 失败"))?;
    // 旧月表可能无 group_name 列，补上
    let has: bool = conn
        .query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('{table}') WHERE name='group_name')"
            ),
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if !has {
        conn.execute_batch(&format!("ALTER TABLE `{table}` ADD COLUMN group_name TEXT;"))
            .with_context(|| format!("月表 `{table}` 补加 group_name 失败"))?;
    }
    Ok(table)
}

// ---------- 货币 ----------

/// 添加货币
pub fn add_currency(conn: &Connection, name: &str) -> Result<i64> {
    conn.execute("INSERT INTO currencies (name) VALUES (?1)", [name])?;
    Ok(conn.last_insert_rowid())
}

/// 列出全部货币
pub fn list_currencies(conn: &Connection) -> Result<Vec<crate::models::Currency>> {
    let mut stmt = conn.prepare("SELECT id, name FROM currencies ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        Ok(crate::models::Currency {
            id: r.get(0)?,
            name: r.get(1)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// 删除货币（必须未被交易对象或任何记录引用）
pub fn delete_currency(conn: &Connection, id: i64) -> Result<bool> {
    let used_in_counterparty: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM counterparties WHERE currency_id = ?1)",
        [id],
        |r| r.get(0),
    )?;
    let used_in_records: bool = month::currency_used(conn, id)?;
    if used_in_counterparty || used_in_records {
        return Ok(false);
    }
    let n = conn.execute("DELETE FROM currencies WHERE id = ?1", [id])?;
    Ok(n > 0)
}

// ---------- 交易对象 ----------

/// 添加交易对象
pub fn add_counterparty(
    conn: &Connection,
    name: &str,
    currency_id: i64,
    balance: crate::models::Money,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO counterparties (name, currency_id, balance_value, balance_divisor)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![name, currency_id, balance.value, balance.divisor],
    )?;
    Ok(conn.last_insert_rowid())
}

/// 列出全部交易对象
pub fn list_counterparties(conn: &Connection) -> Result<Vec<crate::models::Counterparty>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, currency_id, balance_value, balance_divisor
         FROM counterparties ORDER BY id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(crate::models::Counterparty {
            id: r.get(0)?,
            name: r.get(1)?,
            currency_id: r.get(2)?,
            balance: crate::models::Money {
                value: r.get(3)?,
                divisor: r.get(4)?,
            },
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// 调整交易对象余额（delta 为定点数差值：转出为负、转入为正）
pub fn adjust_counterparty_balance(
    conn: &Connection,
    id: i64,
    delta_value: i64,
    delta_divisor: u32,
) -> Result<()> {
    // 读取当前余额，按最大 divisor 对齐后相加，再写回
    let (cur_value, cur_divisor): (i64, u32) = conn.query_row(
        "SELECT balance_value, balance_divisor FROM counterparties WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let divisor = cur_divisor.max(delta_divisor);
    let scale = |v: i64, from: u32, to: u32| -> i64 {
        if from == to { v } else { v * (to / from) as i64 }
    };
    let new_value =
        scale(cur_value, cur_divisor, divisor) + scale(delta_value, delta_divisor, divisor);
    conn.execute(
        "UPDATE counterparties SET balance_value = ?2, balance_divisor = ?3 WHERE id = ?1",
        rusqlite::params![id, new_value, divisor],
    )?;
    Ok(())
}

/// 删除交易对象（必须未被任何记录使用）
pub fn delete_counterparty(conn: &Connection, id: i64) -> Result<bool> {
    if month::counterparty_used(conn, id)? {
        return Ok(false);
    }
    let n = conn.execute("DELETE FROM counterparties WHERE id = ?1", [id])?;
    Ok(n > 0)
}

// ---------- 总表 ----------

/// 写入（或更新）一个月的统计
pub fn upsert_month_stats(conn: &Connection, meta: &MonthMeta) -> Result<()> {
    let stats = serde_json::to_string(&meta.stats)?;
    conn.execute(
        "INSERT INTO total_stats (month, table_name, stats)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(month) DO UPDATE SET
           table_name = excluded.table_name,
           stats = excluded.stats",
        rusqlite::params![meta.month, meta.table_name, stats],
    )?;
    Ok(())
}

/// 查询全部月份统计（按月份排序）
pub fn list_month_stats(conn: &Connection) -> Result<Vec<MonthMeta>> {
    let mut stmt = conn.prepare(
        "SELECT month, table_name, stats FROM total_stats ORDER BY month",
    )?;
    let rows = stmt.query_map([], |r| {
        let month: String = r.get(0)?;
        let table_name: String = r.get(1)?;
        let stats_json: String = r.get(2)?;
        let stats: MonthStats = serde_json::from_str(&stats_json)
            .map_err(|e| rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e)))?;
        Ok(MonthMeta {
            month,
            table_name,
            stats,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// 查询单个月份统计
pub fn get_month_stats(conn: &Connection, month: &str) -> Result<Option<MonthMeta>> {
    let mut stmt = conn.prepare(
        "SELECT month, table_name, stats FROM total_stats WHERE month = ?1",
    )?;
    let mut rows = stmt.query_map([month], |r| {
        let month: String = r.get(0)?;
        let table_name: String = r.get(1)?;
        let stats_json: String = r.get(2)?;
        let stats: MonthStats = serde_json::from_str(&stats_json)
            .map_err(|e| rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e)))?;
        Ok(MonthMeta {
            month,
            table_name,
            stats,
        })
    })?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Money;

    fn test_conn() -> Connection {
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
        conn
    }

    #[test]
    fn add_and_list_currency() {
        let conn = test_conn();
        let id = add_currency(&conn, "RMB").unwrap();
        let list = list_currencies(&conn).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
        assert_eq!(list[0].name, "RMB");
    }

    #[test]
    fn delete_currency_used_by_counterparty_rejected() {
        let conn = test_conn();
        let cid = add_currency(&conn, "RMB").unwrap();
        add_counterparty(&conn, "微信钱包", cid, Money::from_int(0)).unwrap();
        // 被交易对象引用，不允许删除
        assert!(!delete_currency(&conn, cid).unwrap());
        // 无引用的货币可以删除
        let cid2 = add_currency(&conn, "HK").unwrap();
        assert!(delete_currency(&conn, cid2).unwrap());
    }

    #[test]
    fn month_table_name_format() {
        assert_eq!(month_table_name("2026-08"), "transactions_2026_08");
    }

    #[test]
    fn upsert_and_get_stats() {
        let conn = test_conn();
        let stats = MonthStats {
            month: "2026-08".into(),
            income: vec![("RMB".into(), Money::from_int(100))],
            expense: vec![("RMB".into(), Money::from_int(30))],
        };
        upsert_month_stats(
            &conn,
            &MonthMeta {
                month: "2026-08".into(),
                table_name: "transactions_2026_08".into(),
                stats: stats.clone(),
            },
        )
        .unwrap();
        let got = get_month_stats(&conn, "2026-08").unwrap().unwrap();
        assert_eq!(got.stats, stats);
    }

    #[test]
    fn migrate_from_version_zero_sets_current() {
        let conn = test_conn();
        // 模拟"最开始忘了版本控制的版本"：user_version 为 0
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 0);
        migrate(&conn).unwrap();
        assert_eq!(db_version(&conn).unwrap(), DB_VERSION);
    }
}
