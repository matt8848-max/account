//! SQLite 数据访问层
//!
//! 数据库按“月表 + 对象月度明细”组织：
//! - 月表 `transactions_YYYY_MM` — 该月交易记录（组与记录同表）
//! - `counterparties` — 交易对象（余额、起始月份）
//! - `currencies` — 货币
//! - `counterparty_monthly_YYYY` — 每个交易对象 × 货币 × **有交易的月份** 的转出/转入明细
//!
//! 版本 3 的关键变化（详见 [`migrate`]）：
//! - 删除旧的 `total_stats` 汇总表（每月按货币汇总交易量，无业务意义）；
//! - 金额列从 INTEGER（i64/u32）升级为 **TEXT（i128/i128）**；
//! - 新增按年分表的 `counterparty_monthly_YYYY`，**没有交易的月份不建行**；
//! - 所有账务修改函数都要求 `&mut Connection` 并在事务内执行。

use super::models::{Counterparty, Currency, Money};
use anyhow::{Context, Result};
use rusqlite::{Connection, Transaction};
use std::path::Path;

pub mod counterparty_monthly;
pub mod month;

/// 数据库 Schema 当前版本
///
/// 版本历史：
/// - 1：初始版本（早期库视为版本 0 与之同构）
/// - 2：月表新增 `group_months` 列
/// - 3：金额列升级为 TEXT(i128)、删除 `total_stats`、新增按年分表的
///   `counterparty_monthly_YYYY`（无交易不建行）
pub const DB_VERSION: i64 = 3;

/// 当前本地月份（`YYYY-MM`），用于新建交易对象的 `created_month`。
pub fn current_month() -> String {
    chrono::Local::now().format("%Y-%m").to_string()
}

/// 把金额写库：分子/分母分别转成 TEXT。
pub(crate) fn money_to_text(money: Money) -> (String, String) {
    (money.value.to_string(), money.divisor.to_string())
}

/// 从 TEXT 列还原金额；解析失败返回带上下文的错误。
pub(crate) fn money_from_text(value: &str, divisor: &str) -> Result<Money> {
    let value: i128 = value
        .trim()
        .parse()
        .with_context(|| format!("金额分子不是合法整数: {value}"))?;
    let divisor: i128 = divisor
        .trim()
        .parse()
        .with_context(|| format!("金额分母不是合法整数: {divisor}"))?;
    Money::new(value, divisor).map_err(|e| anyhow::anyhow!(e))
}

/// 从查询行读取金额（连续两列：分子、分母，均为 TEXT）。
pub(crate) fn read_money(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<Money> {
    let value: String = row.get(idx)?;
    let divisor: String = row.get(idx + 1)?;
    let value: i128 = value.parse().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            idx,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        )
    })?;
    let divisor: i128 = divisor.parse().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            idx + 1,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        )
    })?;
    Money::new(value, divisor).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            idx,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        )
    })
}

/// 打开（或创建）数据库并初始化表结构。
///
/// - **全新数据库**（无任何业务表）：直接创建当前版本 schema 并写入 `DB_VERSION`。
/// - **已有数据库**：按版本迁移规则升级（见 [`migrate`]）。
pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path).context("打开数据库失败")?;
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;",
    )
    .context("设置数据库参数失败")?;

    let has_tables: bool = conn
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_master
                WHERE type='table' AND name IN ('currencies','counterparties'))",
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
/// 升级规则：**只向下兼容一个版本的升级**——当前版本为 `N` 时，仅允许从
/// `N-1` 升级到 `N`；更旧版本拒绝升级，避免跳过中间迁移。
///
/// 特殊豁免：版本 0 与 1 结构相同（0 只是“忘了写版本号”），因此
/// 0/1 先执行 1→2，再执行 2→3；版本 2 直接执行 2→3。
pub fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = db_version(conn)?;
    if version == DB_VERSION {
        return Ok(());
    }
    if !(0..=2).contains(&version) {
        return Err(anyhow::anyhow!(
            "数据库版本 {version} 过旧：仅支持从版本 0/1/2 升级到当前版本 {DB_VERSION}，请使用与新版本匹配的程序"
        ));
    }

    if version <= 1 {
        migrate_v1_to_v2(conn)?;
    }
    migrate_v2_to_v3(conn)?;

    conn.execute_batch(&format!("PRAGMA user_version = {DB_VERSION};"))
        .context("写入数据库版本号失败")?;
    log::info!("数据库版本迁移: {version} → {DB_VERSION}");
    Ok(())
}

