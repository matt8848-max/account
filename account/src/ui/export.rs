//! 导出落盘（保存对话框 + 平台差异）
//!
//! xlsx 的生成在 `crate::export`（平台无关），本模块只负责「已有字节 → 用户指定位置」：
//! - wasm（Web）：只填文件名，通过浏览器下载（`Blob` + `<a download>`）
//! - native（桌面）：选择保存路径与文件名后写文件（rfd 原生「另存为」对话框）
//!
//! 平台差异用 `#[cfg(target_arch = "wasm32")]` 封装成两个内部组件，外层对话框不感知差异。

use super::{GlobalState, RecordView, push_toast};
use crate::export::{Direction, ExportRecord, ExportSheet, ensure_xlsx_suffix};
use crate::models::Counterparty;
use dioxus::prelude::*;

/// 待保存的导出结果（生成好字节后交给保存对话框）
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PendingExport {
    /// xlsx 文件字节
    pub bytes: Vec<u8>,
    /// 默认文件名（含 `.xlsx`）
    pub default_name: String,
}

/// 由某交易对象的记录视图构建导出表。
///
/// - 方向：该对象是转出方为「转出」，否则为「转入」，与本记录无关的记录直接跳过；
/// - 对方对象：转出时取 `to`，转入时取 `from`；
/// - 分组：无组名时回退为 `组#ID`（与记录页展示保持一致）。
pub(crate) fn sheet_from_views(
    counterparty: &Counterparty,
    currency: &str,
    month_from: Option<String>,
    month_to: Option<String>,
    views: &[RecordView],
) -> ExportSheet {
    let mut records = Vec::new();
    for view in views {
        let Some(direction) = Direction::of(view.from_id, view.to_id, counterparty.id) else {
            continue;
        };
        let other_party = match direction {
            Direction::Out => view.to.clone(),
            Direction::In => view.from.clone(),
        };
        records.push(ExportRecord {
            direction,
            amount: view.amount,
            currency: view.currency.clone(),
            other_party,
            traded_at: view.traded_at.clone(),
            remark: view.remark.clone(),
            group: view
                .group_name
                .clone()
                .or_else(|| Some(format!("组#{}", view.group_id))),
        });
    }
    ExportSheet {
        counterparty: counterparty.name.clone(),
        currency: currency.to_string(),
        month_from,
        month_to,
        records,
    }
}

