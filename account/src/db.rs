//! SQLite 数据访问层
//!
//! 数据库按月分表：
//! - 总表 `total_stats` — 每月汇总与月表名
//! - 月表 `transactions_YYYY_MM` — 该月交易记录（组与记录同表）
//! - `currencies` — 货币表
//! - `counterparties` — 交易对象表

use crate::models::{Money, MonthMeta, MonthStats};
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;

pub mod month;

/// 数据库 Schema 当前版本
///
/// 版本历史：
/// - 1：初始版本（无结构变更，早期库视为版本 0 与之同构）
/// - 2：月表新增 `group_months` 列，记录每个组包含哪些月份
pub const DB_VERSION: i64 = 2;

/// 打开（或创建）数据库并初始化表结构。
///
/// - **全新数据库**（无任何业务表）：直接创建当前版本 schema 并写入 `DB_VERSION`，
///   避免"新建的库不是最新版本"。
/// - **已有数据库**：按版本迁移规则升级（仅向下兼容一个版本，见 [`migrate`]）。
pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path).context("打开数据库失败")?;
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;",
    )
    .context("设置数据库参数失败")?;

    // 判断是否为全新库：无任何业务表则直接建最新 schema
    let has_tables: bool = conn
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_master
                WHERE type='table' AND name IN ('currencies','counterparties','total_stats'))",
            [],
            |r| r.get(0),
        )
        .context("检查数据库结构失败")?;

    if !has_tables {
        init_schema(&conn)?;
        conn.execute_batch(&format!("PRAGMA user_version = {DB_VERSION};"))
            .context("写入数据库版本号失败")?;
        log::info!("新建数据库，schema 版本 = {DB_VERSION}");
    } else {
        migrate(&conn)?;
    }
    Ok(conn)
}

/// 数据库版本迁移。
///
/// 升级规则：**只向下兼容一个版本的升级**——当前最新版本为 `N` 时，
/// 仅允许从 `N-1` 升级到 `N`；读到更旧的版本（≤ `N-2`）拒绝升级，
/// 防止跳过中间迁移导致数据结构不一致。
///
/// 特殊豁免：版本 0 是"最开始忘了版本控制的版本"，其表结构与版本 1 完全一致
/// （版本 1 升级仅补齐版本号，无结构变更），因此版本 0 与 1 均允许升级到 2。
pub fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = db_version(conn)?;
    if version == DB_VERSION {
        return Ok(());
    }
    // 0 与 1 结构相同，都视为可升级的"上一版本"；其余更旧版本拒绝
    let expected = DB_VERSION - 1;
    if version != 0 && version != expected {
        return Err(anyhow::anyhow!(
            "数据库版本 {version} 过旧：仅支持从版本 0/1（结构同 {expected}）升级到当前版本 {DB_VERSION}，请使用与新版本匹配的程序"
        ));
    }

    // 1/0 → 2：月表新增 group_months 列（记录组包含哪些月份），并回填
    migrate_v1_to_v2(conn)?;

    conn.execute_batch(&format!("PRAGMA user_version = {DB_VERSION};"))
        .context("写入数据库版本号失败")?;
    log::info!("数据库版本迁移: {version} → {DB_VERSION}");
    Ok(())
}

