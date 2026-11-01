//! 交易记录导出（xlsx）
//!
//! 本模块只负责「数据 → xlsx 字节流」，不触碰文件系统与平台 API：
//! - native（桌面）拿到字节后写入用户选择的路径（见 `ui::export`）
//! - wasm（Web）拿到字节后交给浏览器下载（见 `ui::export`）
//!
//! 每个交易对象一个 sheet：
//! - 第 1 行：A~I 合并为一个单元格，用一行文本显示交易对象、时间区间与流水总额
//!   （转入合计 / 转出合计 / 净额）
//! - 第 2 行：表头，自动筛选默认开启（冻结前两行）
//! - 第 3 行起：流水明细，按交易日期升序；日期写成 Excel 日期时间，可直接排序
//!
//! 全部单元格水平居中；列宽按「表头 + 数据实际显示文本」自动计算（像素），
//! 避免 Excel 因列宽不足把日期/金额显示成 `####`。
//!
//! B 列「金额」由 H/I 两列原始金额（分子 / 分母）自动计算，并写入缓存值，
//! 保证不支持公式重算的阅读器也能看到数值。

use crate::models::Money;
use rust_xlsxwriter::{
    ExcelDateTime, Format, FormatAlign, FormatBorder, Formula, Workbook, XlsxError,
};

/// 表头（顺序即列顺序，属于导出格式的一部分，勿随意调整）
const HEADERS: [&str; 9] = [
    "转出/转入",
    "金额",
    "货币类型",
    "交易对象",
    "交易日期",
    "交易备注",
    "交易分组",
    "原始金额分子",
    "原始金额分母",
];

/// 每个显示字符估算的像素宽度（略大于 Excel 默认字体的 7px，为替代字体留余量）
const PX_PER_COL: u32 = 8;
/// 单元格左右内边距（像素）：保证内容不贴边
const CELL_PADDING_PX: u32 = 16;
/// 列宽下限（像素）：避免表头文字被压扁
const COLUMN_MIN_PX: u32 = 64;
/// 列宽上限（像素）：避免超长备注把表格撑爆
const COLUMN_MAX_PX: u32 = 400;
/// 日期时间格式化后的显示列数（`yyyy-mm-dd hh:mm:ss`，共 19 个半角字符）
const DATETIME_DISPLAY_COLS: usize = 19;

/// 金额列索引（0 起，B 列）：由原始金额分子/分母计算
const COL_AMOUNT: u16 = 1;
/// 原始金额分子列索引（H 列）
const COL_NUMERATOR: u16 = 7;
/// 原始金额分母列索引（I 列）
const COL_DENOMINATOR: u16 = 8;
/// 表头所在行索引（0 起，即 Excel 第 2 行）
const HEADER_ROW: u32 = 1;
/// 数据起始行索引（0 起，即 Excel 第 3 行）
const DATA_START_ROW: u32 = 2;
/// Excel sheet 名长度上限
const SHEET_NAME_MAX: usize = 31;
/// Excel 日期时间单元格的数字格式
const DATE_FORMAT: &str = "yyyy-mm-dd hh:mm:ss";

/// 流水方向（相对该 sheet 的交易对象）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// 转出：本对象是转出方
    Out,
    /// 转入：本对象是转入方
    In,
}

impl Direction {
    /// 依据记录的两个对象 ID 判断方向：本对象是转出方为「转出」，是转入方为「转入」，
    /// 与本记录无关时返回 None（调用方应过滤掉这类记录）。
    pub fn of(from_id: i64, to_id: i64, counterparty_id: i64) -> Option<Self> {
        if from_id == counterparty_id {
            Some(Self::Out)
        } else if to_id == counterparty_id {
            Some(Self::In)
        } else {
            None
        }
    }

    /// 表格与界面中显示的文字
    pub fn label(self) -> &'static str {
        match self {
            Self::Out => "转出",
            Self::In => "转入",
        }
    }
}

/// 单条导出流水（方向、对方名称、货币名均已在构建阶段解析完毕）
#[derive(Debug, Clone)]
pub struct ExportRecord {
    pub direction: Direction,
    pub amount: Money,
    pub currency: String,
    /// 交易对象（非自己）
    pub other_party: String,
    /// 交易时间，形如 `2026-01-31 12:00:00`
    pub traded_at: String,
    pub remark: Option<String>,
    /// 交易分组（无组名时由调用方回退为 `组#ID`）
    pub group: Option<String>,
}

/// 一个交易对象的导出数据（对应一个 sheet）
#[derive(Debug, Clone)]
pub struct ExportSheet {
    /// 交易对象名称
    pub counterparty: String,
    /// 货币类型名称
    pub currency: String,
    /// 起始月份 `YYYY-MM`；None 表示不限
    pub month_from: Option<String>,
    /// 结束月份 `YYYY-MM`；None 表示不限
    pub month_to: Option<String>,
    pub records: Vec<ExportRecord>,
}