/// 导出保存对话框：确认文件名（桌面端还要确认路径）后写盘或下载。
///
/// - `pending`：已生成的 xlsx 字节与默认文件名
/// - `on_done`：保存成功回调，参数为最终文件名或完整路径（用于提示）
/// - `on_cancel`：取消回调
#[component]
pub(crate) fn ExportSaveDialog(
    pending: PendingExport,
    on_done: EventHandler<String>,
    on_cancel: EventHandler<()>,
) -> Element {
    let mut file_name = use_signal(|| pending.default_name.clone());
    let name_value = (file_name)();
    let bytes = pending.bytes.clone();
    let fallback = pending.default_name.clone();

    // 平台差异：Web 只有「下载」，桌面还有「选择保存位置」
    #[cfg(target_arch = "wasm32")]
    let controls: Element = rsx! {
        WebSaveControls {
            file_name: name_value.clone(),
            fallback: fallback.clone(),
            bytes: bytes.clone(),
            on_saved: move |name: String| on_done.call(name),
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    let controls: Element = rsx! {
        NativeSaveControls {
            file_name: name_value.clone(),
            fallback: fallback.clone(),
            bytes: bytes.clone(),
            on_saved: move |path: String| on_done.call(path),
        }
    };

    rsx! {
        div {
            style: "position:fixed;inset:0;background:rgba(0,0,0,0.35);display:flex;align-items:center;justify-content:center;z-index:1600",
            div {
                style: "background:#fff;border-radius:8px;padding:18px;min-width:360px;max-width:92vw;display:flex;flex-direction:column;gap:10px;box-shadow:0 4px 16px rgba(0,0,0,0.25)",
                h3 { style: "margin:0", "保存导出文件" }
                p { style: "margin:0;font-size:12px;color:#666", "格式 .xlsx，每个交易对象一个 sheet（含自动筛选）。" }
                label { "文件名" }
                input {
                    value: "{name_value}",
                    placeholder: "{fallback}",
                    oninput: move |e| file_name.set(e.value()),
                }
                {controls}
                div { style: "display:flex;gap:8px;justify-content:flex-end",
                    button { onclick: move |_| on_cancel.call(()), "取消" }
                }
            }
        }
    }
}

/// Web：只填文件名，点击后由浏览器下载
#[cfg(target_arch = "wasm32")]
#[component]
fn WebSaveControls(
    file_name: String,
    fallback: String,
    bytes: Vec<u8>,
    on_saved: EventHandler<String>,
) -> Element {
    let st = use_context::<GlobalState>();

    rsx! {
        div { style: "display:flex;gap:8px;align-items:center",
            button {
                onclick: move |_| {
                    let name = ensure_xlsx_suffix(&file_name, &fallback);
                    match save_bytes_wasm(&bytes, &name) {
                        Ok(()) => on_saved.call(name),
                        Err(e) => push_toast(&st, &e),
                    }
                },
                "⬇ 下载"
            }
            span { style: "font-size:12px;color:#888", "（浏览器下载目录由浏览器设置决定）" }
        }
    }
}

/// 桌面：先选目录（可重复选择），再按当前文件名写出
#[cfg(not(target_arch = "wasm32"))]
#[component]
fn NativeSaveControls(
    file_name: String,
    fallback: String,
    bytes: Vec<u8>,
    on_saved: EventHandler<String>,
) -> Element {
    let st = use_context::<GlobalState>();
    // 已选择的目录（空 = 尚未选择，保存时再弹「另存为」）
    let mut chosen_dir = use_signal(String::new);
    let dir_value = (chosen_dir)();
    let has_dir = !dir_value.is_empty();
    // 两个按钮各自捕获一份副本（String 非 Copy，move 闭包会各取一次）
    let pick_name = file_name.clone();
    let pick_fallback = fallback.clone();

    rsx! {
        div { style: "display:flex;gap:8px;align-items:center;flex-wrap:wrap",
            button {
                onclick: move |_| {
                    let name = ensure_xlsx_suffix(&pick_name, &pick_fallback);
                    if let Some(path) = pick_save_path(&name) {
                        // 只记目录：用户随后修改文件名也能生效
                        let dir = path
                            .parent()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default();
                        chosen_dir.set(dir);
                    }
                },
                "📁 选择保存位置…"
            }
            if has_dir {
                span { style: "font-size:12px;color:#666;word-break:break-all", "{dir_value}" }
            }
        }
        button {
            onclick: move |_| {
                let name = ensure_xlsx_suffix(&file_name, &fallback);
                let target = match (chosen_dir)() {
                    dir if !dir.is_empty() => std::path::Path::new(&dir).join(&name),
                    // 未选目录时弹「另存为」；用户取消则中止保存
                    _ => match pick_save_path(&name) {
                        Some(path) => path,
                        None => return,
                    },
                };
                match std::fs::write(&target, &bytes) {
                    Ok(()) => on_saved.call(target.display().to_string()),
                    Err(e) => push_toast(&st, &format!("写入文件失败: {e}")),
                }
            },
            "💾 保存"
        }
    }
}

/// 弹出原生「另存为」对话框，返回用户选择的完整路径（取消返回 None）。
#[cfg(not(target_arch = "wasm32"))]
fn pick_save_path(default_name: &str) -> Option<std::path::PathBuf> {
    rfd::FileDialog::new()
        .set_file_name(default_name)
        .add_filter("Excel 工作簿", &["xlsx"])
        .save_file()
}

/// 浏览器下载：字节 → Blob → `<a download>` 点击 → 释放对象 URL。
///
/// 下载由浏览器接管，因此函数返回即代表已交给浏览器，无需等待落盘。
#[cfg(target_arch = "wasm32")]
fn save_bytes_wasm(bytes: &[u8], file_name: &str) -> Result<(), String> {
    use wasm_bindgen::JsCast;

    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes));
    let blob = web_sys::Blob::new_with_u8_array_sequence(&parts)
        .map_err(|e| format!("创建 Blob 失败: {e:?}"))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)
        .map_err(|e| format!("创建下载链接失败: {e:?}"))?;
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| "无法访问页面文档".to_string())?;
    let anchor = document
        .create_element("a")
        .map_err(|e| format!("创建下载节点失败: {e:?}"))?
        .dyn_into::<web_sys::HtmlAnchorElement>()
        .map_err(|_| "下载节点类型错误".to_string())?;
    anchor.set_href(&url);
    anchor.set_download(file_name);
    anchor.click();
    // 点击后浏览器已接管下载，立即释放对象 URL 避免内存泄漏
    web_sys::Url::revoke_object_url(&url).map_err(|e| format!("释放下载链接失败: {e:?}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Money;

    /// 交易对象（余额与货币不影响导出逻辑）
    fn counterparty(id: i64, name: &str) -> Counterparty {
        Counterparty {
            id,
            name: name.to_string(),
            currency_id: 1,
            balance: Money::from_int(0),
        }
    }

    /// 记录视图：金额固定 12.34，时间固定，便于断言
    fn view(
        from_id: i64,
        to_id: i64,
        from: &str,
        to: &str,
        group_name: Option<&str>,
    ) -> RecordView {
        RecordView {
            id: 1,
            group_id: 9,
            group_name: group_name.map(|s| s.to_string()),
            group_months: None,
            from: from.to_string(),
            from_id,
            to: to.to_string(),
            to_id,
            amount: Money {
                value: 1234,
                divisor: 100,
            },
            currency: "RMB".to_string(),
            remark: Some("午饭".to_string()),
            traded_at: "2026-03-01 12:30:00".to_string(),
        }
    }

    /// 验证方向为「转出」时，对方对象取记录的转入方（to），并保留金额/货币/时间
    #[test]
    fn sheet_should_mark_out_direction_and_other_party() {
        let cp = counterparty(1, "支付宝");
        let views = vec![view(1, 2, "支付宝", "微信钱包", Some("聚餐"))];
        let sheet = sheet_from_views(
            &cp,
            "RMB",
            Some("2026-03".to_string()),
            Some("2026-03".to_string()),
            &views,
        );
        assert_eq!(sheet.counterparty, "支付宝");
        assert_eq!(sheet.currency, "RMB");
        assert_eq!(sheet.month_from.as_deref(), Some("2026-03"));
        assert_eq!(sheet.records.len(), 1);
        let rec = &sheet.records[0];
        assert_eq!(rec.direction, Direction::Out);
        assert_eq!(rec.other_party, "微信钱包");
        assert_eq!(rec.amount.value, 1234);
        assert_eq!(rec.amount.divisor, 100);
        assert_eq!(rec.currency, "RMB");
        assert_eq!(rec.traded_at, "2026-03-01 12:30:00");
        assert_eq!(rec.group.as_deref(), Some("聚餐"));
    }

    /// 验证方向为「转入」时，对方对象取记录的转出方（from）
    #[test]
    fn sheet_should_mark_in_direction_and_other_party() {
        let cp = counterparty(2, "微信钱包");
        let views = vec![view(1, 2, "支付宝", "微信钱包", None)];
        let sheet = sheet_from_views(&cp, "RMB", None, None, &views);
        let rec = &sheet.records[0];
        assert_eq!(rec.direction, Direction::In);
        assert_eq!(rec.other_party, "支付宝");
        // 无组名时回退为 组#ID，与记录页展示一致
        assert_eq!(rec.group.as_deref(), Some("组#9"));
    }

    /// 验证与本对象无关的记录被跳过（不产生空方向或错误对方对象）
    #[test]
    fn sheet_should_skip_records_unrelated_to_counterparty() {
        let cp = counterparty(3, "现金");
        let views = vec![
            view(1, 2, "支付宝", "微信钱包", None),
            view(3, 2, "现金", "微信钱包", None),
        ];
        let sheet = sheet_from_views(&cp, "RMB", None, None, &views);
        assert_eq!(sheet.records.len(), 1, "只应保留与本对象相关的记录");
        assert_eq!(sheet.records[0].direction, Direction::Out);
    }
}
