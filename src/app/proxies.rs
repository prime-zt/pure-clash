//! 代理页：运行模式切换、策略分组折叠列表与节点选择。

use gpui::{AnyElement, Context, SharedString, Styled, div, px};
use rust_i18n::t;

use super::*;
use crate::assets::{ICON_CHEVRON_DOWN, ICON_CHEVRON_RIGHT, ICON_CIRCLE_CHECK, ICON_GIT_BRANCH};
use crate::mihomo::controller::{GroupSnapshot, NodeSnapshot};
use crate::theme::{FontWeightExt, Palette};

fn group_kind_label(group: &GroupSnapshot) -> &'static str {
    if group.selectable {
        "proxy.kind_selector"
    } else {
        "proxy.kind_auto"
    }
}

pub(super) fn group_auto_expanded(groups: &[GroupSnapshot], name: &str) -> bool {
    let mut budget = PROXY_AUTO_EXPAND_NODE_BUDGET;
    for group in groups {
        if group.nodes.len() > PROXY_AUTO_COLLAPSE_NODES {
            if group.name == name {
                return false;
            }
            continue;
        }
        if group.name == name {
            return group.nodes.len() <= budget;
        }
        budget = budget.saturating_sub(group.nodes.len());
    }
    false
}

pub(super) fn render_proxies(
    app: &PureClash,
    palette: Palette,
    cx: &mut Context<PureClash>,
) -> AnyElement {
    let mode_available = app.mihomo_running();
    div()
        .p_6()
        .child(
            div()
                .p_4()
                .rounded_md()
                .bg(palette.surface)
                .border_1()
                .border_color(palette.border)
                .child(
                    div()
                        .flex()
                        .items_start()
                        .justify_between()
                        .child(section_heading(
                            tr("proxy.mode_title"),
                            tr("proxy.mode_detail"),
                            ICON_GIT_BRANCH,
                            palette,
                        ))
                        .child(
                            div()
                                .id("proxies-refresh")
                                .px_3()
                                .h(px(28.0))
                                .rounded_sm()
                                .flex()
                                .items_center()
                                .map(|button| {
                                    if mode_available {
                                        button.cursor_pointer()
                                    } else {
                                        button.opacity(0.5)
                                    }
                                })
                                .bg(palette.surface_alt)
                                .text_xs()
                                .text_color(palette.muted)
                                .child(tr("proxy.refresh"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if this.mihomo_running() {
                                        this.fetch_runtime_state(cx);
                                    }
                                })),
                        ),
                )
                .child(
                    div().mt_3().flex().gap_2().children(
                        [ProxyMode::Rule, ProxyMode::Global, ProxyMode::Direct]
                            .into_iter()
                            .map(|mode| mode_button(mode, app, mode_available, palette, cx)),
                    ),
                ),
        )
        .children(if app.groups.is_empty() {
            // 内核未运行或尚未拉到分组时展示引导空态。
            vec![
                div()
                    .mt_4()
                    .p_8()
                    .rounded_md()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .bg(palette.surface)
                    .border_1()
                    .border_color(palette.border)
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.text)
                            .child(if app.proxies_loading {
                                tr("proxy.loading")
                            } else {
                                tr("proxy.empty_title")
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.muted)
                            .child(tr("proxy.empty_detail")),
                    )
                    .into_any_element(),
            ]
        } else if app.mode == ProxyMode::Direct {
            // 直连模式不经过任何代理节点，分组列表没有操作意义，展示提示。
            vec![direct_mode_hint(palette)]
        } else {
            app.groups
                .iter()
                .enumerate()
                .map(|(index, group)| {
                    let expanded = app.group_expanded(group);
                    let rendered = app.group_rendered_count(group);
                    proxy_group_panel(index, group, expanded, rendered, app, palette, cx)
                })
                .collect()
        })
        .into_any_element()
}

/// 直连模式提示卡：该模式下请求不经过代理节点，无需选择分组。
fn direct_mode_hint(palette: Palette) -> AnyElement {
    div()
        .mt_4()
        .p_8()
        .rounded_md()
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .bg(palette.surface)
        .border_1()
        .border_color(palette.border)
        .child(
            div()
                .text_sm()
                .text_color(palette.text)
                .child(tr("proxy.direct_title")),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette.muted)
                .child(tr("proxy.direct_detail")),
        )
        .into_any_element()
}

fn mode_button(
    mode: ProxyMode,
    app: &PureClash,
    available: bool,
    palette: Palette,
    cx: &mut Context<PureClash>,
) -> AnyElement {
    let active = mode == app.mode;
    div()
        .id(mode.label())
        .map(|button| {
            if available {
                button.cursor_pointer()
            } else {
                button.opacity(0.5)
            }
        })
        .flex_1()
        .min_h(px(56.0))
        .p_3()
        .rounded_sm()
        .cursor_pointer()
        .bg(if active {
            palette.accent_soft
        } else {
            palette.surface_alt
        })
        .border_1()
        .border_color(if active {
            palette.accent
        } else {
            palette.border
        })
        .child(
            div()
                .text_sm()
                .font_medium()
                .text_color(if active { palette.accent } else { palette.text })
                .child(mode.label()),
        )
        .child(
            div()
                .mt_1()
                .text_xs()
                .text_color(palette.muted)
                .child(mode.detail()),
        )
        .on_click(cx.listener(move |this, _, _, cx| this.set_mode(mode, cx)))
        .into_any_element()
}

