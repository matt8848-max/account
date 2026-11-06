//! 中文日期组件
//!
//! 浏览器原生 `input type="date" / type="month"` 的显示语言与小时制跟随系统区域设置
//! （Windows 上常见英文月份名与 12 小时制 AM/PM），与软件其余部分的中文界面不一致。
//! 这里用「自绘日历面板 + 可手输数字输入（[`NumberField`]）」实现完全中文化的日期选择，
//! 且**同时支持手动输入**：
//!
//! - [`DatePicker`]：日期。输入框可直接键入 `YYYY-MM-DD`，也可点「📅」展开中文日历选择
//! - [`TimePicker`]：时/分，均可**直接键入**数字，也可用 ▲/▼ 微调（24 小时制）
//! - [`DayTimeInput`]：日期 + 时间组合，值形如 `YYYY-MM-DDTHH:MM`（与原组件接口一致，
//!   交易记录页 / 新建组表单无需改动即可获得「输入 + 选择」）
//! - [`MonthPicker`]：年月选择，值形如 `YYYY-MM`（可手输，或展开自定义面板点选月份；
//!   统计页月份范围还可设为「不限」）
//! - [`NumberField`]：可手输 + ▲/▼ 微调的数字输入（年/时/分复用同一组件）
//!
//! 纯逻辑（天数、星期对齐、格式解析）放在文件顶部，便于单元测试；
//! 组件只负责渲染与回调，平台无关（桌面 / Web 共用）。

use dioxus::prelude::*;

// ---------- 纯逻辑 ----------

/// 是否闰年（能被 4 整除但不能被 100 整除，或能被 400 整除）
fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// 某年某月的天数；非法月份返回 0（日历面板据此渲染空表）
pub(crate) fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// 某日是星期几：`0` = 周一 … `6` = 周日（日历面板按「一」到「日」排列）
///
/// 采用 Sakamoto 算法：先得到 0 = 周日 … 6 = 周六，再平移为周一起算。
/// 只用于 1900~2100 年的日期，无需考虑格里高利历改革前的差异。
pub(crate) fn weekday_index_monday(year: i32, month: u32, day: u32) -> u32 {
    if !(1..=12).contains(&month) {
        return 0;
    }
    const OFFSETS: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    let idx = (month as usize) - 1;
    let w = (y + y / 4 - y / 100 + y / 400 + OFFSETS[idx] + day as i32).rem_euclid(7);
    ((w + 6) % 7) as u32
}

/// 月份加减（`delta` 可为负），返回 `(年, 月)`
///
/// 用「总月数」做整除/取余，跨年与跨多年都只需一次运算。
pub(crate) fn shift_month(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let total = year * 12 + (month as i32 - 1) + delta;
    (total.div_euclid(12), total.rem_euclid(12) as u32 + 1)
}

/// 解析 `YYYY-MM-DD` / `YYYY-M-D` → `(年, 月, 日)`
///
/// 允许非补零写法（方便手动输入），但必须是**真实存在的日期**（如 2 月 30 日非法）。
pub(crate) fn parse_date(text: &str) -> Option<(i32, u32, u32)> {
    let mut parts = text.trim().split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    // 多于三段（如 `2026-11-06-01`）视为非法
    if parts.next().is_some() {
        return None;
    }
    if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        return None;
    }
    Some((year, month, day))
}

/// 解析 `YYYY-MM` / `YYYY-M` → `(年, 月)`
pub(crate) fn parse_month(text: &str) -> Option<(i32, u32)> {
    let mut parts = text.trim().split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) {
        return None;
    }
    Some((year, month))
}

/// 格式化日期为补零的 `YYYY-MM-DD`
pub(crate) fn format_date(year: i32, month: u32, day: u32) -> String {
    format!("{year:04}-{month:02}-{day:02}")
}

/// 格式化月份为补零的 `YYYY-MM`
pub(crate) fn format_month(year: i32, month: u32) -> String {
    format!("{year:04}-{month:02}")
}