/// 数据库版本 1/0 → 2：为月表补 `group_name` / `group_months` 列并回填。
///
/// `group_name` 只有极早期库可能缺失；`group_months` 记录组包含的月份，
/// 供跨月组按“组实例”定位删除/追加记录。
fn migrate_v1_to_v2(conn: &Connection) -> Result<()> {
    let tables = existing_transaction_tables(conn)?;

    for table in &tables {
        ensure_column(conn, table, "group_name", "TEXT")?;
        ensure_column(conn, table, "group_months", "TEXT")?;
    }

    // 回填：group_id → 涉及月份集合（跨所有月表聚合）
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
        "数据库迁移 1→2 完成：为 {} 个月表补充 group_name/group_months 并回填",
        tables.len()
    );
    Ok(())
}

/// 确保表上存在某列；旧库补列时使用。
fn ensure_column(conn: &Connection, table: &str, column: &str, ty: &str) -> Result<()> {
    let has: bool = conn
        .query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('{table}') WHERE name='{column}')"
            ),
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if !has {
        conn.execute_batch(&format!(
            "ALTER TABLE `{table}` ADD COLUMN `{column}` {ty};"
        ))
        .with_context(|| format!("表 `{table}` 补加 {column} 列失败"))?;
        log::info!("表 `{table}` 已补加 {column} 列");
    }
    Ok(())
}

/// 数据库版本 2 → 3 的迁移。
///
/// 步骤：
/// 1. 关闭外键（必须在事务外切换）；
/// 2. 重建所有月表，把金额列 `CAST` 为 TEXT；
/// 3. 重建 `counterparties`，金额列改 TEXT 并补 `created_month`；
/// 4. 删除 `total_stats`；
/// 5. 从月表回填对象月度明细到 `counterparty_monthly_YYYY` 年表（只写真实交易月份）。
fn migrate_v2_to_v3(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA foreign_keys = OFF;")
        .context("关闭外键失败")?;

    let result = (|| -> Result<()> {
        let tx = conn.unchecked_transaction().context("开启迁移事务失败")?;
        let tables = existing_transaction_tables(&tx)?;
        // 向下兼容：旧库没有 created_month，取月份最靠前的月表作为所有对象的起始月；
        // 完全没有月表时用当前月。
        let start_month = tables
            .first()
            .map(|t| month_from_table_name(t))
            .unwrap_or_else(current_month);

        for table in &tables {
            rebuild_month_table(&tx, table)?;
        }
        rebuild_counterparties(&tx, &start_month)?;
        tx.execute_batch("DROP TABLE IF EXISTS total_stats;")
            .context("删除 total_stats 失败")?;

        backfill_counterparty_monthly(&tx, &tables)?;
        tx.commit().context("提交迁移事务失败")?;
        Ok(())
    })();

    // 无论成功失败都恢复外键（失败时向上返回原始错误）。
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .context("恢复外键失败")?;
    result?;
    log::info!("数据库迁移 2→3 完成");
    Ok(())
}

/// 月表 DDL（金额为 TEXT，保存 i128 的十进制字符串）。
fn month_table_ddl(table: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS `{table}` (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            group_id       INTEGER NOT NULL,
            group_name     TEXT,
            group_months   TEXT,
            from_id        INTEGER NOT NULL REFERENCES counterparties(id),
            to_id          INTEGER NOT NULL REFERENCES counterparties(id),
            amount_value   TEXT NOT NULL,
            amount_divisor TEXT NOT NULL,
            currency_id    INTEGER NOT NULL REFERENCES currencies(id),
            remark         TEXT,
            traded_at      TEXT NOT NULL
        );"
    )
}

/// 交易对象表 DDL（金额 TEXT + 起始月份）。
fn counterparties_ddl(table: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS `{table}` (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            name            TEXT NOT NULL,
            currency_id     INTEGER NOT NULL REFERENCES currencies(id),
            balance_value   TEXT NOT NULL DEFAULT '0',
            balance_divisor TEXT NOT NULL DEFAULT '1',
            created_month   TEXT NOT NULL,
            UNIQUE(name, currency_id)
        );"
    )
}