/// 汇总结果：转入合计、转出合计、净额（净额 = 转入 − 转出）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportTotals {
    pub income: Money,
    pub expense: Money,
    pub net: Money,
}

/// 汇总流水：按方向分别累加，得到转入合计、转出合计与净额。
///
/// 金额本身恒为正数（建记录时已校验），方向决定加减；
/// 累加溢出（极端数据）时保留上一次结果而非 panic。
pub fn summarize(records: &[ExportRecord]) -> ExportTotals {
    let mut income = Money::from_int(0);
    let mut expense = Money::from_int(0);
    for rec in records {
        match rec.direction {
            Direction::In => income = income.checked_add(rec.amount).unwrap_or(income),
            Direction::Out => expense = expense.checked_add(rec.amount).unwrap_or(expense),
        }
    }
    let net = income
        .checked_sub(expense)
        .unwrap_or_else(|_| Money::from_int(0));
    ExportTotals {
        income: income.normalized(),
        expense: expense.normalized(),
        net: net.normalized(),
    }
}

/// 把 `YYYY-MM` 解析为 `(年, 月)`；格式非法或月份越界返回 None。
fn parse_month(month: &str) -> Option<(i32, u32)> {
    let (year, month) = month.trim().split_once('-')?;
    let year = year.parse::<i32>().ok()?;
    let month = month.parse::<u32>().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    Some((year, month))
}

/// 该月天数（含闰年 2 月 29 天）
fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 30,
    }
}

/// 由月份区间计算接口查询用的时间边界：
/// - 起始月 → `YYYY-MM-01T00:00:00`
/// - 结束月 → `YYYY-MM-<最后一天>T23:59:59`（含整月，避免漏掉月末当天的记录）
///
/// 月份非法时对应边界为 None（不限）。
pub fn month_bounds(from: Option<&str>, to: Option<&str>) -> (Option<String>, Option<String>) {
    let from_bound = from
        .and_then(parse_month)
        .map(|(year, month)| format!("{year:04}-{month:02}-01T00:00:00"));
    let to_bound = to.and_then(parse_month).map(|(year, month)| {
        format!(
            "{year:04}-{month:02}-{:02}T23:59:59",
            days_in_month(year, month)
        )
    });
    (from_bound, to_bound)
}

/// 月份区间的展示文本：`2026-01 ~ 2026-03` / `不限 ~ 2026-03` / `不限`
pub fn range_label(from: Option<&str>, to: Option<&str>) -> String {
    match (from, to) {
        (Some(from), Some(to)) => format!("{from} ~ {to}"),
        (Some(from), None) => format!("{from} ~ 不限"),
        (None, Some(to)) => format!("不限 ~ {to}"),
        (None, None) => "不限".to_string(),
    }
}

/// 默认导出文件名：`account_export_2026-01_2026-03.xlsx`（不限时用 `all` 占位）
pub fn default_file_name(from: Option<&str>, to: Option<&str>) -> String {
    format!(
        "account_export_{}_{}.xlsx",
        from.unwrap_or("all"),
        to.unwrap_or("all")
    )
}

/// 金额在表格中的显示文本：与数字格式 `#,##0.######` 一致（整数部分加千位分隔符）。
///
/// 用于计算列宽：`1234567.89` 实际显示为 `1,234,567.89`，按后者算宽度才不会偏窄。
pub fn amount_display(amount: &Money) -> String {
    let text = amount.to_decimal_string();
    let (sign, rest) = match text.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", text.as_str()),
    };
    let (int_part, frac_part) = match rest.split_once('.') {
        Some((int_part, frac)) => (int_part, Some(frac)),
        None => (rest, None),
    };
    let digits: Vec<char> = int_part.chars().collect();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (idx, ch) in digits.iter().enumerate() {
        // 从右往左每 3 位插一个逗号（最高位前不加）
        if idx > 0 && (digits.len() - idx).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(*ch);
    }
    match frac_part {
        Some(frac) => format!("{sign}{grouped}.{frac}"),
        None => format!("{sign}{grouped}"),
    }
}

/// 金额 + 货币的显示文本（货币为空时只显示金额，避免多余空格）。
fn amount_with_currency(amount: &Money, currency: &str) -> String {
    let currency = currency.trim();
    if currency.is_empty() {
        amount_display(amount)
    } else {
        format!("{} {}", amount_display(amount), currency)
    }
}

/// 第 1 行合并单元格的标题文本：所有信息用一行文本展示（不含换行）。
///
/// 流水总额的三项金额后面都带上货币类型（如 `1,234.56 RMB`），
/// 避免只看首行时无法判断金额币种。
pub fn title_line(sheet: &ExportSheet, totals: &ExportTotals) -> String {
    format!(
        "交易对象: {}（{}） | 时间区间: {} | 流水总额-转入: {} | 流水总额-转出: {} | 流水总额-净额: {}",
        sheet.counterparty,
        sheet.currency,
        range_label(sheet.month_from.as_deref(), sheet.month_to.as_deref()),
        amount_with_currency(&totals.income, &sheet.currency),
        amount_with_currency(&totals.expense, &sheet.currency),
        amount_with_currency(&totals.net, &sheet.currency),
    )
}

