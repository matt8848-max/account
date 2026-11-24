//! 导出交易记录（xlsx）
//!
//! 流程：确认范围 → 逐个对象拉取记录 → 生成 xlsx → 系统「另存为」→ 写盘。
//! 生成与写盘都在后台线程执行，进度写入 `AccountApp.task`（状态栏显示）。

use super::app::AccountApp;
use super::app_state::{RecordView, TaskProgress};
use super::date;
use super::http;
use super::stats_page::{build_groups, data_range};
use crate::server::export::{
    Direction, ExportCardRow, ExportFlowRow, ExportRecord, ExportSheet, ExtraSheet,
    build_xlsx_with_extras, default_file_name, month_bounds,
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
        let from = date::picker_month(self.stats_from.read(cx));
        let to = date::picker_month(self.stats_to.read(cx));
        let (iso_from, iso_to) = month_bounds(from.as_deref(), to.as_deref());

        // 卡片数据已经在前端统计缓存里（含多取一年的环比/同比基准），
        // 这里直接复用，避免导出时再次计算。
        let mut balances = std::collections::HashMap::new();
        for c in &cps {
            balances.insert(c.id, c.balance);
        }
        let cards: Vec<ExportCardRow> = if self.export_cards {
            let (data_start, data_end) = data_range(&self.stats, &cps);
            build_groups(
                &self.stats,
                &balances,
                from.as_deref(),
                to.as_deref(),
                data_start.as_deref(),
                data_end.as_deref(),
            )
            .iter()
            .flat_map(|g| {
                g.monthly.iter().map(move |row| ExportCardRow {
                    counterparty: g.name.clone(),
                    currency: g.currency.clone(),
                    month: row.month.clone(),
                    income: row.income,
                    income_mom: row.income_mom,
                    income_yoy: row.income_yoy,
                    expense: row.expense,
                    expense_mom: row.expense_mom,
                    expense_yoy: row.expense_yoy,
                    settlement: row.settlement,
                })
            })
            .collect()
        } else {
            Vec::new()
        };
        let want_sankey = self.export_sankey;
        let want_cards = self.export_cards;
        let main_label = ids
            .iter()
            .filter_map(|id| cps.iter().find(|c| c.id == *id).map(|c| c.name.clone()))
            .collect::<Vec<_>>()
            .join("、");

        self.task = Some(TaskProgress::new("导出交易记录", ids.len() + 2));

        cx.spawn(async move |this, cx| {
            let mut sheets: Vec<ExportSheet> = Vec::new();
            let mut flow_map: std::collections::BTreeMap<(String, String), ExportFlowRow> =
                std::collections::BTreeMap::new();
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
                    // 流向图/流向 sheet 与界面保持一致：组内互转（两端都在已选对象内）不计入。
                    if want_sankey && !(ids.contains(&view.from_id) && ids.contains(&view.to_id)) {
                        let key = (other_party.clone(), view.currency.clone());
                        let flow = flow_map.entry(key).or_insert_with(|| ExportFlowRow {
                            main: main_label.clone(),
                            other: other_party.clone(),
                            currency: view.currency.clone(),
                            out_amount: crate::server::models::Money::ZERO,
                            in_amount: crate::server::models::Money::ZERO,
                        });
                        let slot = match direction {
                            Direction::Out => &mut flow.out_amount,
                            Direction::In => &mut flow.in_amount,
                        };
                        *slot = slot.checked_add(view.amount).unwrap_or(*slot);
                    }
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

            let mut flows: Vec<ExportFlowRow> = flow_map.into_values().collect();
            flows.sort_by(|a, b| {
                b.total()
                    .cmp(&a.total())
                    .then_with(|| a.other.cmp(&b.other))
            });
            let mut extras: Vec<ExtraSheet> = Vec::new();
            // 勾选即生成对应 sheet；没有数据时也保留 sheet 并写明原因，
            // 避免用户勾选后完全看不到该页。
            if want_cards {
                extras.push(ExtraSheet::Cards(cards));
            }
            if want_sankey {
                extras.push(ExtraSheet::Sankey(flows));
            }

            let bytes = cx
                .background_spawn(async move { build_xlsx_with_extras(&sheets, &extras) })
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
