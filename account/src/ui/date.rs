//! 日期/月份选择封装
//!
//! 使用 gpui-kit `DatePicker`，避免手输日期：
//! - 交易时间 / 查询起止：日期 + 时间（分钟精度），显示 `%Y-%m-%d %H:%M`
//! - 统计月份：显示 `%Y-%m`（选择任意日期后取年月）
//!
//! 本模块只做状态创建、读取与格式转换；渲染由各页面直接使用 `DatePicker`。

use super::app::AccountApp;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Timelike as _};
use gpui_kit::AppContext as _;
use gpui_kit::component::date_picker::{DatePickerState, DateTime as PickerDate};
use gpui_kit::component::time_field::TimePrecision;
use gpui_kit::{Context, Entity, Window};

/// 日期时间显示格式（DatePicker 输入框）。
pub const DATETIME_FORMAT: &str = "%Y-%m-%d %H:%M";
/// 月份显示格式。
pub const MONTH_FORMAT: &str = "%Y-%m";
/// 提交给后端的 ISO 时间格式。
pub const API_DATETIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%S";

/// 创建日期时间选择器（分钟精度）。
pub fn new_datetime_picker(
    window: &mut Window,
    cx: &mut Context<AccountApp>,
    default: Option<NaiveDateTime>,
) -> Entity<DatePickerState> {
    cx.new(|cx| {
        let mut state = DatePickerState::new(window, cx)
            .time_precision(TimePrecision::Minute)
            .date_format(DATETIME_FORMAT)
            .default_time(NaiveTime::from_hms_opt(0, 0, 0).expect("合法时间"));
        if let Some(dt) = default {
            state.set_date_time(dt, window, cx);
        }
        state
    })
}

/// 创建月份选择器（显示 `YYYY-MM`）。
pub fn new_month_picker(
    window: &mut Window,
    cx: &mut Context<AccountApp>,
    default: Option<NaiveDate>,
) -> Entity<DatePickerState> {
    cx.new(|cx| {
        let mut state = DatePickerState::new(window, cx).date_format(MONTH_FORMAT);
        if let Some(date) = default {
            state.set_date(date, window, cx);
        }
        state
    })
}

/// 读取选择器中的日期时间；未选择返回 None。
pub fn picker_datetime(state: &DatePickerState) -> Option<NaiveDateTime> {
    match state.date_time() {
        PickerDate::Single(Some(dt)) => Some(dt),
        _ => None,
    }
}

/// 读取选择器中的月份（`YYYY-MM`）；未选择返回 None。
pub fn picker_month(state: &DatePickerState) -> Option<String> {
    picker_datetime(state).map(|dt| dt.format(MONTH_FORMAT).to_string())
}

/// 当前本地日期时间（精确到分钟）。
pub fn now_local() -> NaiveDateTime {
    let now = chrono::Local::now().naive_local();
    now.with_second(0)
        .unwrap_or(now)
        .with_nanosecond(0)
        .unwrap_or(now)
}

/// 当前时间往前 N 天。
pub fn now_minus_days(days: i64) -> NaiveDateTime {
    now_local() - chrono::Duration::days(days)
}

/// 当前本地月份 `YYYY-MM`。
pub fn now_month() -> String {
    chrono::Local::now().format(MONTH_FORMAT).to_string()
}

/// 日期时间 → 后端 ISO 字符串（补秒）。
pub fn to_api_datetime(dt: NaiveDateTime) -> String {
    dt.format(API_DATETIME_FORMAT).to_string()
}

/// 后端 ISO 字符串 → 日期时间；兼容不带秒的旧格式。
pub fn from_api_datetime(text: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(text, API_DATETIME_FORMAT)
        .or_else(|_| NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M"))
        .ok()
}

/// 后端 ISO 时间 → 界面显示（`YYYY-MM-DD HH:MM`）；解析失败原样返回。
pub fn display_api_datetime(text: &str) -> String {
    from_api_datetime(text)
        .map(|dt| dt.format(DATETIME_FORMAT).to_string())
        .unwrap_or_else(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证后端时间格式补秒与解析可互相还原。
    #[test]
    fn api_datetime_should_roundtrip() {
        let dt = NaiveDate::from_ymd_opt(2026, 11, 17)
            .unwrap()
            .and_hms_opt(20, 5, 0)
            .unwrap();
        let text = to_api_datetime(dt);
        assert_eq!(text, "2026-11-17T20:05:00");
        assert_eq!(from_api_datetime(&text), Some(dt));
        assert_eq!(from_api_datetime("2026-11-17T20:05"), Some(dt));
        assert_eq!(from_api_datetime("abc"), None);
        assert_eq!(
            display_api_datetime("2026-11-17T20:05:00"),
            "2026-11-17 20:05"
        );
        // 不相等断言：不能把不带秒的字符串原样当带秒格式
        assert_ne!(text, "2026-11-17T20:05");
    }

    /// 验证月份格式。
    #[test]
    fn month_format_should_be_year_month() {
        let dt = NaiveDate::from_ymd_opt(2026, 1, 31)
            .unwrap()
            .and_hms_opt(23, 59, 0)
            .unwrap();
        assert_eq!(dt.format(MONTH_FORMAT).to_string(), "2026-01");
        // 不相等断言：不能包含日
        assert_ne!(dt.format(MONTH_FORMAT).to_string(), "2026-01-31");
    }
}