fn proxy_group_panel(
    group_index: usize,
    group: &GroupSnapshot,
    expanded: bool,
    rendered: usize,
    app: &PureClash,
    palette: Palette,
    cx: &mut Context<PureClash>,
) -> AnyElement {
    let group_name = group.name.clone();
    let test_name = group.name.clone();
    let more_source = group.name.clone();
    let group_testing = group
        .nodes
        .iter()
        .any(|node| app.delay_testing.contains(&node.name));
    div()
        .p_4()
        .rounded_md()
        .bg(palette.surface)
        .border_1()
        .border_color(palette.border)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .id(SharedString::from(format!("proxy-group-{group_index}")))
                        .flex()
                        .items_center()
                        .gap_2()
                        .cursor_pointer()
                        // 点击标题折叠/展开节点列表，避免大分组常驻布局。
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_group_expanded(group_name.clone(), cx)
                        }))
                        .child(
                            div()
                                .text_base()
                                .font_semibold()
                                .text_color(palette.text)
                                .child(group.name.clone()),
                        )
                        .child(
                            div()
                                .px_2()
                                .py(px(2.0))
                                .rounded_sm()
                                .bg(palette.surface_alt)
                                .text_xs()
                                .text_color(palette.muted)
                                .child(tr(group_kind_label(group))),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        // 整组测速：一次请求更新组内全部节点延迟。
                        .child(
                            div()
                                .id(SharedString::from(format!("proxy-test-{group_index}")))
                                .px_2()
                                .py(px(3.0))
                                .rounded_sm()
                                .flex()
                                .items_center()
                                .cursor_pointer()
                                .bg(palette.surface_alt)
                                .text_xs()
                                .text_color(if group_testing {
                                    palette.accent
                                } else {
                                    palette.muted
                                })
                                .child(if group_testing {
                                    tr("proxy.testing")
                                } else {
                                    tr("proxy.test")
                                })
                                .on_click({
                                    let name = test_name.clone();
                                    cx.listener(move |this, _, _, cx| {
                                        this.test_group_delay(name.clone(), cx);
                                    })
                                }),
                        )
                        .children((!expanded && !group.now.is_empty()).then(|| {
                            div()
                                .max_w(px(240.0))
                                .truncate()
                                .text_xs()
                                .text_color(palette.muted)
                                .child(group.now.clone())
                        }))
                        .child(div().text_xs().text_color(palette.muted).child(
                            t!("proxy.nodes", count = group.nodes.len().to_string()).into_owned(),
                        ))
                        .child(icon(
                            if expanded {
                                ICON_CHEVRON_DOWN
                            } else {
                                ICON_CHEVRON_RIGHT
                            },
                            palette.muted,
                            14.0,
                        )),
                ),
        )
        // 折叠时不渲染节点行；展开时按页渲染普通网格，剩余节点通过
        // “显示更多”加载，单次布局量有硬上界，滚动只保留页面一层。
        .children(expanded.then(|| {
            let rendered = rendered.min(group.nodes.len());
            let rows = rendered.div_ceil(PROXY_NODE_COLUMNS);
            let mut list = div()
                .mt_3()
                .flex()
                .flex_col()
                .gap_2()
                .children((0..rows).map(|row| {
                    let start = row * PROXY_NODE_COLUMNS;
                    let end = (start + PROXY_NODE_COLUMNS).min(rendered);
                    div()
                        .flex()
                        .gap_2()
                        .children(group.nodes[start..end].iter().enumerate().map(
                            |(column, node)| {
                                proxy_node_row(
                                    group_index,
                                    start + column,
                                    node,
                                    group.now.as_str(),
                                    app,
                                    palette,
                                    cx,
                                )
                            },
                        ))
                }));
            if rendered < group.nodes.len() {
                let more_name = more_source.clone();
                let remaining = group.nodes.len() - rendered;
                list = list.child(
                    div()
                        .id(SharedString::from(format!("proxy-more-{group_index}")))
                        .mt_1()
                        .h(px(32.0))
                        .rounded_sm()
                        .bg(palette.surface_alt)
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .text_xs()
                        .text_color(palette.muted)
                        .child(t!("proxy.show_more", count = remaining.to_string()).into_owned())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.show_more_nodes(more_name.clone(), cx)
                        })),
                );
            }
            list
        }))
        .into_any_element()
}