/// 把数字文本收敛到 `[min, max]` 并补零到 `width` 位
///
/// 非数字或空串回退到 `min`：`年` / `时` / `分` 这类字段不允许留空，
/// 清空后视为下界（0 时 0 分、1900 年），失焦即恢复为合法值。
pub(crate) fn clamp_number(text: &str, min: u32, max: u32, width: usize) -> String {
    let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
    let value = digits.parse::<u32>().unwrap_or(min).clamp(min, max);
    format!("{value:0width$}", width = width)
}

/// 在 `[min, max]` 范围内步进（到边界后钳住，不回绕）
pub(crate) fn step_number(text: &str, min: u32, max: u32, delta: i32, width: usize) -> String {
    let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
    let current = digits.parse::<i64>().unwrap_or(min as i64);
    let next = (current + delta as i64).clamp(min as i64, max as i64) as u32;
    format!("{next:0width$}", width = width)
}

/// 把 `YYYY-MM-DDTHH:MM` 拆成 `(日期, 时, 分)`，越界值会被收敛到合法范围
///
/// 组件内部统一用这个函数读取值，因此上游即使传入半截文本（用户正在输入）
/// 也能得到可渲染的默认值，不会 panic。
pub(crate) fn split_datetime(value: &str) -> (String, String, String) {
    let (date, time) = match value.split_once('T') {
        Some((d, t)) => (d.to_string(), t.to_string()),
        None => (value.trim().to_string(), String::new()),
    };
    let mut hm = time.split(':');
    let hour = hm
        .next()
        .and_then(|h| h.trim().parse::<u32>().ok())
        .filter(|h| *h < 24)
        .unwrap_or(0);
    let minute = hm
        .next()
        .and_then(|m| m.trim().parse::<u32>().ok())
        .filter(|m| *m < 60)
        .unwrap_or(0);
    (date, format!("{hour:02}"), format!("{minute:02}"))
}

/// 面板当前浏览的 `(年, 月)`：优先取已选值，否则取今天
fn browsed_month_of(value: &str) -> (i32, u32) {
    parse_date(value)
        .map(|(y, m, _)| (y, m))
        .or_else(|| {
            let (date, _, _) = split_datetime(&super::now_local_iso());
            parse_date(&date).map(|(y, m, _)| (y, m))
        })
        .unwrap_or((2026, 1))
}

/// 今天：`(年, 月, 日)`
fn today() -> (i32, u32, u32) {
    let (date, _, _) = split_datetime(&super::now_local_iso());
    parse_date(&date).unwrap_or((2026, 1, 1))
}

// ---------- 组件 ----------

