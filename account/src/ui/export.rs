//! 导出交易记录（xlsx）
//!
//! 流程：确认范围 → 逐个对象拉取记录 → 生成 xlsx → 系统「另存为」→ 写盘。
//! 生成与写盘都在后台线程执行，进度写入 `AccountApp.task`（状态栏显示）。

use super::app::AccountApp;
use super::http;
use super::state::{RecordView, TaskProgress};
use crate::server::export::{
    Direction, ExportRecord, ExportSheet, build_xlsx, default_file_name, month_bounds,
};
use gpui_kit::AppContext as _;
use gpui_kit::Context;
use serde_json::json;

impl AccountApp {
    /// 启动导出任务（由「确认导出范围」弹窗确认后调用）。
    pub fn start_export(&mut self, cx: &mut Context<Self>) {
        if self.task.is_some() {
            self.push_toast(cx, "已有导出任务在进行中");
            return;
        }
        let ids = self.stats_cp.clone();
        if ids.is_empty() {
            self.push_toast(cx, "请至少选择一个交易对象");
            return;
        }
        let base = self.base(cx);
        let version = self.http_version;
        let cps = self.counterparties.clone();
        let currencies = self.currencies.clone();
        let from = {
            let v = self.stats_from.read(cx).value().trim().to_string();
            if v.is_empty() { None } else { Some(v) }
        };
        let to = {
            let v = self.stats_to.read(cx).value().trim().to_string();
            if v.is_empty() { None } else { Some(v) }
        };
        let (iso_from, iso_to) = month_bounds(from.as_deref(), to.as_deref());

        self.task = Some(TaskProgress::new("导出交易记录", ids.len() + 2));

        cx.spawn(async move |this, cx| {
            let mut sheets: Vec<ExportSheet> = Vec::new();
            for id in ids.iter().copied() {
                let Some(cp) = cps.iter().find(|c| c.id == id).cloned() else {
                    continue;
                };
                let currency = currencies
                    .iter()
                    .find(|c| c.id == cp.currency_id)
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| format!("id={}", cp.currency_id));
                let body = json!({
                    "counterparty_id": id,
                    "from": iso_from,
                    "to": iso_to,
                });
                let request_base = base.clone();
                let request = cx.background_spawn(async move {
                    http::put(&request_base, "/api/records/object", Some(body), version)
                });
                let result = request.await;
                let views: Vec<RecordView> = match result {
                    Ok(v) => serde_json::from_value(v).unwrap_or_default(),
                    Err(e) => {
                        this.update(cx, |this, cx| {
                            this.task = None;
                            this.push_toast(cx, format!("导出失败: {e}"));
                            cx.notify();
                        })
                        .ok();
                        return;
                    }
                };
                let mut records = Vec::new();
                for view in views {
                    let Some(direction) = Direction::of(view.from_id, view.to_id, id) else {
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
                sheets.push(ExportSheet {
                    counterparty: cp.name,
                    currency,
                    month_from: from.clone(),
                    month_to: to.clone(),
                    records,
                });
                this.update(cx, |this, cx| {
                    if let Some(task) = &mut this.task {
                        task.advance();
                    }
                    cx.notify();
                })
                .ok();
            }

            let bytes = cx
                .background_spawn(async move { build_xlsx(&sheets) })
                .await;
            let bytes = match bytes {
                Ok(bytes) => bytes,
                Err(e) => {
                    this.update(cx, |this, cx| {
                        this.task = None;
                        this.push_toast(cx, format!("生成 xlsx 失败: {e}"));
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            let default_name = default_file_name(from.as_deref(), to.as_deref());
            let saved = cx
                .background_spawn(async move {
                    let path = rfd::FileDialog::new()
                        .set_file_name(&default_name)
                        .save_file();
                    let Some(path) = path else {
                        return Ok::<_, String>(None);
                    };
                    std::fs::write(&path, &bytes).map_err(|e| format!("写入文件失败: {e}"))?;
                    Ok(Some(path.display().to_string()))
                })
                .await;
            this.update(cx, |this, cx| {
                this.task = None;
                match saved {
                    Ok(Some(path)) => this.push_toast(cx, format!("导出完成: {path}")),
                    Ok(None) => this.push_toast(cx, "已取消导出"),
                    Err(e) => this.push_toast(cx, e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