/// 规范化用户填写的文件名：去掉首尾空白，为空时回退默认名，缺 `.xlsx` 后缀时补齐。
///
/// 只补后缀不做路径处理：路径分隔符由各自的保存流程（桌面选路径 / 浏览器下载）负责。
pub fn ensure_xlsx_suffix(name: &str, fallback: &str) -> String {
    let name = name.trim();
    let name = if name.is_empty() { fallback } else { name };
    if name.to_lowercase().ends_with(".xlsx") {
        name.to_string()
    } else {
        format!("{name}.xlsx")
    }
}

/// 生成 Excel sheet 名。
///
/// 规则：
/// - 拼接 `名称(货币)`，避免不同货币下的同名对象互相覆盖；
/// - 替换 Excel 禁止的字符 `: \ / ? * [ ]`，并去掉首尾的 `'`；
/// - 截断到 31 字符（Excel 上限），空名回退 `Sheet`；
/// - 与 `used` 中已有名字重复（忽略大小写）时追加 `_2`、`_3`… 并同样截断。
pub fn sheet_name(name: &str, currency: &str, used: &mut Vec<String>) -> String {
    // 名称与货币各自去掉首尾空白与单引号（Excel 不允许 sheet 名首尾为单引号）
    let name = name.trim().trim_matches('\'');
    let currency = currency.trim().trim_matches('\'');
    let raw = if currency.is_empty() {
        name.to_string()
    } else {
        format!("{name}({currency})")
    };
    let cleaned: String = raw
        .chars()
        .map(|c| match c {
            ':' | '\\' | '/' | '?' | '*' | '[' | ']' => '_',
            other => other,
        })
        .collect();

    let mut base: String = cleaned.trim().chars().take(SHEET_NAME_MAX).collect();
    if base.is_empty() {
        base = "Sheet".to_string();
    }

    let mut candidate = base.clone();
    let mut seq = 2;
    while used.iter().any(|u| u.eq_ignore_ascii_case(&candidate)) {
        let suffix = format!("_{seq}");
        let keep = SHEET_NAME_MAX.saturating_sub(suffix.len());
        candidate = format!("{}{suffix}", base.chars().take(keep).collect::<String>());
        seq += 1;
    }
    used.push(candidate.clone());
    candidate
}

/// 把 `YYYY-MM-DD HH:MM:SS`（也接受 `T` 分隔、缺省时分秒）转成 Excel 日期时间。
///
/// 解析失败返回 None，调用方回退为写字符串，避免导出中断。
fn excel_datetime(traded_at: &str) -> Option<ExcelDateTime> {
    let (date, time) = traded_at
        .split_once(['T', ' '])
        .unwrap_or((traded_at, "00:00:00"));
    let mut date_parts = date.split('-');
    let year = date_parts.next()?.parse::<u16>().ok()?;
    let month = date_parts.next()?.parse::<u8>().ok()?;
    let day = date_parts.next()?.parse::<u8>().ok()?;
    let mut time_parts = time.split(':');
    let hour = time_parts.next()?.parse::<u16>().ok()?;
    let minute = time_parts.next().unwrap_or("0").parse::<u8>().ok()?;
    let second = time_parts.next().unwrap_or("0").parse::<f64>().ok()?;
    ExcelDateTime::from_ymd(year, month, day)
        .ok()?
        .and_hms(hour, minute, second)
        .ok()
}

/// 0 起列索引 → Excel 列字母（0→A、1→B … 7→H、8→I）。
fn column_letter(col: u16) -> char {
    // 导出固定 9 列（A~I），无需处理 AA 以上
    (b'A' + (col.min(25) as u8)) as char
}

/// 是否为全角/中日韩字符（在 Excel 中约占 2 个半角字符的宽度）。
fn is_wide_char(c: char) -> bool {
    matches!(
        c as u32,
        0x1100..=0x115F     // 谚文字母
        | 0x2E80..=0xA4CF   // 康熙部首、CJK、假名、注音、谚文兼容等
        | 0xAC00..=0xD7A3   // 谚文音节
        | 0xF900..=0xFAFF   // CJK 兼容汉字
        | 0xFE30..=0xFE6F   // CJK 兼容形式
        | 0xFF00..=0xFF60   // 全角 ASCII
        | 0xFFE0..=0xFFE6   // 全角符号
        | 0x20000..=0x3FFFD // CJK 扩展 B 及以后
    )
}

/// 单行文本的显示宽度：半角字符计 1，全角/中日韩字符计 2。
pub fn display_cols(text: &str) -> usize {
    text.chars()
        .map(|c| if is_wide_char(c) { 2 } else { 1 })
        .sum()
}