/// 中文日期选择：输入框可直接键入，点「📅」展开日历面板选择
///
/// `value` 约定为 `YYYY-MM-DD`（空串表示未选择），变更时通过 `onchange` 回传。
#[component]
pub(crate) fn DatePicker(value: String, onchange: EventHandler<String>) -> Element {
    // 面板是否展开
    let mut open = use_signal(|| false);
    // 面板浏览的 (年, 月)
    let mut view = use_signal(|| browsed_month_of(&value));
    let (view_year, view_month) = (view)();
    let selected = parse_date(&value);
    let today_parts = today();

    // 首日之前用空白补齐，使「一」列对齐；尾部补齐整行避免最后一行被拉伸
    let total_days = days_in_month(view_year, view_month);
    let lead = weekday_index_monday(view_year, view_month, 1) as usize;
    let mut cells: Vec<Option<u32>> = vec![None; lead];
    cells.extend((1..=total_days).map(Some));
    while !cells.len().is_multiple_of(7) {
        cells.push(None);
    }

    let year_text = format!("{view_year:04}");
    let month_text = format!("{view_month:02}");
    // 闭包是 FnMut（可多次调用），String 又非 Copy：每个回调各持一份副本
    let value_for_blur = value.clone();
    let value_for_open = value.clone();

    rsx! {
        div { style: "position:relative;display:inline-block",
            input {
                // 输入框直接绑定传入值：既能手打日期，也能被面板选择回填
                value: "{value}",
                placeholder: "YYYY-MM-DD",
                style: "width:110px",
                oninput: move |e| onchange.call(e.value()),
                // 失焦时规范化：合法日期补零（2026-1-6 → 2026-01-06），
                // 非法文本保持原样，交给业务层校验后提示
                onblur: move |_| {
                    let text = value_for_blur.clone();
                    if let Some((y, m, d)) = parse_date(&text) {
                        let normalized = format_date(y, m, d);
                        if normalized != text {
                            onchange.call(normalized);
                        }
                    }
                },
            }
            button {
                style: "margin-left:2px",
                title: "选择日期",
                onclick: move |_| {
                    // 展开时把面板对齐到已选日期（未选则对齐到今天）
                    let current = value_for_open.clone();
                    view.set(browsed_month_of(&current));
                    open.toggle();
                },
                "📅"
            }

            if (open)() {
                div {
                    style: "position:absolute;top:100%;left:0;margin-top:4px;background:#fff;border:1px solid #ccc;border-radius:6px;box-shadow:0 4px 12px rgba(0,0,0,0.18);padding:8px;z-index:800;width:264px",
                    // 年月选择 + 上/下年、上/下月切换
                    div { style: "display:flex;align-items:center;gap:4px",
                        button {
                            title: "上一年",
                            onclick: move |_| {
                                let (y, m) = (view)();
                                view.set(shift_month(y, m, -12));
                            },
                            "«"
                        }
                        button {
                            title: "上一月",
                            onclick: move |_| {
                                let (y, m) = (view)();
                                view.set(shift_month(y, m, -1));
                            },
                            "‹"
                        }
                        // 年 / 月用自定义数字输入（可手输 + ▲/▼ 微调），
                        // 不再用原生 <select>（其样式与语言随系统变化）
                        NumberField {
                            value: year_text,
                            min: 1900,
                            max: 2100,
                            width: 4,
                            onchange: move |y: String| {
                                if let Ok(v) = y.parse::<i32>() {
                                    let (_, m) = (view)();
                                    view.set((v.clamp(1900, 2100), m));
                                }
                            },
                        }
                        span { "年" }
                        NumberField {
                            value: month_text,
                            min: 1,
                            max: 12,
                            width: 2,
                            onchange: move |m: String| {
                                if let Ok(v) = m.parse::<u32>() {
                                    let (y, _) = (view)();
                                    view.set((y, v.clamp(1, 12)));
                                }
                            },
                        }
                        span { "月" }
                        button {
                            title: "下一月",
                            onclick: move |_| {
                                let (y, m) = (view)();
                                view.set(shift_month(y, m, 1));
                            },
                            "›"
                        }
                        button {
                            title: "下一年",
                            onclick: move |_| {
                                let (y, m) = (view)();
                                view.set(shift_month(y, m, 12));
                            },
                            "»"
                        }
                    }

                    // 星期表头（周一起算，与日期格排列一致）
                    div { style: "display:grid;grid-template-columns:repeat(7,1fr);gap:2px;margin-top:6px;font-size:12px;color:#888;text-align:center",
                        for w in ["一", "二", "三", "四", "五", "六", "日"] {
                            div { "{w}" }
                        }
                    }

                    // 日期格：空白格不可点；选中项填充高亮；今天描边
                    div { style: "display:grid;grid-template-columns:repeat(7,1fr);gap:2px;margin-top:2px",
                        for day in cells.iter().cloned() {
                            if let Some(d) = day {
                                button {
                                    style: if selected == Some((view_year, view_month, d)) {
                                        "padding:2px 0;border:1px solid #3498db;background:#3498db;color:#fff;border-radius:4px;cursor:pointer"
                                    } else if today_parts == (view_year, view_month, d) {
                                        "padding:2px 0;border:1px solid #3498db;background:#fff;color:#333;border-radius:4px;cursor:pointer"
                                    } else {
                                        "padding:2px 0;border:1px solid transparent;background:#fff;color:#333;border-radius:4px;cursor:pointer"
                                    },
                                    // 用 mousedown + prevent_default：避免输入框先失焦打断点击
                                    onmousedown: move |e| {
                                        e.prevent_default();
                                        onchange.call(format_date(view_year, view_month, d));
                                        open.set(false);
                                    },
                                    "{d}"
                                }
                            } else {
                                div { style: "padding:2px 0" }
                            }
                        }
                    }

                    // 底部操作：今天 / 清空 / 关闭
                    div { style: "display:flex;gap:6px;justify-content:flex-end;margin-top:6px",
                        button {
                            onclick: move |_| {
                                let (y, m, d) = today();
                                view.set((y, m));
                                onchange.call(format_date(y, m, d));
                                open.set(false);
                            },
                            "今天"
                        }
                        button {
                            onclick: move |_| {
                                onchange.call(String::new());
                                open.set(false);
                            },
                            "清空"
                        }
                        button { onclick: move |_| open.set(false), "关闭" }
                    }
                }
            }
        }
    }
}
/// 数字输入（可**直接键入** + ▲/▼ 微调）
///
/// - `value`：当前显示值（补零文本，如 `08`、`2026`）
/// - `min` / `max`：允许范围；手输超界或清空时在**失焦后**收敛到边界
/// - `width`：补零宽度（2 = 时/分，4 = 年份）
///
/// 输入过程中只过滤非数字字符、**不立刻补零或截断**：
/// 否则用户输入「15」时会被中间态（「01」补零、或长度截断）打断。
/// 补零与夹紧统一由失焦时的 [`clamp_number`] 完成，步进按钮用 [`step_number`]。
#[component]
pub(crate) fn NumberField(
    value: String,
    min: u32,
    max: u32,
    width: usize,
    onchange: EventHandler<String>,
) -> Element {
    // 输入框宽度按数字位数留出余量（ch 单位随字号）
    let input_width = width.max(1) + 2;
    // 闭包是 FnMut，String 非 Copy：每个回调各持一份副本
    let value_for_blur = value.clone();
    let value_for_up = value.clone();
    let value_for_down = value.clone();

    rsx! {
        div { style: "display:inline-flex;align-items:center;gap:2px",
            input {
                // 触屏设备弹数字键盘
                inputmode: "numeric",
                value: "{value}",
                style: "width:{input_width}ch;text-align:center",
                oninput: move |e| {
                    let digits: String = e
                        .value()
                        .chars()
                        .filter(|c| c.is_ascii_digit())
                        .collect();
                    onchange.call(digits);
                },
                // 失焦收敛：清空回退到下界、超界夹到边界，并补零
                onblur: move |_| onchange.call(clamp_number(&value_for_blur, min, max, width)),
            }
            div { style: "display:flex;flex-direction:column;gap:1px",
                button {
                    title: "加一",
                    style: "font-size:8px;line-height:8px;padding:1px 3px;background:#fff;border:1px solid #ccc;border-radius:2px;cursor:pointer",
                    onclick: move |_| onchange.call(step_number(&value_for_up, min, max, 1, width)),
                    "▲"
                }
                button {
                    title: "减一",
                    style: "font-size:8px;line-height:8px;padding:1px 3px;background:#fff;border:1px solid #ccc;border-radius:2px;cursor:pointer",
                    onclick: move |_| onchange.call(step_number(&value_for_down, min, max, -1, width)),
                    "▼"
                }
            }
        }
    }
}

