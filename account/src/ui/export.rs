//! 导出：保存目标选择 + 后台导出任务
//!
//! xlsx 的生成在 `crate::export`（平台无关），本模块负责：
//! - **选择保存目标**：wasm 只填文件名（浏览器下载）；native 用「另存为」对话框选路径
//! - **后台执行**：逐个对象拉记录 → 生成 xlsx → 写盘/下载，进度写入 `GlobalState.task`，
//!   由底部状态栏展示
//!
//! 性能与稳定性说明（见任务清单「导出交易记录闪退」）：
//! - 桌面端使用 `rfd::AsyncFileDialog`（内部在工作线程弹窗），不再在 UI 事件回调里
//!   调用阻塞版 `FileDialog::save_file()`——那会卡住 webview 事件循环，快速连点还会重入；
//! - xlsx 生成与写盘走 `run_blocking`（native 为 `tokio::task::spawn_blocking`），
//!   大文件也不会冻结界面。

use super::{GlobalState, RecordView, TaskProgress, http_put, push_toast, run_blocking};
use crate::export::{
    Direction, ExportRecord, ExportSheet, build_xlsx, default_file_name, ensure_xlsx_suffix,
    month_bounds,
};
use crate::models::{Counterparty, Currency};
use dioxus::prelude::*;

/// 导出任务参数（范围确认后由统计页构造）
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExportJob {
    /// 全部交易对象（用于解析名称与货币）
    pub counterparties: Vec<Counterparty>,
    /// 全部货币（用于解析货币名称）
    pub currencies: Vec<Currency>,
    /// 选中的交易对象 ID（顺序即导出顺序）
    pub ids: Vec<i64>,
    /// 起始月份 `YYYY-MM`；None 表示不限
    pub from: Option<String>,
    /// 结束月份 `YYYY-MM`；None 表示不限
    pub to: Option<String>,
}

impl ExportJob {
    /// 默认文件名（`account_export_2026-01_2026-03.xlsx`）
    pub fn default_name(&self) -> String {
        default_file_name(self.from.as_deref(), self.to.as_deref())
    }
}