/// 数据库版本 1/0 → 2 的迁移。
///
/// 1. 为所有已存在的月表 `transactions_YYYY_MM` 追加 `group_months TEXT` 列
///    （存储该组包含的所有月份，逗号分隔，如 `2026-08,2026-09`）；
/// 2. 按现有记录回填：对每个组聚合其在所有月表中的交易月份，写入该组各条记录。
fn migrate_v1_to_v2(conn: &Connection) -> Result<()> {
    let tables = existing_transaction_tables(conn)?;

    // 1. 补列
    for table in &tables {
        let has: bool = conn
            .query_row(
                &format!(
                    "SELECT EXISTS(SELECT 1 FROM pragma_table_info('{table}') WHERE name='group_months')"
                ),
                [],
                |r| r.get(0),
            )
            .unwrap_or(false);
        if !has {
            conn.execute_batch(&format!(
                "ALTER TABLE `{table}` ADD COLUMN group_months TEXT;"
            ))
            .with_context(|| format!("旧月表 `{table}` 补加 group_months 列失败"))?;
            log::info!("旧月表 `{table}` 已补加 group_months 列");
        }
    }

    // 2. 回填：group_id → 涉及月份集合（跨所有月表聚合）
    let mut group_months: std::collections::HashMap<i64, std::collections::BTreeSet<String>> =
        std::collections::HashMap::new();
    for table in &tables {
        let mut stmt = conn.prepare(&format!("SELECT group_id, traded_at FROM `{table}`"))?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (gid, traded_at) = row?;
            let month = month::month_of(&traded_at).to_string();
            group_months.entry(gid).or_default().insert(month);
        }
    }

    // 3. 写回
    for (gid, months) in group_months {
        if months.is_empty() {
            continue;
        }
        let joined = months.iter().cloned().collect::<Vec<_>>().join(",");
        for table in &tables {
            conn.execute(
                &format!("UPDATE `{table}` SET group_months = ?1 WHERE group_id = ?2"),
                rusqlite::params![joined, gid],
            )?;
        }
    }
    log::info!(
        "数据库迁移 1→2 完成：为 {} 个月表补充 group_months 并回填",
        tables.len()
    );
    Ok(())
}