/// 显示列数 → 列宽（像素）：按每列 [`PX_PER_COL`] 估算并加内边距，钳制在上下限之间。
fn column_width_px(cols: usize) -> u32 {
    let px = (cols as u32).saturating_mul(PX_PER_COL) + CELL_PADDING_PX;
    px.clamp(COLUMN_MIN_PX, COLUMN_MAX_PX)
}

/// 计算 9 列的建议列宽（像素）：取「表头 + 所有数据行实际显示文本」的最大显示宽度。
///
/// - 日期列按格式化后的 `yyyy-mm-dd hh:mm:ss` 计（解析失败时按写入的原始文本），
///   避免出现「列宽按原始字符串算够、实际显示却不够」的 `####` 问题；
/// - 金额列按带千位分隔符的显示文本计；
/// - 合并的第 1 行标题不参与计算，否则会把 A 列撑到极宽。
pub fn column_widths(rows: &[&ExportRecord], headers: &[&str; 9]) -> [u32; 9] {
    let mut max_cols: [usize; 9] = [0; 9];
    for (idx, header) in headers.iter().enumerate() {
        max_cols[idx] = display_cols(header);
    }
    for rec in rows {
        let amount = amount_display(&rec.amount);
        // 能解析成 Excel 日期时间时，单元格显示的是固定格式文本（19 列）
        let traded_at_cols = match excel_datetime(&rec.traded_at) {
            Some(_) => DATETIME_DISPLAY_COLS,
            None => display_cols(&rec.traded_at),
        };
        let values = [
            display_cols(rec.direction.label()),
            display_cols(&amount),
            display_cols(&rec.currency),
            display_cols(&rec.other_party),
            traded_at_cols,
            display_cols(rec.remark.as_deref().unwrap_or("")),
            display_cols(rec.group.as_deref().unwrap_or("")),
            display_cols(&rec.amount.value.to_string()),
            display_cols(&rec.amount.divisor.to_string()),
        ];
        for (idx, cols) in values.iter().enumerate() {
            if *cols > max_cols[idx] {
                max_cols[idx] = *cols;
            }
        }
    }
    max_cols.map(column_width_px)
}

/// 统一把 `XlsxError` 转成给用户看的错误文本
fn xlsx_err(err: XlsxError) -> String {
    format!("生成 xlsx 失败: {err}")
}