/// 重建单个月表：旧金额列 INTEGER → 新 TEXT。
fn rebuild_month_table(tx: &Transaction<'_>, table: &str) -> Result<()> {
    let new_table = format!("{table}__v3");
    tx.execute_batch(&month_table_ddl(&new_table))
        .with_context(|| format!("创建新表 `{new_table}` 失败"))?;
    tx.execute_batch(&format!(
        "INSERT INTO `{new_table}`
            (id, group_id, group_name, group_months, from_id, to_id,
             amount_value, amount_divisor, currency_id, remark, traded_at)
         SELECT id, group_id, group_name, group_months, from_id, to_id,
                CAST(amount_value AS TEXT), CAST(amount_divisor AS TEXT),
                currency_id, remark, traded_at
         FROM `{table}`;
         DROP TABLE `{table}`;
         ALTER TABLE `{new_table}` RENAME TO `{table}`;"
    ))
    .with_context(|| format!("重建月表 `{table}` 失败"))?;
    Ok(())
}

/// 重建交易对象表：金额列改 TEXT，并写入 `created_month`。
fn rebuild_counterparties(tx: &Transaction<'_>, start_month: &str) -> Result<()> {
    tx.execute_batch(&counterparties_ddl("counterparties__v3"))
        .context("创建新 counterparties 表失败")?;
    tx.execute(
        "INSERT INTO counterparties__v3
            (id, name, currency_id, balance_value, balance_divisor, created_month)
         SELECT id, name, currency_id, CAST(balance_value AS TEXT),
                CAST(balance_divisor AS TEXT), ?1
         FROM counterparties",
        [start_month],
    )
    .context("复制交易对象失败")?;
    tx.execute_batch(
        "DROP TABLE counterparties;
         ALTER TABLE counterparties__v3 RENAME TO counterparties;",
    )
    .context("替换 counterparties 表失败")?;
    Ok(())
}

/// 从月表回填 `counterparty_monthly_YYYY` 年表。
fn backfill_counterparty_monthly(tx: &Transaction<'_>, tables: &[String]) -> Result<()> {
    // 对象 → 货币
    let mut currencies: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    {
        let mut stmt = tx.prepare("SELECT id, currency_id FROM counterparties")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (id, currency_id) = row?;
            currencies.insert(id, currency_id);
        }
    }

    for table in tables {
        let month = month_from_table_name(table);
        let mut stmt = tx.prepare(&format!(
            "SELECT from_id, to_id, amount_value, amount_divisor FROM `{table}`"
        ))?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        for row in rows {
            let (from_id, to_id, value, divisor) = row?;
            let amount = money_from_text(&value, &divisor)?;
            if let Some(currency_id) = currencies.get(&from_id) {
                counterparty_monthly::add_amounts(
                    tx,
                    from_id,
                    *currency_id,
                    &month,
                    amount,
                    Money::ZERO,
                )?;
            }
            if let Some(currency_id) = currencies.get(&to_id) {
                counterparty_monthly::add_amounts(
                    tx,
                    to_id,
                    *currency_id,
                    &month,
                    Money::ZERO,
                    amount,
                )?;
            }
        }
    }
    Ok(())
}

/// 从月表名还原月份：`transactions_2026_08` → `2026-08`。
pub(crate) fn month_from_table_name(table: &str) -> String {
    table
        .strip_prefix("transactions_")
        .map(|s| s.replace('_', "-"))
        .unwrap_or_default()
}

/// 列出所有已存在的月表名（`transactions_` 前缀，按名称升序即按月份升序）。
pub(crate) fn existing_transaction_tables(conn: &Connection) -> Result<Vec<String>> {
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

/// 初始化全新数据库的表结构。
fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS currencies (
            id   INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE
        );",
    )
    .context("创建 currencies 表失败")?;
    conn.execute_batch(&counterparties_ddl("counterparties"))
        .context("创建 counterparties 表失败")?;
    Ok(())
}

/// 返回某月份的月表名
pub fn month_table_name(month: &str) -> String {
    format!("transactions_{}", month.replace('-', "_"))
}

/// 确保某月份的月表存在（金额列 TEXT）。
pub fn ensure_month_table(conn: &Connection, month: &str) -> Result<String> {
    let table = month_table_name(month);
    conn.execute_batch(&month_table_ddl(&table))
        .with_context(|| format!("创建月表 `{table}` 失败"))?;
    // 旧月表可能缺列，补上（新表已包含，不会重复添加）。
    ensure_column(conn, &table, "group_name", "TEXT")?;
    ensure_column(conn, &table, "group_months", "TEXT")?;
    Ok(table)
}

// ---------- 货币 ----------

/// 添加货币
pub fn add_currency(conn: &Connection, name: &str) -> Result<i64> {
    conn.execute("INSERT INTO currencies (name) VALUES (?1)", [name])?;
    Ok(conn.last_insert_rowid())
}