/// 时分输入（24 小时制）
///
/// 时、分都**可直接键入**数字，也可用 ▲/▼ 微调；`time` 约定为 `HH:MM`。
/// 键入的数字原样回传上游，渲染时再由 [`split_datetime`] 收敛为两位，
/// 因此输入「15」不会被中间态打断（先出现「01」再补成「15」也成立）。
#[component]
pub(crate) fn TimePicker(time: String, onchange: EventHandler<String>) -> Element {
    // 借用占位日期复用 split_datetime 的收敛逻辑，避免再写一套解析实现
    let (_, hour, minute) = split_datetime(&format!("1970-01-01T{time}"));
    // 闭包是 FnMut：两个字段各持一份对方取值的副本
    let minute_for_hour = minute.clone();
    let hour_for_minute = hour.clone();

    rsx! {
        div { style: "display:flex;gap:4px;align-items:center",
            NumberField {
                value: hour,
                min: 0,
                max: 23,
                width: 2,
                onchange: move |h: String| onchange.call(format!("{h}:{minute_for_hour}")),
            }
            span { "时" }
            NumberField {
                value: minute,
                min: 0,
                max: 59,
                width: 2,
                onchange: move |m: String| onchange.call(format!("{hour_for_minute}:{m}")),
            }
            span { "分" }
        }
    }
}