/// 保存目标：每个平台只有一种形态
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExportTarget {
    /// 桌面：用户选择的完整路径
    #[cfg(not(target_arch = "wasm32"))]
    Path(String),
    /// Web：交给浏览器的下载文件名
    #[cfg(target_arch = "wasm32")]
    FileName(String),
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

/// 导出保存对话框：确认文件名（桌面端还确认保存位置）后回调 `on_confirm`。
///
/// - `default_name`：默认文件名（含 `.xlsx`）
/// - `on_confirm`：用户确认保存目标（桌面为完整路径，Web 为下载文件名）
/// - `on_cancel`：取消回调
///
/// 对话框本身不生成/写文件：真正的导出在 `spawn_export` 里异步执行，避免阻塞 UI。
#[component]
pub(crate) fn ExportSaveDialog(
    default_name: String,
    on_confirm: EventHandler<ExportTarget>,
    on_cancel: EventHandler<()>,
) -> Element {
    let mut file_name = use_signal(|| default_name.clone());
    let name_value = (file_name)();
    let fallback = default_name.clone();

    // 平台差异：Web 只有「下载」，桌面还有「选择保存位置」
    #[cfg(target_arch = "wasm32")]
    let controls: Element = rsx! {
        WebSaveControls {
            file_name: name_value.clone(),
            fallback: fallback.clone(),
            on_confirm: move |target: ExportTarget| on_confirm.call(target),
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    let controls: Element = rsx! {
        NativeSaveControls {
            file_name: name_value.clone(),
            fallback: fallback.clone(),
            on_confirm: move |target: ExportTarget| on_confirm.call(target),
        }
    };

    rsx! {
        div {
            style: "position:fixed;inset:0;background:rgba(0,0,0,0.35);display:flex;align-items:center;justify-content:center;z-index:1600",
            div {
                style: "background:#fff;border-radius:8px;padding:18px;min-width:360px;max-width:92vw;display:flex;flex-direction:column;gap:10px;box-shadow:0 4px 16px rgba(0,0,0,0.25)",
                h3 { style: "margin:0", "导出交易记录" }
                p { style: "margin:0;font-size:12px;color:#666", "格式 .xlsx，每个交易对象一个 sheet（含自动筛选）。导出在后台进行，进度显示在底部状态栏。" }
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

/// Web：只填文件名，确认后由浏览器下载
#[cfg(target_arch = "wasm32")]
#[component]
fn WebSaveControls(
    file_name: String,
    fallback: String,
    on_confirm: EventHandler<ExportTarget>,
) -> Element {
    rsx! {
        div { style: "display:flex;gap:8px;align-items:center",
            button {
                onclick: move |_| {
                    let name = ensure_xlsx_suffix(&file_name, &fallback);
                    on_confirm.call(ExportTarget::FileName(name));
                },
                "⬇ 导出"
            }
            span { style: "font-size:12px;color:#888", "（浏览器下载目录由浏览器设置决定）" }
        }
    }
}
/// 桌面：选择保存位置（异步对话框，不阻塞 UI），确认后回传完整路径
#[cfg(not(target_arch = "wasm32"))]
#[component]
fn NativeSaveControls(
    file_name: String,
    fallback: String,
    on_confirm: EventHandler<ExportTarget>,
) -> Element {
    // 已选择的目录（空 = 尚未选择，导出时再弹「另存为」）
    let chosen_dir = use_signal(String::new);
    // 对话框打开中：防止连点重入（旧实现里同步阻塞对话框被反复调用会卡死/闪退）
    let picking = use_signal(|| false);
    let dir_value = (chosen_dir)();
    let picking_now = (picking)();
    // 两个按钮各自捕获一份副本（String 非 Copy，move 闭包会各取一次）
    let pick_name = file_name.clone();
    let pick_fallback = fallback.clone();

    rsx! {
        div { style: "display:flex;gap:8px;align-items:center;flex-wrap:wrap",
            button {
                disabled: picking_now,
                onclick: move |_| {
                    if (picking)() {
                        return;
                    }
                    let mut picking = picking;
                    let mut chosen_dir = chosen_dir;
                    let name = ensure_xlsx_suffix(&pick_name, &pick_fallback);
                    picking.set(true);
                    spawn(async move {
                        // 只记目录：用户随后修改文件名也能生效
                        if let Some(path) = pick_save_path(&name).await {
                            let dir = path
                                .parent()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default();
                            chosen_dir.set(dir);
                        }
                        picking.set(false);
                    });
                },
                "📁 选择保存位置…"
            }
            if !dir_value.is_empty() {
                span { style: "font-size:12px;color:#666;word-break:break-all", "{dir_value}" }
            }
        }
        button {
            disabled: picking_now,
            onclick: move |_| {
                if (picking)() {
                    return;
                }
                let mut picking = picking;
                let name = ensure_xlsx_suffix(&file_name, &fallback);
                let dir = (chosen_dir)();
                picking.set(true);
                spawn(async move {
                    let target = if dir.is_empty() {
                        // 未选目录时弹「另存为」；用户取消则中止导出
                        pick_save_path(&name).await
                    } else {
                        Some(std::path::Path::new(&dir).join(&name))
                    };
                    picking.set(false);
                    if let Some(path) = target {
                        on_confirm.call(ExportTarget::Path(path.display().to_string()));
                    }
                });
            },
            "💾 导出"
        }
    }
}

/// 异步弹出「另存为」对话框，返回用户选择的完整路径（取消返回 None）。
///
/// 用 `AsyncFileDialog` 而不是阻塞版 `FileDialog`：rfd 会把对话框放到工作线程，
/// 不占用 webview 事件循环，配合调用方的 `picking` 标记杜绝重入。
#[cfg(not(target_arch = "wasm32"))]
async fn pick_save_path(default_name: &str) -> Option<std::path::PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_file_name(default_name)
        .add_filter("Excel 工作簿", &["xlsx"])
        .save_file()
        .await
        .map(|handle| handle.path().to_path_buf())
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

// ---------- 后台导出任务 ----------

/// 进度加一步（总步数不变）
fn advance(st: &mut GlobalState) {
    if let Some(mut task) = (st.task)() {
        task.advance();
        st.task.set(Some(task));
    }
}

/// 启动后台导出：进度写入 `GlobalState.task`（底部状态栏展示），结束后自动清空。
///
/// 已有任务在跑时直接返回（导出按钮已按 `task` 禁用，这里再兜一层防重入）。
pub(crate) fn spawn_export(mut st: GlobalState, job: ExportJob, target: ExportTarget) {
    if (st.task)().is_some() {
        return;
    }
    // 总步数 = 每个对象拉取一次 + 生成一次 + 写入一次
    let total = job.ids.len() + 2;
    st.task.set(Some(TaskProgress::new("导出交易记录", total)));
    spawn(async move {
        let result = run_export(&mut st, &job, &target).await;
        st.task.set(None);
        match result {
            Ok(msg) => push_toast(&st, &msg),
            Err(e) => push_toast(&st, &format!("导出失败: {e}")),
        }
    });
}

/// 执行导出：拉记录 → 生成 xlsx → 写盘/下载；成功返回提示文案。
///
/// 逐对象拉取是网络 IO（await 不阻塞 UI），xlsx 生成与写盘是 CPU/磁盘密集操作，
/// 统一走 `run_blocking`，因此大数据量也不会冻结界面。
async fn run_export(
    st: &mut GlobalState,
    job: &ExportJob,
    target: &ExportTarget,
) -> Result<String, String> {
    // 月份区间 → 接口时间边界（起始月 1 号 0 点 / 结束月最后一天 23:59:59）
    let (from_bound, to_bound) = month_bounds(job.from.as_deref(), job.to.as_deref());
    let base = (st.base)();
    let mut sheets: Vec<ExportSheet> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();

    for id in &job.ids {
        let Some(cp) = job.counterparties.iter().find(|c| c.id == *id) else {
            advance(st);
            continue;
        };
        let currency = job
            .currencies
            .iter()
            .find(|c| c.id == cp.currency_id)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        let body = serde_json::json!({
            "from": from_bound.clone(),
            "to": to_bound.clone(),
            "counterparty_id": id,
        });
        // 后端 /api/records 按 counterparty_id 过滤（from 或 to 命中）
        let views: Vec<RecordView> = http_put(&base, "/api/records", Some(body))
            .await
            .map(|v| serde_json::from_value(v).unwrap_or_default())
            .map_err(|e| format!("拉取 {} 记录失败: {e}", cp.name))?;
        let sheet = sheet_from_views(cp, &currency, job.from.clone(), job.to.clone(), &views);
        if sheet.records.is_empty() {
            skipped.push(cp.name.clone());
        } else {
            sheets.push(sheet);
        }
        advance(st);
    }

    if sheets.is_empty() {
        return Err("所选对象在该时间区间内没有交易记录".to_string());
    }

    // 生成 xlsx：CPU 密集，放阻塞线程池执行
    let generate = sheets.clone();
    let bytes = run_blocking(move || build_xlsx(&generate))
        .await
        .map_err(|e| format!("生成 xlsx 失败: {e}"))??;
    advance(st);

    // 写到用户指定位置 / 交给浏览器下载
    let saved = match target {
        #[cfg(not(target_arch = "wasm32"))]
        ExportTarget::Path(path) => {
            let target_path = path.clone();
            let data = bytes;
            run_blocking(move || std::fs::write(&target_path, &data))
                .await
                .map_err(|e| format!("写入文件失败: {e}"))?
                .map_err(|e| format!("写入文件失败: {e}"))?;
            path.clone()
        }
        #[cfg(target_arch = "wasm32")]
        ExportTarget::FileName(name) => {
            save_bytes_wasm(&bytes, name)?;
            name.clone()
        }
    };
    advance(st);

    let mut msg = format!("导出完成: {saved}");
    if !skipped.is_empty() {
        msg = format!(
            "导出完成: {saved}（区间内无记录已跳过：{}）",
            skipped.join("、")
        );
    }
    Ok(msg)
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