/// 重命名货币；空名与重名由调用方/数据库约束拒绝。
pub fn rename_currency(conn: &mut Connection, id: i64, name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(anyhow::anyhow!("货币名称不能为空"));
    }
    let tx = conn.transaction().context("开启事务失败")?;
    match tx.execute(
        "UPDATE currencies SET name = ?2 WHERE id = ?1",
        rusqlite::params![id, name],
    ) {
        Ok(0) => return Err(anyhow::anyhow!("货币不存在")),
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => return Err(anyhow::anyhow!("货币名称已存在")),
        Err(e) => return Err(e.into()),
    }
    tx.commit().context("提交事务失败")?;
    Ok(())
}

/// 列出全部货币
pub fn list_currencies(conn: &Connection) -> Result<Vec<Currency>> {
    let mut stmt = conn.prepare("SELECT id, name FROM currencies ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        Ok(Currency {
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

/// 删除货币（必须未被交易对象、记录或月度明细引用）。
pub fn delete_currency(conn: &mut Connection, id: i64) -> Result<bool> {
    let tx = conn.transaction().context("开启事务失败")?;
    let used_in_counterparty: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM counterparties WHERE currency_id = ?1)",
        [id],
        |r| r.get(0),
    )?;
    let used_in_records: bool = month::currency_used(&tx, id)?;
    let used_in_monthly: bool = counterparty_monthly::currency_used(&tx, id)?;
    if used_in_counterparty || used_in_records || used_in_monthly {
        return Ok(false);
    }
    let n = tx.execute("DELETE FROM currencies WHERE id = ?1", [id])?;
    tx.commit().context("提交事务失败")?;
    Ok(n > 0)
}

// ---------- 交易对象 ----------

/// 添加交易对象（不创建任何月度明细；有交易时才会建行）。
pub fn add_counterparty(
    conn: &mut Connection,
    name: &str,
    currency_id: i64,
    balance: Money,
) -> Result<i64> {
    let created_month = current_month();
    let (value, divisor) = money_to_text(balance);
    let tx = conn.transaction().context("开启事务失败")?;
    tx.execute(
        "INSERT INTO counterparties
            (name, currency_id, balance_value, balance_divisor, created_month)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![name, currency_id, value, divisor, created_month],
    )
    .context("插入交易对象失败")?;
    let id = tx.last_insert_rowid();
    tx.commit().context("提交事务失败")?;
    Ok(id)
}

/// 列出全部交易对象
pub fn list_counterparties(conn: &Connection) -> Result<Vec<Counterparty>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, currency_id, balance_value, balance_divisor, created_month
         FROM counterparties ORDER BY id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Counterparty {
            id: r.get(0)?,
            name: r.get(1)?,
            currency_id: r.get(2)?,
            balance: read_money(r, 3)?,
            created_month: r.get(5)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// 重命名交易对象。
pub fn rename_counterparty(conn: &mut Connection, id: i64, name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(anyhow::anyhow!("交易对象名称不能为空"));
    }
    let tx = conn.transaction().context("开启事务失败")?;
    match tx.execute(
        "UPDATE counterparties SET name = ?2 WHERE id = ?1",
        rusqlite::params![id, name],
    ) {
        Ok(0) => return Err(anyhow::anyhow!("交易对象不存在")),
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => {
            return Err(anyhow::anyhow!("同一货币下已存在同名交易对象"));
        }
        Err(e) => return Err(e.into()),
    }
    tx.commit().context("提交事务失败")?;
    Ok(())
}

/// 调整交易对象余额（`delta`：转出为负、转入为正）。
///
/// **必须在事务内调用**：调用方（月表增删函数）负责开启事务，
/// 保证记录与余额要么一起成功、要么一起回滚。
pub fn adjust_counterparty_balance(conn: &Connection, id: i64, delta: Money) -> Result<()> {
    let current = {
        let mut stmt = conn
            .prepare("SELECT balance_value, balance_divisor FROM counterparties WHERE id = ?1")?;
        let mut rows = stmt.query([id])?;
        let row = rows
            .next()?
            .ok_or_else(|| anyhow::anyhow!("交易对象不存在: id={id}"))?;
        read_money(row, 0)?
    };
    let computed = current.checked_add(delta).map_err(|e| anyhow::anyhow!(e))?;
    let (value, divisor) = money_to_text(computed);
    conn.execute(
        "UPDATE counterparties SET balance_value = ?2, balance_divisor = ?3 WHERE id = ?1",
        rusqlite::params![id, value, divisor],
    )?;
    Ok(())
}