/// 日期 + 时间输入（`YYYY-MM-DDTHH:MM`）
///
/// 接口与原实现保持一致（`value` / `onchange`），但日期部分改为中文日历 + 可手输，
/// 时间部分为「时」「分」两个可**直接键入**的数字输入（也可 ▲/▼ 微调）。
/// 交易记录页与新建组表单直接复用本组件。
#[component]
pub(crate) fn DayTimeInput(value: String, onchange: EventHandler<String>) -> Element {
    let (date, hour, minute) = split_datetime(&value);
    // 闭包需 move 捕获，String 非 Copy，两个回调各持一份副本
    let date_for_date = date.clone();
    let hour_for_date = hour.clone();
    let minute_for_date = minute.clone();
    let date_for_time = date;
    let time_text = format!("{hour}:{minute}");

    rsx! {
        div { style: "display:flex;gap:6px;align-items:center;flex-wrap:wrap",
            DatePicker {
                value: date_for_date,
                onchange: move |d: String| {
                    // 日期被清空时整体清空，避免留下「只有时间」的半截值
                    if d.is_empty() {
                        onchange.call(String::new());
                    } else {
                        onchange.call(format!("{d}T{hour_for_date}:{minute_for_date}"));
                    }
                },
            }
            TimePicker {
                time: time_text,
                onchange: move |t: String| {
                    if date_for_time.is_empty() {
                        onchange.call(String::new());
                    } else {
                        onchange.call(format!("{date_for_time}T{t}"));
                    }
                },
            }
        }
    }
}

