//! 桑基图 / 流向图（纯 `div` 绘制）
//!
//! 本项目不引入第三方绘图库：每条流向用一根按金额比例设置高度的色带表示。
//! 布局为「三栏」：左侧是净流入主要对象的次要对象，中间是主要对象组，右侧是净流出对象。
//! 双向往来先在 [`FlowView::net`] 中合并为净额，再按净额方向放到左侧或右侧，
//! 因此同一个对象不会同时出现在流入和流出两侧。
//!
//! 布局计算与渲染分离：`amount_heights` / `inflow_indices` / `outflow_indices`
//! 是纯函数（可单元测试），`sankey_view` 只负责读取主题色并拼装元素。

use super::app_state::FlowView;
use crate::server::models::Money;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{App, IntoElement, ParentElement as _, Styled as _, div, px};

/// 单条流向的最小高度（px）：保证三行文字可读。
const MIN_FLOW_HEIGHT: f32 = 56.0;
/// 相邻流向之间的间距（px）。
const FLOW_GAP: f32 = 6.0;
/// 每一侧的图表高度预算（px）：超出时页面整体滚动。
const DIAGRAM_HEIGHT: f32 = 520.0;
/// 主要对象组节点宽度（px）。
const MAIN_NODE_WIDTH: f32 = 160.0;
/// 次要对象节点宽度（px）。
const OTHER_NODE_WIDTH: f32 = 240.0;

/// 双向往来合并后的净流量绝对值。
///
/// `FlowView` 已按（次要对象, 货币）聚合了转入/转出；这里用
/// `转入 − 转出` 的绝对值作为条带宽度，净流入/净流出由符号决定。
pub fn net_amount(flow: &FlowView) -> Money {
    let net = flow.net();
    if net.value < 0 {
        net.checked_neg().unwrap_or(net)
    } else {
        net
    }
}

/// 净流入主要对象的流向索引（按净额排序后传入的 `flows` 顺序）。
pub fn inflow_indices(flows: &[FlowView]) -> Vec<usize> {
    flows
        .iter()
        .enumerate()
        .filter(|(_, flow)| flow.net().value > 0)
        .map(|(idx, _)| idx)
        .collect()
}

/// 净流出主要对象的流向索引。
pub fn outflow_indices(flows: &[FlowView]) -> Vec<usize> {
    flows
        .iter()
        .enumerate()
        .filter(|(_, flow)| flow.net().value < 0)
        .map(|(idx, _)| idx)
        .collect()
}

/// 按金额比例分配每条流向的高度。
///
/// - 空输入返回空 `Vec`；
/// - 每条至少 [`MIN_FLOW_HEIGHT`]；当最小高度总和超过预算时整体变高，
///   由外层页面滚动承载，不会压缩到不可读；
/// - 金额全部为 0 时等分最小高度。
pub fn amount_heights(amounts: &[Money], budget: f32) -> Vec<f32> {
    if amounts.is_empty() {
        return Vec::new();
    }
    let weights: Vec<f64> = amounts
        .iter()
        .map(|amount| amount.value as f64 / amount.divisor as f64)
        .collect();
    let sum: f64 = weights.iter().sum();
    let gaps = FLOW_GAP * (amounts.len() as f32 - 1.0);
    let available = (budget - gaps).max(0.0);
    if sum <= 0.0 || !sum.is_finite() {
        return vec![MIN_FLOW_HEIGHT; amounts.len()];
    }
    let scale = available as f64 / sum;
    weights
        .iter()
        .map(|w| ((w * scale) as f32).max(MIN_FLOW_HEIGHT))
        .collect()
}