/// 删除交易对象（必须未被任何记录使用；月度明细随外键级联删除）。
pub fn delete_counterparty(conn: &mut Connection, id: i64) -> Result<bool> {
    let tx = conn.transaction().context("开启事务失败")?;
    if month::counterparty_used(&tx, id)? {
        return Ok(false);
    }
    let n = tx.execute("DELETE FROM counterparties WHERE id = ?1", [id])?;
    tx.commit().context("提交事务失败")?;
    Ok(n > 0)
}

/// 判断 SQLite 错误是否为唯一约束冲突（重名）。
fn is_unique_violation(err: &rusqlite::Error) -> bool {
    matches!(
        err,
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::ConstraintViolation,
                ..
            },
            _
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 内存库 + 当前 schema，避免测试产生临时 db 文件。
    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("打开内存库");
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        init_schema(&conn).unwrap();
        conn
    }

    /// 验证货币增删改查：重命名生效，被引用时不可删除。
    #[test]
    fn currency_crud_and_rename_should_work() {
        let mut conn = test_conn();
        let cid = add_currency(&conn, "RMB").unwrap();
        rename_currency(&mut conn, cid, "人民币").unwrap();
        assert_eq!(list_currencies(&conn).unwrap()[0].name, "人民币");
        assert_ne!(list_currencies(&conn).unwrap()[0].name, "RMB");

        let cp = add_counterparty(&mut conn, "微信", cid, Money::ZERO).unwrap();
        assert!(!delete_currency(&mut conn, cid).unwrap());
        delete_counterparty(&mut conn, cp).unwrap();
        assert!(delete_currency(&mut conn, cid).unwrap());
    }

    /// 验证新增交易对象：余额按 TEXT 保存、created_month 为当前月，
    /// 且不创建任何月度明细行（无交易不建行）。
    #[test]
    fn add_counterparty_should_not_create_monthly_rows() {
        let mut conn = test_conn();
        let cid = add_currency(&conn, "RMB").unwrap();
        let balance = Money::new(123, 100).unwrap();
        let id = add_counterparty(&mut conn, "微信", cid, balance).unwrap();

        let list = list_counterparties(&conn).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
        assert_eq!(list[0].balance, balance);
        assert_eq!(list[0].created_month, current_month());
        assert_ne!(list[0].created_month, "1970-01");

        let rows = counterparty_monthly::list(
            &conn,
            &crate::server::models::CounterpartyMonthlyQuery {
                counterparty_ids: vec![id],
                from: None,
                to: None,
            },
        )
        .unwrap();
        assert!(rows.is_empty());
        // 不相等断言：新建对象不能凭空产生月度行
        assert_ne!(rows.len(), 1);
    }

    /// 验证余额调整：正负增量都按数值累加，i128 精度不丢失。
    #[test]
    fn adjust_balance_should_add_signed_money() {
        let mut conn = test_conn();
        let cid = add_currency(&conn, "RMB").unwrap();
        let id = add_counterparty(&mut conn, "现金", cid, Money::from(100)).unwrap();
        adjust_counterparty_balance(&conn, id, Money::new(-1250, 100).unwrap()).unwrap();
        let balance = list_counterparties(&conn).unwrap()[0].balance;
        // 100 - 12.5 = 87.5（数值比较，不依赖内部约分表示）
        assert_eq!(balance, Money::new(875, 10).unwrap());
        // 不相等断言：不能把负数增量当成正数（否则会得到 112.5）
        assert_ne!(balance, Money::new(1125, 10).unwrap());
    }

    /// 验证新建数据库直接是版本 3：无 total_stats、无单表 counterparty_monthly，
    /// 年表按需创建。
    #[test]
    fn open_creates_v3_without_total_stats_or_monthly_tables() {
        let conn = open(Path::new(":memory:")).unwrap();
        assert_eq!(db_version(&conn).unwrap(), DB_VERSION);
        let has_total_stats: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='total_stats')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!has_total_stats);
        let has_legacy_monthly: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='counterparty_monthly')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!has_legacy_monthly);
        assert!(
            counterparty_monthly::existing_years(&conn)
                .unwrap()
                .is_empty()
        );
    }

    /// 验证 v2 → v3 迁移：
    /// - 金额列改为 TEXT 且数值不变；
    /// - total_stats 被删除；
    /// - 月度明细按年表回填，只写真实交易月份（不补空月份）。
    #[test]
    fn migrate_v2_to_v3_should_upgrade_money_and_backfill_monthly() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE total_stats (month TEXT PRIMARY KEY, table_name TEXT NOT NULL, stats TEXT NOT NULL);
             CREATE TABLE currencies (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE);
             CREATE TABLE counterparties (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 name TEXT NOT NULL,
                 currency_id INTEGER NOT NULL REFERENCES currencies(id),
                 balance_value INTEGER NOT NULL DEFAULT 0,
                 balance_divisor INTEGER NOT NULL DEFAULT 1,
                 UNIQUE(name, currency_id)
             );
             CREATE TABLE transactions_2026_08 (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 group_id INTEGER NOT NULL,
                 group_name TEXT,
                 group_months TEXT,
                 from_id INTEGER NOT NULL,
                 to_id INTEGER NOT NULL,
                 amount_value INTEGER NOT NULL,
                 amount_divisor INTEGER NOT NULL,
                 currency_id INTEGER NOT NULL,
                 remark TEXT,
                 traded_at TEXT NOT NULL
             );
             CREATE TABLE transactions_2026_09 (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 group_id INTEGER NOT NULL,
                 group_name TEXT,
                 group_months TEXT,
                 from_id INTEGER NOT NULL,
                 to_id INTEGER NOT NULL,
                 amount_value INTEGER NOT NULL,
                 amount_divisor INTEGER NOT NULL,
                 currency_id INTEGER NOT NULL,
                 remark TEXT,
                 traded_at TEXT NOT NULL
             );
             INSERT INTO currencies (id, name) VALUES (1, 'RMB');
             INSERT INTO counterparties (id, name, currency_id, balance_value, balance_divisor)
                 VALUES (1, '支付宝', 1, -13000, 100), (2, '微信', 1, 13000, 100);
             INSERT INTO transactions_2026_08
                 (group_id, from_id, to_id, amount_value, amount_divisor, currency_id, traded_at)
                 VALUES (1, 1, 2, 10000, 100, 1, '2026-08-14T10:30:00');
             INSERT INTO transactions_2026_09
                 (group_id, from_id, to_id, amount_value, amount_divisor, currency_id, traded_at)
                 VALUES (1, 1, 2, 3000, 100, 1, '2026-09-05T08:00:00');
             PRAGMA user_version = 2;",
        )
        .unwrap();

        migrate(&conn).unwrap();
        assert_eq!(db_version(&conn).unwrap(), DB_VERSION);
        let total_stats: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='total_stats')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!total_stats);

        // 金额列类型为 TEXT，数值保持不变
        let ty: String = conn
            .query_row(
                "SELECT type FROM pragma_table_info('transactions_2026_08') WHERE name='amount_value'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ty, "TEXT");
        let (value, divisor): (String, String) = conn
            .query_row(
                "SELECT amount_value, amount_divisor FROM transactions_2026_09",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(value, "3000");
        assert_eq!(divisor, "100");

        // 余额也迁移为 TEXT
        let balance: (String, String) = conn
            .query_row(
                "SELECT balance_value, balance_divisor FROM counterparties WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(balance, ("-13000".into(), "100".into()));

        // 年表：2026 年表存在，且没有单表 counterparty_monthly
        assert_eq!(
            counterparty_monthly::existing_years(&conn).unwrap(),
            vec![2026]
        );
        let has_legacy: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='counterparty_monthly')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!has_legacy);

        // 月度明细：8 月转出 100、9 月转出 30；转入方对应 100/30
        let aug_out: (String, String) = conn
            .query_row(
                "SELECT out_value, out_divisor FROM counterparty_monthly_2026
                 WHERE counterparty_id = 1 AND month = '2026-08'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            money_from_text(&aug_out.0, &aug_out.1).unwrap(),
            Money::from(100)
        );
        let sep_in: (String, String) = conn
            .query_row(
                "SELECT in_value, in_divisor FROM counterparty_monthly_2026
                 WHERE counterparty_id = 2 AND month = '2026-09'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            money_from_text(&sep_in.0, &sep_in.1).unwrap(),
            Money::from(30)
        );

        // 只有真实交易月份：8 月与 9 月各一行，不补 10 月/11 月等空月份
        let rows = counterparty_monthly::list(
            &conn,
            &crate::server::models::CounterpartyMonthlyQuery {
                counterparty_ids: vec![1],
                from: None,
                to: None,
            },
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].month, "2026-08");
        assert_eq!(rows[1].month, "2026-09");
        // 不相等断言：不能补出空月份
        assert_ne!(rows.len(), 3);
    }
}