/// 生成 xlsx 字节流：每个 [`ExportSheet`] 一个 sheet，表头固定 9 列并开启自动筛选。
///
/// 无任何数据时返回错误（不生成空文件）。
pub fn build_xlsx(sheets: &[ExportSheet]) -> Result<Vec<u8>, String> {
    if sheets.is_empty() {
        return Err("没有可导出的数据".to_string());
    }

    let title_fmt = Format::new().set_bold().set_align(FormatAlign::Center);
    let header_fmt = Format::new()
        .set_bold()
        .set_align(FormatAlign::Center)
        .set_border(FormatBorder::Thin)
        .set_background_color("#D9E1F2");
    // 正文统一水平居中（用户要求所有列居中）
    let body_fmt = Format::new().set_align(FormatAlign::Center);
    let amount_fmt = Format::new()
        .set_num_format("#,##0.######")
        .set_align(FormatAlign::Center);
    let int_fmt = Format::new()
        .set_num_format("0")
        .set_align(FormatAlign::Center);
    let date_fmt = Format::new()
        .set_num_format(DATE_FORMAT)
        .set_align(FormatAlign::Center);

    let mut workbook = Workbook::new();
    let mut used_names: Vec<String> = Vec::new();

    for sheet in sheets {
        let name = sheet_name(&sheet.counterparty, &sheet.currency, &mut used_names);
        // 先按交易时间升序，保证导出的流水顺序稳定（同一时间保持原有顺序）
        let mut rows: Vec<&ExportRecord> = sheet.records.iter().collect();
        rows.sort_by(|a, b| a.traded_at.cmp(&b.traded_at));
        let totals = summarize(&sheet.records);

        let worksheet = workbook.add_worksheet();
        worksheet.set_name(name).map_err(xlsx_err)?;

        // 第 1 行：A~I 合并为一个单元格，用一行文本展示全部信息
        worksheet
            .merge_range(0, 0, 0, 8, &title_line(sheet, &totals), &title_fmt)
            .map_err(xlsx_err)?;

        // 第 2 行：表头（自动筛选的标题行）
        for (col, title) in HEADERS.iter().enumerate() {
            worksheet
                .write_string_with_format(HEADER_ROW, col as u16, *title, &header_fmt)
                .map_err(xlsx_err)?;
        }

        for (idx, rec) in rows.iter().enumerate() {
            let row = DATA_START_ROW + idx as u32;
            // 公式用 Excel 行号（1 起）
            let excel_row = row + 1;
            worksheet
                .write_string_with_format(row, 0, rec.direction.label(), &body_fmt)
                .map_err(xlsx_err)?;
            // 金额 = 原始金额分子 / 分母；写入缓存值供不重算公式的阅读器显示
            let formula = Formula::new(format!(
                "={}{excel_row}/{}{excel_row}",
                column_letter(COL_NUMERATOR),
                column_letter(COL_DENOMINATOR)
            ))
            .set_result(rec.amount.to_decimal_string());
            worksheet
                .write_formula_with_format(row, COL_AMOUNT, formula, &amount_fmt)
                .map_err(xlsx_err)?;
            worksheet
                .write_string_with_format(row, 2, rec.currency.as_str(), &body_fmt)
                .map_err(xlsx_err)?;
            worksheet
                .write_string_with_format(row, 3, rec.other_party.as_str(), &body_fmt)
                .map_err(xlsx_err)?;
            match excel_datetime(&rec.traded_at) {
                Some(datetime) => {
                    worksheet
                        .write_datetime_with_format(row, 4, &datetime, &date_fmt)
                        .map_err(xlsx_err)?;
                }
                // 时间无法解析时退化为文本，至少不丢数据
                None => {
                    worksheet
                        .write_string_with_format(row, 4, rec.traded_at.as_str(), &body_fmt)
                        .map_err(xlsx_err)?;
                }
            }
            worksheet
                .write_string_with_format(row, 5, rec.remark.as_deref().unwrap_or(""), &body_fmt)
                .map_err(xlsx_err)?;
            worksheet
                .write_string_with_format(row, 6, rec.group.as_deref().unwrap_or(""), &body_fmt)
                .map_err(xlsx_err)?;
            worksheet
                .write_number_with_format(row, COL_NUMERATOR, rec.amount.value as f64, &int_fmt)
                .map_err(xlsx_err)?;
            worksheet
                .write_number_with_format(
                    row,
                    COL_DENOMINATOR,
                    f64::from(rec.amount.divisor),
                    &int_fmt,
                )
                .map_err(xlsx_err)?;
        }

        // 自动筛选覆盖「表头 + 全部数据行」；冻结前两行保证滚动时表头可见
        let last_row = (DATA_START_ROW + rows.len() as u32).saturating_sub(1);
        worksheet
            .autofilter(HEADER_ROW, 0, last_row.max(HEADER_ROW), 8)
            .map_err(xlsx_err)?;
        worksheet
            .set_freeze_panes(DATA_START_ROW, 0)
            .map_err(xlsx_err)?;
        // 列宽按内容自动计算（像素）：日期列按格式化后的文本算，避免显示成 `####`
        for (col, width) in column_widths(&rows, &HEADERS).iter().enumerate() {
            worksheet
                .set_column_width_pixels(col as u16, *width)
                .map_err(xlsx_err)?;
        }
    }

    workbook
        .save_to_buffer()
        .map_err(|e| format!("生成 xlsx 失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一条导出流水（金额与货币固定，方向/时间可按需定制）
    fn rec(direction: Direction, value: i64, divisor: u32, traded_at: &str) -> ExportRecord {
        ExportRecord {
            direction,
            amount: Money { value, divisor },
            currency: "RMB".to_string(),
            other_party: "微信钱包".to_string(),
            traded_at: traded_at.to_string(),
            remark: None,
            group: None,
        }
    }

    /// 构造一个导出表（单对象）
    fn sheet(records: Vec<ExportRecord>) -> ExportSheet {
        ExportSheet {
            counterparty: "支付宝".to_string(),
            currency: "RMB".to_string(),
            month_from: Some("2026-01".to_string()),
            month_to: Some("2026-03".to_string()),
            records,
        }
    }

    /// 验证方向判定：本对象为转出方 → 转出；为转入方 → 转入；与记录无关 → None
    #[test]
    fn direction_should_map_by_counterparty_id() {
        assert_eq!(Direction::of(1, 2, 1), Some(Direction::Out));
        assert_eq!(Direction::of(1, 2, 2), Some(Direction::In));
        assert_eq!(Direction::of(1, 2, 3), None);
        // 自转（同一对象既是转出方也是转入方）按"转出"处理，避免漏记
        assert_eq!(Direction::of(1, 1, 1), Some(Direction::Out));
        // 展示文字即导出枚举列内容
        assert_eq!(Direction::Out.label(), "转出");
        assert_eq!(Direction::In.label(), "转入");
    }

    /// 验证汇总：转入合计、转出合计分别累加（不同精度自动对齐并去零），净额 = 转入 − 转出
    #[test]
    fn summarize_should_total_each_direction_and_net() {
        let records = vec![
            rec(Direction::In, 13, 10, "2026-01-01 00:00:00"),
            rec(Direction::In, 27, 10, "2026-01-02 00:00:00"),
            rec(Direction::Out, 205, 100, "2026-01-03 00:00:00"),
        ];
        let totals = summarize(&records);
        // 1.3 + 2.7 = 4.0 → 去零为 (4,1)；2.05 保持 (205,100)
        assert_eq!(totals.income, Money::from_int(4));
        assert_eq!(
            totals.expense,
            Money {
                value: 205,
                divisor: 100
            }
        );
        // 4 - 2.05 = 1.95
        assert_eq!(
            totals.net,
            Money {
                value: 195,
                divisor: 100
            }
        );
    }

    /// 验证空流水：三项合计均为 0，不 panic
    #[test]
    fn summarize_should_return_zeros_for_empty_records() {
        let totals = summarize(&[]);
        assert_eq!(totals.income, Money::from_int(0));
        assert_eq!(totals.expense, Money::from_int(0));
        assert_eq!(totals.net, Money::from_int(0));
    }

    /// 验证月份边界：起始月取 1 号 0 点，结束月取该月最后一天 23:59:59（含整月）
    #[test]
    fn month_bounds_should_cover_whole_months() {
        let (from, to) = month_bounds(Some("2026-01"), Some("2026-03"));
        assert_eq!(from.as_deref(), Some("2026-01-01T00:00:00"));
        assert_eq!(to.as_deref(), Some("2026-03-31T23:59:59"));
        // 4/6/9/11 月 30 天
        let (_, to) = month_bounds(Some("2026-04"), Some("2026-06"));
        assert_eq!(to.as_deref(), Some("2026-06-30T23:59:59"));
    }

    /// 验证闰年边界：能被 4 整除（2024）、被 400 整除（2000）为 29 天；
    /// 被 100 整除但非 400（2100）与平年（2026）为 28 天
    #[test]
    fn month_bounds_should_handle_february_leap_year_boundary() {
        for (year, expected) in [("2024", 29), ("2000", 29), ("2100", 28), ("2026", 28)] {
            let month = format!("{year}-02");
            let (_, to) = month_bounds(Some(&month), Some(&month));
            assert_eq!(
                to.as_deref(),
                Some(format!("{year}-02-{expected:02}T23:59:59").as_str()),
                "{year} 年 2 月天数应为 {expected}"
            );
        }
    }

    /// 验证非法/缺省月份：None 表示不限，非法文本或越界月份同样视为不限（不产生错误边界）
    #[test]
    fn month_bounds_should_ignore_invalid_month() {
        assert_eq!(month_bounds(None, None), (None, None));
        assert_eq!(month_bounds(Some("2026-13"), Some("abc")), (None, None));
        assert_eq!(month_bounds(Some("2026-00"), None), (None, None));
    }

    /// 验证区间文本与默认文件名：单边/双边/全不限三种组合
    #[test]
    fn range_label_and_file_name_should_cover_all_combinations() {
        assert_eq!(
            range_label(Some("2026-01"), Some("2026-03")),
            "2026-01 ~ 2026-03"
        );
        assert_eq!(range_label(Some("2026-01"), None), "2026-01 ~ 不限");
        assert_eq!(range_label(None, Some("2026-03")), "不限 ~ 2026-03");
        assert_eq!(range_label(None, None), "不限");

        assert_eq!(
            default_file_name(Some("2026-01"), Some("2026-03")),
            "account_export_2026-01_2026-03.xlsx"
        );
        assert_eq!(default_file_name(None, None), "account_export_all_all.xlsx");
    }

    /// 验证 sheet 名清洗：替换 Excel 非法字符、首尾单引号去掉、空名回退 Sheet
    #[test]
    fn sheet_name_should_replace_forbidden_chars_and_keep_valid_name() {
        let mut used = Vec::new();
        assert_eq!(sheet_name("支付宝", "RMB", &mut used), "支付宝(RMB)");
        // 非法字符全部替换为下划线
        let mut used = Vec::new();
        assert_eq!(
            sheet_name("a:b\\c/d?e*f[g]h", "RMB", &mut used),
            "a_b_c_d_e_f_g_h(RMB)"
        );
        // 首尾单引号被去掉
        let mut used = Vec::new();
        assert_eq!(sheet_name("'钱包'", "USD", &mut used), "钱包(USD)");
        // 空名回退
        let mut used = Vec::new();
        assert_eq!(sheet_name("", "", &mut used), "Sheet");
    }

    /// 验证 sheet 名长度边界：恰好 31 字符不截断，第 32 个字符被截掉（Excel 上限 31）
    #[test]
    fn sheet_name_should_truncate_at_31_chars() {
        // 名称 27 个 ASCII 字符 + "(RMB)" 5 字符 = 32，应截断到 31
        let mut used = Vec::new();
        let name = format!("{}(RMB)", "a".repeat(27));
        let got = sheet_name(&"a".repeat(27), "RMB", &mut used);
        assert_eq!(
            got.chars().count(),
            SHEET_NAME_MAX,
            "超长名应截断到 31 字符"
        );
        assert_eq!(got, name[..31]);

        // 恰好 31 字符不截断
        let mut used = Vec::new();
        let got = sheet_name(&"b".repeat(26), "RMB", &mut used);
        assert_eq!(got.chars().count(), SHEET_NAME_MAX);
        assert_eq!(got, format!("{}(RMB)", "b".repeat(26)));
    }

    /// 验证 sheet 名去重：重名（忽略大小写）追加 _2/_3，且追加后仍不超过 31 字符
    #[test]
    fn sheet_name_should_dedupe_case_insensitively() {
        let mut used = Vec::new();
        assert_eq!(sheet_name("支付宝", "RMB", &mut used), "支付宝(RMB)");
        assert_eq!(sheet_name("支付宝", "RMB", &mut used), "支付宝(RMB)_2");
        assert_eq!(sheet_name("支付宝", "RMB", &mut used), "支付宝(RMB)_3");
        // 大小写不同视为同名（Excel sheet 名大小写不敏感）
        assert_eq!(sheet_name("BANK", "USD", &mut used), "BANK(USD)");
        assert_eq!(sheet_name("bank", "usd", &mut used), "bank(usd)_2");

        // 超长重名：追加后缀后仍需 ≤ 31 字符
        let mut used = Vec::new();
        let long = "c".repeat(31);
        let first = sheet_name(&long, "", &mut used);
        let second = sheet_name(&long, "", &mut used);
        assert_eq!(first.chars().count(), SHEET_NAME_MAX);
        assert!(
            second.chars().count() <= SHEET_NAME_MAX,
            "去重后仍不得超过 31 字符"
        );
        assert!(second.ends_with("_2"));
    }

    /// 验证 xlsx 生成：单 sheet 双记录产出合法 zip 容器（PK 开头），且非空
    #[test]
    fn build_xlsx_should_produce_non_empty_archive() {
        let data = vec![sheet(vec![
            // 故意乱序：导出时应按时间升序排列（此处仅验证能生成）
            rec(Direction::In, 1000, 100, "2026-02-01 08:30:00"),
            rec(Direction::Out, 25, 1, "2026-01-15 20:00:00"),
        ])];
        let bytes = build_xlsx(&data).expect("应生成 xlsx");
        assert!(
            bytes.len() > 1000,
            "xlsx 内容不应为空，实际 {} 字节",
            bytes.len()
        );
        assert_eq!(&bytes[..2], b"PK", "xlsx 应为 zip 容器（PK 开头）");
    }

    /// 验证 xlsx 生成：多对象 → 多个 sheet（同一 workbook 内 sheet 名互不相同）
    #[test]
    fn build_xlsx_should_create_one_sheet_per_counterparty() {
        let mut second = sheet(vec![rec(Direction::Out, 500, 100, "2026-03-02 10:00:00")]);
        second.counterparty = "微信钱包".to_string();
        let bytes = build_xlsx(&[sheet(vec![]), second]).expect("应生成 xlsx");
        assert_eq!(&bytes[..2], b"PK");
    }

    /// 验证导出时间无法解析时退化为文本：不报错、仍能生成文件
    #[test]
    fn build_xlsx_should_tolerate_unparsable_datetime() {
        let bytes = build_xlsx(&[sheet(vec![rec(Direction::In, 1, 1, "not-a-time")])])
            .expect("时间解析失败不应导致导出失败");
        assert_eq!(&bytes[..2], b"PK");
    }

    /// 验证空导出（无对象）返回错误，避免生成空文件
    #[test]
    fn build_xlsx_should_reject_empty_sheets() {
        assert!(build_xlsx(&[]).is_err());
    }

    /// 验证文件名规范化：空名回退默认名，缺/有 `.xlsx` 后缀两种情况、大小写后缀均正确
    #[test]
    fn ensure_xlsx_suffix_should_normalize_file_name() {
        assert_eq!(ensure_xlsx_suffix("", "default.xlsx"), "default.xlsx");
        assert_eq!(ensure_xlsx_suffix("   ", "default.xlsx"), "default.xlsx");
        assert_eq!(ensure_xlsx_suffix("导出", "default.xlsx"), "导出.xlsx");
        assert_eq!(ensure_xlsx_suffix("导出.XLSX", "default.xlsx"), "导出.XLSX");
        assert_eq!(
            ensure_xlsx_suffix("  导出.xlsx  ", "default.xlsx"),
            "导出.xlsx"
        );
    }

    /// 验证金额显示文本：整数部分按千位加逗号，小数部分原样保留（与 `#,##0.######` 一致）
    #[test]
    fn amount_display_should_insert_thousand_separators() {
        // 1234567.89 → 1,234,567.89
        assert_eq!(
            amount_display(&Money {
                value: 123456789,
                divisor: 100
            }),
            "1,234,567.89"
        );
        // 三位以内不加逗号；整数无需补小数点
        assert_eq!(amount_display(&Money::from_int(999)), "999");
        assert_eq!(amount_display(&Money::from_int(1000)), "1,000");
        assert_eq!(amount_display(&Money::from_int(0)), "0");
        // 负数：负号保留在最前，逗号只加在整数部分
        assert_eq!(
            amount_display(&Money {
                value: -1234567,
                divisor: 1
            }),
            "-1,234,567"
        );
    }

    /// 验证显示宽度：中文/全角按 2 列计，ASCII 按 1 列计（空串为 0）
    #[test]
    fn display_cols_should_count_wide_chars_as_two() {
        assert_eq!(display_cols(""), 0);
        assert_eq!(display_cols("abc"), 3);
        assert_eq!(display_cols("交易对象"), 8);
        assert_eq!(display_cols("2026-03-01 12:30:00"), DATETIME_DISPLAY_COLS);
        // 中英混排：2 个中文（4）+ 3 个 ASCII（3）
        assert_eq!(display_cols("备注abc"), 7);
    }

    /// 验证首行标题文本：全部信息拼成一行（不含换行），含对象、货币、区间与三项汇总
    #[test]
    fn title_line_should_join_all_info_in_one_line() {
        let data = sheet(vec![
            rec(Direction::In, 123456, 100, "2026-01-01 00:00:00"),
            rec(Direction::Out, 25, 1, "2026-01-02 00:00:00"),
        ]);
        let totals = summarize(&data.records);
        let line = title_line(&data, &totals);
        assert!(!line.contains('\n'), "标题必须是单行文本");
        assert!(line.contains("交易对象: 支付宝（RMB）"));
        assert!(line.contains("时间区间: 2026-01 ~ 2026-03"));
        // 流水总额三项金额后面都要带货币类型
        assert!(line.contains("流水总额-转入: 1,234.56 RMB"));
        assert!(line.contains("流水总额-转出: 25 RMB"));
        assert!(line.contains("流水总额-净额: 1,209.56 RMB"));

        // 月份区间为「不限」时同样可读
        let mut open = sheet(vec![]);
        open.month_from = None;
        open.month_to = None;
        let line = title_line(&open, &summarize(&[]));
        assert!(line.contains("时间区间: 不限"));

        // 货币为空时只显示金额，不追加多余空格
        let mut no_currency = sheet(vec![]);
        no_currency.currency = String::new();
        let line = title_line(&no_currency, &summarize(&[]));
        assert!(line.contains("流水总额-转入: 0 | 流水总额-转出: 0 |"));
        assert!(!line.contains("  "), "空货币不应留下双空格");
        assert!(!line.ends_with(' '), "空货币不应留下尾随空格");
    }

    /// 验证列宽：无数据行时按表头计算，且不低于下限（表头不被压扁）
    #[test]
    fn column_widths_should_use_header_when_no_records() {
        let widths = column_widths(&[], &HEADERS);
        for (idx, width) in widths.iter().enumerate() {
            let expected = (display_cols(HEADERS[idx]) as u32) * PX_PER_COL + CELL_PADDING_PX;
            assert_eq!(*width, expected.clamp(COLUMN_MIN_PX, COLUMN_MAX_PX));
        }
        // 表头「原始金额分子」为 12 个显示列 → 112px
        assert_eq!(widths[7], 112);
    }

    /// 验证列宽：日期列按格式化后的 19 列计算，足够显示 `yyyy-mm-dd hh:mm:ss`（回归：曾因 20 字符宽显示 `####`）
    #[test]
    fn column_widths_should_leave_room_for_datetime() {
        let records = [rec(Direction::Out, 1, 1, "2026-03-01 12:30:00")];
        let rows: Vec<&ExportRecord> = records.iter().collect();
        let widths = column_widths(&rows, &HEADERS);
        let expected = DATETIME_DISPLAY_COLS as u32 * PX_PER_COL + CELL_PADDING_PX;
        assert_eq!(widths[4], expected);
        assert!(
            widths[4] >= 160,
            "日期列过窄会把内容显示成 ####，实际 {}px",
            widths[4]
        );
    }

    /// 验证列宽：金额列按带千位分隔符的显示文本计算，不会被截断
    #[test]
    fn column_widths_should_account_for_amount_separators() {
        let records = [rec(Direction::In, 123456789, 100, "2026-03-01 00:00:00")];
        let rows: Vec<&ExportRecord> = records.iter().collect();
        let widths = column_widths(&rows, &HEADERS);
        // "1,234,567.89" 为 12 个显示列 → 112px
        assert_eq!(widths[1], 112);
    }

    /// 验证列宽：超长备注被钳制到上限，避免单列撑爆表格
    #[test]
    fn column_widths_should_clamp_long_text() {
        let mut long = rec(Direction::Out, 1, 1, "not-a-time");
        long.remark = Some("很长的备注".repeat(100));
        let records = [long];
        let rows: Vec<&ExportRecord> = records.iter().collect();
        let widths = column_widths(&rows, &HEADERS);
        assert_eq!(widths[5], COLUMN_MAX_PX);
        // 时间无法解析时退化为文本，按原始字符串长度计（"not-a-time" 为 10 列）
        assert_eq!(widths[4], 10 * PX_PER_COL + CELL_PADDING_PX);
    }
}