/// 列出所有已存在的月表名（`transactions_` 前缀）
fn existing_transaction_tables(conn: &Connection) -> Result<Vec<String>> {
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
            group_months   TEXT,
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
    // 旧月表可能无 group_name / group_months 列，补上
    let has_name: bool = conn
        .query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('{table}') WHERE name='group_name')"
            ),
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if !has_name {
        conn.execute_batch(&format!(
            "ALTER TABLE `{table}` ADD COLUMN group_name TEXT;"
        ))
        .with_context(|| format!("月表 `{table}` 补加 group_name 失败"))?;
    }
    let has_months: bool = conn
        .query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('{table}') WHERE name='group_months')"
            ),
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if !has_months {
        conn.execute_batch(&format!(
            "ALTER TABLE `{table}` ADD COLUMN group_months TEXT;"
        ))
        .with_context(|| format!("月表 `{table}` 补加 group_months 失败"))?;
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
    // 读取当前余额，先按定点数运算规则对齐精度并**去零**（计算后自动去零），
    // 再按原有最大 divisor 写回——存储不强制去零，保持历史数据精度行为。
    let (cur_value, cur_divisor): (i64, u32) = conn.query_row(
        "SELECT balance_value, balance_divisor FROM counterparties WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let current = Money {
        value: cur_value,
        divisor: cur_divisor,
    };
    let delta = Money {
        value: delta_value,
        divisor: delta_divisor,
    };
    let computed = current.checked_add(delta).map_err(|e| anyhow::anyhow!(e))?;
    let divisor = cur_divisor.max(delta_divisor);
    let (new_value, new_divisor) = if divisor % computed.divisor == 0 {
        let scale = (divisor / computed.divisor) as i64;
        match computed.value.checked_mul(scale) {
            Some(v) => (v, divisor),
            None => return Err(anyhow::anyhow!("余额计算溢出")),
        }
    } else {
        (computed.value, computed.divisor)
    };
    conn.execute(
        "UPDATE counterparties SET balance_value = ?2, balance_divisor = ?3 WHERE id = ?1",
        rusqlite::params![id, new_value, new_divisor],
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
    let mut stmt =
        conn.prepare("SELECT month, table_name, stats FROM total_stats ORDER BY month")?;
    let rows = stmt.query_map([], |r| {
        let month: String = r.get(0)?;
        let table_name: String = r.get(1)?;
        let stats_json: String = r.get(2)?;
        let stats: MonthStats = serde_json::from_str(&stats_json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e))
        })?;
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
    let mut stmt =
        conn.prepare("SELECT month, table_name, stats FROM total_stats WHERE month = ?1")?;
    let mut rows = stmt.query_map([month], |r| {
        let month: String = r.get(0)?;
        let table_name: String = r.get(1)?;
        let stats_json: String = r.get(2)?;
        let stats: MonthStats = serde_json::from_str(&stats_json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e))
        })?;
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
        // 模拟"最开始忘了版本控制的版本"：user_version 为 0。
        // 版本 0 与 1 结构相同，0 → 2 是合法升级（补列 group_months 并回填）。
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 0);
        migrate(&conn).unwrap();
        assert_eq!(db_version(&conn).unwrap(), DB_VERSION);
    }

    /// 验证版本 1 → 2 的迁移：旧月表补加 group_months 列，并按已有记录回填
    #[test]
    fn migrate_v1_to_v2_backfills_group_months() {
        let conn = test_conn();
        // 构造版本 1 的旧月表（无 group_months 列）与旧数据
        conn.execute_batch(
            "CREATE TABLE transactions_2026_08 (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id       INTEGER NOT NULL,
                group_name     TEXT,
                from_id        INTEGER NOT NULL,
                to_id          INTEGER NOT NULL,
                amount_value   INTEGER NOT NULL,
                amount_divisor INTEGER NOT NULL,
                currency_id    INTEGER NOT NULL,
                remark         TEXT,
                traded_at      TEXT NOT NULL
            );
            CREATE TABLE transactions_2026_09 (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id       INTEGER NOT NULL,
                group_name     TEXT,
                from_id        INTEGER NOT NULL,
                to_id          INTEGER NOT NULL,
                amount_value   INTEGER NOT NULL,
                amount_divisor INTEGER NOT NULL,
                currency_id    INTEGER NOT NULL,
                remark         TEXT,
                traded_at      TEXT NOT NULL
            );
            INSERT INTO transactions_2026_08
                (group_id, group_name, from_id, to_id, amount_value, amount_divisor, currency_id, remark, traded_at)
                VALUES (1, NULL, 1, 2, 100, 1, 1, NULL, '2026-08-14T10:30:00'),
                       (2, NULL, 1, 2, 50, 1, 1, NULL, '2026-08-02T08:00:00');
            INSERT INTO transactions_2026_09
                (group_id, group_name, from_id, to_id, amount_value, amount_divisor, currency_id, remark, traded_at)
                VALUES (1, NULL, 1, 2, 30, 1, 1, NULL, '2026-09-05T08:00:00');
            PRAGMA user_version = 1;",
        )
        .expect("构造版本 1 旧库");

        migrate(&conn).unwrap();
        assert_eq!(db_version(&conn).unwrap(), DB_VERSION);

        // 组 1 跨 8、9 月 → group_months 应为逗号分隔升序；组 2 只在 8 月
        let g1_aug: String = conn
            .query_row(
                "SELECT group_months FROM transactions_2026_08 WHERE group_id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(g1_aug, "2026-08,2026-09");
        let g1_sep: String = conn
            .query_row(
                "SELECT group_months FROM transactions_2026_09 WHERE group_id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(g1_sep, "2026-08,2026-09");
        let g2: String = conn
            .query_row(
                "SELECT group_months FROM transactions_2026_08 WHERE group_id = 2",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(g2, "2026-08");
    }

    /// 验证升级规则：过旧版本（≤ DB_VERSION-2 且非 0）必须拒绝升级
    #[test]
    fn migrate_rejects_too_old_version() {
        let conn = test_conn();
        // 当前 DB_VERSION=2，允许升级的版本只有 0/1；-1 视为过旧，应报错
        conn.execute_batch("PRAGMA user_version = -1;").unwrap();
        let err = migrate(&conn).unwrap_err();
        assert!(err.to_string().contains("过旧"));
        // 版本未被改动
        assert_eq!(db_version(&conn).unwrap(), -1);
    }

    /// 验证当前版本无需迁移（幂等）
    #[test]
    fn migrate_is_noop_when_current() {
        let conn = test_conn();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        assert_eq!(db_version(&conn).unwrap(), DB_VERSION);
    }

    /// 验证 open 新建的数据库直接是最新版本（而非版本 0）
    #[test]
    fn open_creates_new_db_at_current_version() {
        let conn = open(Path::new(":memory:")).unwrap();
        assert_eq!(db_version(&conn).unwrap(), DB_VERSION);
        // 基本表结构已就绪
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='table' AND name IN ('currencies','counterparties','total_stats')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 3);
    }
}