fn proxy_node_row(
    group_index: usize,
    node_index: usize,
    node: &NodeSnapshot,
    selected_name: &str,
    app: &PureClash,
    palette: Palette,
    cx: &mut Context<PureClash>,
) -> AnyElement {
    let selected = node.name == selected_name;
    div()
        .id(SharedString::from(format!(
            "proxy-node-{group_index}-{node_index}"
        )))
        .flex_1()
        .min_h(px(48.0))
        .px_3()
        .rounded_sm()
        .flex()
        .items_center()
        .gap_3()
        .cursor_pointer()
        .bg(if selected {
            palette.accent_soft
        } else {
            palette.surface
        })
        .border_1()
        .border_color(if selected {
            palette.accent
        } else {
            palette.border
        })
        .child(div().size_2().rounded_full().bg(if selected {
            palette.success
        } else {
            palette.border
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_sm()
                        .font_medium()
                        .text_color(palette.text)
                        .child(node.name.clone()),
                )
                .child(
                    div()
                        .mt_1()
                        .truncate()
                        .text_xs()
                        .text_color(palette.muted)
                        .child(node.kind.to_lowercase()),
                ),
        )
        .children(node_delay_badge(app, node, palette, cx))
        .when(selected, |row| {
            row.child(icon(ICON_CIRCLE_CHECK, palette.success, 16.0))
        })
        .on_click(cx.listener(move |this, _, _, cx| this.select_node(group_index, node_index, cx)))
        .into_any_element()
}

/// 节点延迟徽标：测速中显示省略号，有结果按延迟分色，点击单独重测该节点。
/// 无数据时不渲染，保持未测速节点的行高稳定。
fn node_delay_badge(
    app: &PureClash,
    node: &NodeSnapshot,
    palette: Palette,
    cx: &mut Context<PureClash>,
) -> Option<AnyElement> {
    let testing = app.delay_testing.contains(&node.name);
    let (label, color) = if testing {
        (tr("proxy.testing"), palette.muted)
    } else {
        match app.node_delay(node) {
            None => return None,
            Some(None) => (tr("proxy.timeout"), rgb(0xd15b5b)),
            Some(Some(delay)) => {
                let label = SharedString::from(format!("{delay} ms"));
                // 常见面板配色：<200 优秀，<500 可用，更高偏红提示。
                let color = if delay < 200 {
                    palette.success
                } else if delay < 500 {
                    palette.text
                } else {
                    rgb(0xd15b5b)
                };
                (label, color)
            }
        }
    };
    let node_name = node.name.clone();
    Some(
        div()
            .id(SharedString::from(format!("proxy-delay-{}", node.name)))
            .px_2()
            .py(px(2.0))
            .rounded_sm()
            .flex_none()
            .cursor_pointer()
            .bg(palette.surface_alt)
            .text_xs()
            .text_color(color)
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                // 点击徽标只重测该节点，不触发整行的节点选择。
                cx.stop_propagation();
                this.test_node_delay(node_name.clone(), cx);
            }))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proxy_group(name: &str, node_count: usize) -> GroupSnapshot {
        GroupSnapshot {
            name: name.to_string(),
            now: String::new(),
            selectable: true,
            nodes: (0..node_count)
                .map(|index| NodeSnapshot {
                    name: format!("节点{index}"),
                    kind: "vless".into(),
                    delay: None,
                })
                .collect(),
        }
    }

    #[test]

    fn groups_follow_config_order_with_global_first() {
        let mut groups = vec![
            proxy_group("美国", 2),
            proxy_group("GLOBAL", 3),
            proxy_group("自动选择", 1),
            proxy_group("香港", 2),
        ];
        let order = vec![
            "香港".to_string(),
            "美国".to_string(),
            "自动选择".to_string(),
        ];
        order_groups(&mut groups, &order);

        let names: Vec<&str> = groups.iter().map(|group| group.name.as_str()).collect();
        // GLOBAL 置顶，其余严格按配置定义顺序。
        assert_eq!(names, vec!["GLOBAL", "香港", "美国", "自动选择"]);

        // 顺序表缺失（解析失败兜底）时保持快照原序，只是 GLOBAL 仍在首位。
        let mut fallback = vec![proxy_group("美国", 1), proxy_group("GLOBAL", 1)];
        order_groups(&mut fallback, &[]);
        let names: Vec<&str> = fallback.iter().map(|group| group.name.as_str()).collect();
        assert_eq!(names, vec!["GLOBAL", "美国"]);
    }

    #[test]
    fn oversized_group_defaults_collapsed() {
        let groups = vec![proxy_group("大分组", 200), proxy_group("小分组", 5)];
        assert!(!group_auto_expanded(&groups, "大分组"));
        assert!(group_auto_expanded(&groups, "小分组"));
        // 不在列表中的组名按折叠处理。
        assert!(!group_auto_expanded(&groups, "未知分组"));
    }

    #[test]
    fn auto_expand_stops_at_node_budget() {
        // 8 个 20 节点的组，预算 120 只够前 6 个自动展开。
        let groups: Vec<GroupSnapshot> = (0..8)
            .map(|index| proxy_group(&format!("g{index}"), 20))
            .collect();
        assert!(group_auto_expanded(&groups, "g0"));
        assert!(group_auto_expanded(&groups, "g5"));
        assert!(!group_auto_expanded(&groups, "g6"));
        assert!(!group_auto_expanded(&groups, "g7"));
    }
}