/// 渲染流向图。
///
/// 左侧：净流入主要对象的次要对象（指向主要对象组）；
/// 中间：主要对象组；
/// 右侧：净流出主要对象的次要对象（由主要对象组指出）。
pub fn sankey_view(flows: &[FlowView], group_label: &str, cx: &App) -> gpui_kit::AnyElement {
    let palette = [
        cx.theme().chart_1,
        cx.theme().chart_2,
        cx.theme().chart_3,
        cx.theme().chart_4,
        cx.theme().chart_5,
    ];
    let primary = cx.theme().primary;
    let muted = cx.theme().muted_foreground;

    let inflows = inflow_indices(flows);
    let outflows = outflow_indices(flows);
    if inflows.is_empty() && outflows.is_empty() {
        return v_flex()
            .w_full()
            .gap_1()
            .pt_2()
            .child(div().text_lg().child("资金流向（桑基图）"))
            .child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child("所选范围内双向往来互相抵消，没有净流向"),
            )
            .into_any_element();
    }

    let inflow_amounts: Vec<Money> = inflows.iter().map(|&idx| net_amount(&flows[idx])).collect();
    let outflow_amounts: Vec<Money> = outflows
        .iter()
        .map(|&idx| net_amount(&flows[idx]))
        .collect();
    let inflow_heights = amount_heights(&inflow_amounts, DIAGRAM_HEIGHT);
    let outflow_heights = amount_heights(&outflow_amounts, DIAGRAM_HEIGHT);
    let column_height = |heights: &[f32]| -> f32 {
        heights.iter().sum::<f32>() + FLOW_GAP * (heights.len().saturating_sub(1)) as f32
    };
    let main_height = column_height(&inflow_heights)
        .max(column_height(&outflow_heights))
        .max(64.);

    // 次要对象节点：显示名称、双向原始金额与合并后的净额。
    let node = |flow: &FlowView, color: gpui_kit::Hsla, inflow: bool| {
        let title = if inflow {
            format!("净流入 {}", flow.net())
        } else {
            format!("净流出 {}", net_amount(flow))
        };
        v_flex()
            .w(px(OTHER_NODE_WIDTH))
            .h_full()
            .flex_shrink_0()
            .justify_center()
            .gap_0p5()
            .px_2()
            .border_1()
            .border_color(color)
            .rounded(px(4.))
            .child(
                div()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child(format!("{} ({})", flow.name, flow.currency)),
            )
            .child(div().text_sm().text_color(muted).child(format!(
                "转入 {} · 转出 {}",
                flow.in_amount, flow.out_amount
            )))
            .child(div().text_sm().text_color(color).child(title))
    };

    let left_rows: Vec<gpui_kit::AnyElement> = inflows
        .iter()
        .zip(inflow_heights)
        .map(|(&idx, height)| {
            let flow = &flows[idx];
            let color = palette[idx % palette.len()];
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .h(px(height))
                .child(node(flow, color, true))
                .child(
                    div()
                        .flex_1()
                        .h_full()
                        .bg(color.opacity(0.55))
                        .rounded(px(4.)),
                )
                .into_any_element()
        })
        .collect();
    let right_rows: Vec<gpui_kit::AnyElement> = outflows
        .iter()
        .zip(outflow_heights)
        .map(|(&idx, height)| {
            let flow = &flows[idx];
            let color = palette[idx % palette.len()];
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .h(px(height))
                .child(
                    div()
                        .flex_1()
                        .h_full()
                        .bg(color.opacity(0.55))
                        .rounded(px(4.)),
                )
                .child(node(flow, color, false))
                .into_any_element()
        })
        .collect();

    let main_node = v_flex()
        .w(px(MAIN_NODE_WIDTH))
        .h(px(main_height))
        .flex_shrink_0()
        .justify_center()
        .items_center()
        .gap_1()
        .p_2()
        .border_1()
        .border_color(primary)
        .rounded(px(6.))
        .bg(primary.opacity(0.10))
        .child(div().text_sm().text_color(muted).child("主要对象组"))
        .child(
            div()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .child(group_label.to_string()),
        );

    let mut columns: Vec<gpui_kit::AnyElement> = Vec::new();
    if !left_rows.is_empty() {
        columns.push(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(FLOW_GAP))
                .children(left_rows)
                .into_any_element(),
        );
    }
    columns.push(main_node.into_any_element());
    if !right_rows.is_empty() {
        columns.push(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(FLOW_GAP))
                .children(right_rows)
                .into_any_element(),
        );
    }

    v_flex()
        .w_full()
        .gap_2()
        .pt_2()
        .child(
            h_flex()
                .w_full()
                .justify_between()
                .items_center()
                .child(
                    div()
                        .text_lg()
                        .child("资金流向（桑基图 · 左流入 / 右流出）"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child(format!("主要对象组：{group_label}")),
                ),
        )
        .child(h_flex().w_full().gap_3().items_center().children(columns))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::models::Money;

    fn flow(in_amount: i128, out_amount: i128) -> FlowView {
        FlowView {
            counterparty_id: 1,
            name: "商店".into(),
            currency: "RMB".into(),
            out_amount: Money::from(out_amount),
            in_amount: Money::from(in_amount),
        }
    }

    fn amounts(values: &[i128]) -> Vec<Money> {
        values.iter().map(|value| Money::from(*value)).collect()
    }

    /// 验证双向往来合并：按净额只归入流入或流出的一侧，净额为 0 时两侧都不出现。
    #[test]
    fn bidirectional_flows_should_merge_into_one_side() {
        let flows = vec![
            flow(30, 12), // 净流入 18
            flow(12, 30), // 净流出 18
            flow(20, 20), // 相互抵消
            flow(50, 0),  // 净流入
        ];
        assert_eq!(inflow_indices(&flows), vec![0, 3]);
        assert_eq!(outflow_indices(&flows), vec![1]);
        assert_eq!(net_amount(&flows[0]), Money::from(18));
        assert_eq!(net_amount(&flows[1]), Money::from(18));
        assert_eq!(net_amount(&flows[2]), Money::ZERO);
        // 不相等断言：同一笔流向不能同时出现在流入和流出两侧
        assert!(!inflow_indices(&flows).contains(&1));
        assert!(!outflow_indices(&flows).contains(&0));
        assert_ne!(inflow_indices(&flows), outflow_indices(&flows));
    }

    /// 验证高度分配：空输入为空；金额大的条带更高；每条不低于最小高度。
    #[test]
    fn amount_heights_should_be_proportional_and_readable() {
        assert!(amount_heights(&[], 400.0).is_empty());

        let heights = amount_heights(&amounts(&[300, 100]), 406.0);
        assert_eq!(heights.len(), 2);
        assert!(heights[0] > heights[1]);
        assert!(heights[1] >= MIN_FLOW_HEIGHT);
        // 不相等断言：比例不能反过来
        assert_ne!(heights[0], heights[1]);
    }

    /// 验证极端情况：金额全为 0 时等分最小高度，零预算也不会 panic。
    #[test]
    fn amount_heights_should_handle_zero_amounts_and_budget() {
        let heights = amount_heights(&amounts(&[0, 0]), 0.0);
        assert_eq!(heights, vec![MIN_FLOW_HEIGHT, MIN_FLOW_HEIGHT]);
        assert_ne!(heights[0], 0.0);
    }
}