/// 年月选择（`YYYY-MM`）
///
/// 与 [`DatePicker`] 同一套交互：输入框可**直接键入** `YYYY-MM`，
/// 也可点「📅」展开自定义面板——面板用 [`NumberField`] 选年、月份以 3×4 格子点选，
/// 不再依赖浏览器原生 `<select>`（语言与样式随系统变化）。
///
/// - `allow_empty`：为真时提供「不限」入口清空取值（统计页月份范围可留空）
#[component]
pub(crate) fn MonthPicker(
    value: String,
    allow_empty: bool,
    onchange: EventHandler<String>,
) -> Element {
    // 面板是否展开
    let mut open = use_signal(|| false);
    // 面板浏览的年份（月份用格子直接点选，无需单独状态）
    let mut view_year = use_signal(|| {
        parse_month(&value)
            .map(|(y, _)| y)
            .unwrap_or_else(|| today().0)
    });
    let selected = parse_month(&value);
    let browse_year = (view_year)();
    let year_text = format!("{browse_year:04}");
    let months: Vec<u32> = (1..=12).collect();
    // 闭包是 FnMut，String 非 Copy：每个回调各持一份副本
    let value_for_blur = value.clone();
    let value_for_open = value.clone();
    let placeholder = if allow_empty { "不限" } else { "YYYY-MM" };

    rsx! {
        div { style: "position:relative;display:inline-block",
            input {
                // 与 DatePicker 一致：可直接键入 YYYY-MM
                value: "{value}",
                placeholder: "{placeholder}",
                style: "width:90px",
                oninput: move |e| onchange.call(e.value()),
                // 失焦规范化：合法年月补零（2026-8 → 2026-08）；
                // 允许为空时清空即表示「不限」，非法文本保持原样交给业务校验
                onblur: move |_| {
                    let text = value_for_blur.trim().to_string();
                    if text.is_empty() {
                        if allow_empty {
                            onchange.call(String::new());
                        }
                        return;
                    }
                    if let Some((y, m)) = parse_month(&text) {
                        let normalized = format_month(y, m);
                        if normalized != text {
                            onchange.call(normalized);
                        }
                    }
                },
            }
            button {
                style: "margin-left:2px",
                title: "选择年月",
                onclick: move |_| {
                    // 展开时把面板对齐到已选年月（未选则对齐到本月）
                    let current = value_for_open.trim().to_string();
                    view_year.set(
                        parse_month(&current)
                            .map(|(y, _)| y)
                            .unwrap_or_else(|| today().0),
                    );
                    open.toggle();
                },
                "📅"
            }

            if (open)() {
                div {
                    style: "position:absolute;top:100%;left:0;margin-top:4px;background:#fff;border:1px solid #ccc;border-radius:6px;box-shadow:0 4px 12px rgba(0,0,0,0.18);padding:8px;z-index:800;width:232px",
                    // 年份：可手输 + ▲/▼ 微调（与日期面板共用 NumberField）
                    div { style: "display:flex;align-items:center;justify-content:center;gap:4px",
                        button {
                            title: "上一年",
                            onclick: move |_| view_year.set((browse_year - 1).clamp(1900, 2100)),
                            "‹"
                        }
                        NumberField {
                            value: year_text,
                            min: 1900,
                            max: 2100,
                            width: 4,
                            onchange: move |y: String| {
                                if let Ok(v) = y.parse::<i32>() {
                                    view_year.set(v.clamp(1900, 2100));
                                }
                            },
                        }
                        span { "年" }
                        button {
                            title: "下一年",
                            onclick: move |_| view_year.set((browse_year + 1).clamp(1900, 2100)),
                            "›"
                        }
                    }

                    // 月份：3×4 格子点选（当前年月高亮）
                    div { style: "display:grid;grid-template-columns:repeat(3,1fr);gap:4px;margin-top:8px",
                        for m in months.iter().cloned() {
                            button {
                                style: if selected == Some((browse_year, m)) {
                                    "padding:4px 0;border:1px solid #3498db;background:#3498db;color:#fff;border-radius:4px;cursor:pointer"
                                } else {
                                    "padding:4px 0;border:1px solid #ddd;background:#fff;color:#333;border-radius:4px;cursor:pointer"
                                },
                                // mousedown + prevent_default：避免输入框先失焦打断点击
                                onmousedown: move |e| {
                                    e.prevent_default();
                                    onchange.call(format_month(browse_year, m));
                                    open.set(false);
                                },
                                "{m} 月"
                            }
                        }
                    }

                    // 底部操作：本月 / 不限 / 关闭
                    div { style: "display:flex;gap:6px;justify-content:flex-end;margin-top:8px",
                        button {
                            onclick: move |_| {
                                let (y, m, _) = today();
                                view_year.set(y);
                                onchange.call(format_month(y, m));
                                open.set(false);
                            },
                            "本月"
                        }
                        if allow_empty {
                            button {
                                title: "不限制月份",
                                onclick: move |_| {
                                    onchange.call(String::new());
                                    open.set(false);
                                },
                                "不限"
                            }
                        }
                        button { onclick: move |_| open.set(false), "关闭" }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, NaiveDate};

    /// 验证每月天数：与 chrono 对 1900~2100 年全部月份逐一比对（含闰年与世纪闰年规则）
    #[test]
    fn days_in_month_should_match_calendar() {
        for year in 1900..=2100 {
            for month in 1..=12u32 {
                let next = if month == 12 {
                    NaiveDate::from_ymd_opt(year + 1, 1, 1)
                } else {
                    NaiveDate::from_ymd_opt(year, month + 1, 1)
                }
                .expect("构造下个月首日");
                let expected = next.pred_opt().expect("上一天").day();
                assert_eq!(
                    days_in_month(year, month),
                    expected,
                    "{year}-{month} 的天数错误"
                );
            }
        }
        // 不相等断言：2 月天数必须随闰年变化，不能恒为 28
        assert_ne!(days_in_month(2024, 2), days_in_month(2023, 2));
        // 非法月份返回 0（日历面板据此渲染空表）
        assert_eq!(days_in_month(2026, 0), 0);
        assert_eq!(days_in_month(2026, 13), 0);
    }

    /// 验证星期对齐：与 chrono 对 1900~2100 年每月 1 日与月末逐一比对（0 = 周一）
    #[test]
    fn weekday_index_should_match_calendar() {
        for year in 1900..=2100 {
            for month in 1..=12u32 {
                let last = days_in_month(year, month);
                for day in [1, last] {
                    let date = NaiveDate::from_ymd_opt(year, month, day).expect("合法日期");
                    let expected = date.weekday().num_days_from_monday();
                    assert_eq!(
                        weekday_index_monday(year, month, day),
                        expected,
                        "{year}-{month}-{day} 的星期对齐错误"
                    );
                }
            }
        }
    }

    /// 验证月份加减：跨年、跨多年与负偏移都能正确进位/借位
    #[test]
    fn shift_month_should_cross_years() {
        assert_eq!(shift_month(2026, 1, -1), (2025, 12));
        assert_eq!(shift_month(2026, 12, 1), (2027, 1));
        assert_eq!(shift_month(2026, 6, 0), (2026, 6));
        assert_eq!(shift_month(2026, 6, 7), (2027, 1));
        assert_eq!(shift_month(2026, 6, -18), (2024, 12));
        assert_eq!(shift_month(2026, 6, 18), (2027, 12));
        // 不相等断言：偏移 0 与偏移 ±1 的结果必须不同（防止实现忽略 delta）
        assert_ne!(shift_month(2026, 6, 0), shift_month(2026, 6, 1));
    }

    /// 验证日期解析：接受补零与非补零写法，拒绝不存在的日期与多余字段
    #[test]
    fn parse_date_should_accept_valid_and_reject_invalid() {
        assert_eq!(parse_date("2026-11-06"), Some((2026, 11, 6)));
        // 非补零写法（手输常见）
        assert_eq!(parse_date("2026-1-6"), Some((2026, 1, 6)));
        // 前后空白会被裁剪
        assert_eq!(parse_date(" 2026-02-28 "), Some((2026, 2, 28)));
        // 闰年 2 月 29 日合法，平年非法
        assert_eq!(parse_date("2024-02-29"), Some((2024, 2, 29)));
        assert_eq!(parse_date("2023-02-29"), None);

        assert_eq!(parse_date("2026-13-01"), None);
        assert_eq!(parse_date("2026-04-31"), None);
        assert_eq!(parse_date("2026-00-10"), None);
        assert_eq!(parse_date("2026-11-00"), None);
        assert_eq!(parse_date("2026-11-06-01"), None);
        assert_eq!(parse_date("2026/11/06"), None);
        assert_eq!(parse_date(""), None);
        assert_eq!(parse_date("abc"), None);
    }

    /// 验证月份解析：补零/非补零都接受，月份越界与缺月拒绝
    #[test]
    fn parse_month_should_accept_valid_and_reject_invalid() {
        assert_eq!(parse_month("2026-08"), Some((2026, 8)));
        assert_eq!(parse_month("2026-8"), Some((2026, 8)));
        assert_eq!(parse_month("2026-13"), None);
        assert_eq!(parse_month("2026-00"), None);
        assert_eq!(parse_month("2026"), None);
        assert_eq!(parse_month("2026-08-01"), None);
        assert_eq!(parse_month(""), None);
    }

    /// 验证格式化：月/日补零，且与解析互为逆运算
    #[test]
    fn format_should_zero_pad() {
        assert_eq!(format_date(2026, 1, 6), "2026-01-06");
        assert_eq!(format_month(2026, 8), "2026-08");
        // 不相等断言：补零不能省略分隔符
        assert_ne!(format_date(2026, 1, 6), "20260106");
        // 解析与格式化互逆
        assert_eq!(parse_date(&format_date(2026, 12, 31)), Some((2026, 12, 31)));
        assert_eq!(parse_month(&format_month(2026, 7)), Some((2026, 7)));
    }

    /// 验证数字收敛：补零、夹到边界、空/非法文本回退到下界、非数字字符被过滤
    #[test]
    fn clamp_number_should_pad_and_clamp() {
        assert_eq!(clamp_number("8", 0, 23, 2), "08");
        assert_eq!(clamp_number("15", 0, 23, 2), "15");
        // 超上界夹到 max
        assert_eq!(clamp_number("99", 0, 23, 2), "23");
        // 空串 / 非数字回退到下界（时分为 0、年份为 1900）
        assert_eq!(clamp_number("", 0, 23, 2), "00");
        assert_eq!(clamp_number("abc", 0, 23, 2), "00");
        assert_eq!(clamp_number("", 1900, 2100, 4), "1900");
        // 年份：合法值原样、越界夹紧
        assert_eq!(clamp_number("2026", 1900, 2100, 4), "2026");
        assert_eq!(clamp_number("1234", 1900, 2100, 4), "1900");
        // 混入的非数字字符被过滤（"1a5" → 15）
        assert_eq!(clamp_number("1a5", 0, 23, 2), "15");
        // 不相等断言：补零不能改变数值（不能把 8 变成 80）
        assert_ne!(clamp_number("8", 0, 23, 2), "80");
    }

    /// 验证步进：正常加减，到边界后钳住而不回绕
    #[test]
    fn step_number_should_clamp_at_bounds() {
        assert_eq!(step_number("08", 0, 23, 1, 2), "09");
        assert_eq!(step_number("08", 0, 23, -1, 2), "07");
        // 上界 +1 保持上界、下界 -1 保持下界（不回绕）
        assert_eq!(step_number("23", 0, 23, 1, 2), "23");
        assert_eq!(step_number("00", 0, 23, -1, 2), "00");
        assert_eq!(step_number("59", 0, 59, 1, 2), "59");
        assert_eq!(step_number("2026", 1900, 2100, 1, 4), "2027");
        assert_eq!(step_number("2100", 1900, 2100, 1, 4), "2100");
        // 不相等断言：正常步进必须真的改变数值
        assert_ne!(step_number("08", 0, 23, 1, 2), "08");
    }

    /// 验证 datetime 拆分：缺时间/缺日期/越界时间都被收敛为可渲染的默认值
    #[test]
    fn split_datetime_should_normalize_parts() {
        assert_eq!(
            split_datetime("2026-11-06T08:05"),
            ("2026-11-06".to_string(), "08".to_string(), "05".to_string())
        );
        // 只有日期：时间补 00
        assert_eq!(
            split_datetime("2026-11-06"),
            ("2026-11-06".to_string(), "00".to_string(), "00".to_string())
        );
        // 非补零时间被规范化
        assert_eq!(
            split_datetime("2026-11-06T8:5"),
            ("2026-11-06".to_string(), "08".to_string(), "05".to_string())
        );
        // 越界时间收敛到 0，不会 panic
        assert_eq!(
            split_datetime("2026-11-06T99:99"),
            ("2026-11-06".to_string(), "00".to_string(), "00".to_string())
        );
        // 空值与半截输入
        assert_eq!(
            split_datetime(""),
            (String::new(), "00".to_string(), "00".to_string())
        );
        assert_eq!(
            split_datetime("2026-1"),
            ("2026-1".to_string(), "00".to_string(), "00".to_string())
        );
        // 不相等断言：拆分结果不能把分钟当成小时
        assert_ne!(
            split_datetime("2026-11-06T08:05"),
            ("2026-11-06".to_string(), "05".to_string(), "08".to_string())
        );
    }
}
