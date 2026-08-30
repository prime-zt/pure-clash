//! 概览页：统计卡片、当前出站摘要、运行状态卡与最近连接。

use gpui::{AnyElement, Context, SharedString, Styled, div, px};

use super::connections::connection_row;
use super::*;
use crate::assets::{ICON_CHART, ICON_GIT_BRANCH, ICON_LAYOUT, ICON_MESSAGE, ICON_TERMINAL};
use crate::theme::{FontWeightExt, Palette};

pub(super) fn render_overview(
    app: &PureClash,
    palette: Palette,
    cx: &mut Context<PureClash>,
) -> AnyElement {
    div()
        .p_6()
        .when_some(app.integration_error.as_ref(), |page, error| {
            page.child(integration_error_banner(error, palette))
        })
        .child(render_stat_row(app, palette))
        .child(
            div()
                .mt_4()
                .flex()
                .gap_4()
                .child(render_overview_outbound(app, palette, cx))
                .child(render_runtime_card(app, palette, cx)),
        )
        .child(render_recent_connections(app, palette, cx))
        .into_any_element()
}

fn render_stat_row(app: &PureClash, palette: Palette) -> AnyElement {
    let running = app.mihomo_running();
    div()
        .flex()
        .gap_3()
        .child(stat_card(
            tr("overview.current_mode"),
            app.mode.label(),
            app.mode.detail(),
            ICON_GIT_BRANCH,
            palette,
        ))
        .child(stat_card(
            tr("overview.proxy_groups"),
            app.groups.len().to_string(),
            tr("overview.loaded_groups"),
            ICON_LAYOUT,
            palette,
        ))
        .child(stat_card(
            tr("overview.active_connections"),
            app.connections.len().to_string(),
            if running {
                tr("overview.live_stats")
            } else {
                tr("overview.core_not_running")
            },
            ICON_MESSAGE,
            palette,
        ))
        .child(stat_card(
            tr("overview.traffic"),
            SharedString::from(format_speed(app.traffic_down_speed + app.traffic_up_speed)),
            SharedString::from(
                t!(
                    "overview.traffic_detail",
                    down = format_speed(app.traffic_down_speed),
                    up = format_speed(app.traffic_up_speed)
                )
                .into_owned(),
            ),
            ICON_CHART,
            palette,
        ))
        .into_any_element()
}

fn stat_card(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    icon_path: &'static str,
    palette: Palette,
) -> AnyElement {
    let label = label.into();
    let detail = detail.into();
    div()
        .flex_1()
        .min_w_0()
        .min_h(px(112.0))
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
                .child(div().text_xs().text_color(palette.muted).child(label))
                .child(icon(icon_path, palette.accent, 16.0)),
        )
        .child(
            div()
                .mt_3()
                .text_xl()
                .font_semibold()
                .text_color(palette.text)
                .child(value.into()),
        )
        .child(
            div()
                .mt_1()
                .text_xs()
                .text_color(palette.muted)
                .child(detail),
        )
        .into_any_element()
}

/// 概览页的当前出站摘要：只展示生效中的代理组与节点，点击前往代理页。
/// 不再渲染完整分组与节点列表，大订阅也不会拖慢默认首页。
fn render_overview_outbound(
    app: &PureClash,
    palette: Palette,
    cx: &mut Context<PureClash>,
) -> AnyElement {
    let (group_name, node_name) = current_outbound(app);
    div()
        .id("overview-outbound")
        .flex_1()
        .min_w_0()
        .p_4()
        .rounded_md()
        .bg(palette.surface)
        .border_1()
        .border_color(palette.border)
        .cursor_pointer()
        .child(section_heading(
            tr("overview.current_outbound"),
            tr("overview.current_outbound_detail"),
            ICON_GIT_BRANCH,
            palette,
        ))
        .child(
            div()
                .mt_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(outbound_summary_row(
                    tr("overview.outbound_group"),
                    group_name,
                    palette,
                    None,
                ))
                .child(outbound_summary_row(
                    tr("overview.outbound_node"),
                    node_name.clone(),
                    palette,
                    None,
                ))
                // 节点是具体代理（而非 DIRECT 或组）时，补充接入信息。
                .children(app.node_endpoints.get(node_name.as_ref()).map(|endpoint| {
                    // 服务器地址默认打码，眼睛图标切换明文；点击需拦截，
                    // 避免冒泡触发整卡跳转代理页。
                    let server_value = if app.server_visible {
                        SharedString::from(format!("{}:{}", endpoint.server, endpoint.port))
                    } else {
                        SharedString::from("＊＊＊＊＊＊")
                    };
                    let eye = div()
                        .id("toggle-server-visible")
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .child(icon(
                            if app.server_visible {
                                ICON_EYE_OFF
                            } else {
                                ICON_EYE
                            },
                            palette.muted,
                            14.0,
                        ))
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.server_visible = !this.server_visible;
                            cx.notify();
                        }));
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(outbound_summary_row(
                            tr("overview.outbound_kind"),
                            // 协议类型统一大写展示，如 VLESS。
                            SharedString::from(endpoint.kind.to_uppercase()),
                            palette,
                            None,
                        ))
                        .child(outbound_summary_row(
                            tr("overview.outbound_server"),
                            server_value,
                            palette,
                            Some(eye.into_any_element()),
                        ))
                })),
        )
        .on_click(cx.listener(|this, _, _, cx| this.select_page(Page::Proxies, cx)))
        .into_any_element()
}

/// 系统代理托管状态文件：存在即代表客户端正在托管，异常退出后据此自愈。

pub(super) fn integration_error_banner(error: &str, palette: Palette) -> AnyElement {
    div()
        .mb_4()
        .p_3()
        .rounded_sm()
        .bg(palette.surface_alt)
        .text_xs()
        .text_color(rgb(0xd15b5b))
        .child(error.to_string())
        .into_any_element()
}

/// 概览行：左侧固定标签，右侧当前值；`trailing` 为行尾附加控件（如眼睛图标）。
fn outbound_summary_row(
    label: SharedString,
    value: SharedString,
    palette: Palette,
    trailing: Option<AnyElement>,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .p_3()
        .rounded_sm()
        .bg(palette.surface_alt)
        .child(div().text_xs().text_color(palette.muted).child(label))
        .child(
            div()
                .flex()
                .min_w_0()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .font_medium()
                        .text_color(palette.text)
                        .child(value),
                )
                .children(trailing),
        )
        .into_any_element()
}

/// 当前出站摘要：全局模式看 GLOBAL 的选中节点，直连模式固定 DIRECT，
/// 规则模式取订阅里第一个可手动选择的主策略组。
fn current_outbound(app: &PureClash) -> (SharedString, SharedString) {
    match app.mode {
        ProxyMode::Direct => (
            tr("overview.outbound_direct"),
            tr("overview.outbound_direct"),
        ),
        ProxyMode::Global => {
            let node = app
                .groups
                .iter()
                .find(|group| group.name == "GLOBAL")
                .map(|group| group.now.clone())
                .unwrap_or_default();
            let node = if node.is_empty() {
                tr("overview.no_selection")
            } else {
                SharedString::from(node)
            };
            (SharedString::from("GLOBAL"), node)
        }
        ProxyMode::Rule => match app
            .groups
            .iter()
            .find(|group| group.selectable && group.name != "GLOBAL")
        {
            Some(group) => {
                let node = if group.now.is_empty() {
                    tr("overview.no_selection")
                } else {
                    SharedString::from(group.now.clone())
                };
                (SharedString::from(group.name.clone()), node)
            }
            None => (
                tr("overview.outbound_direct"),
                tr("overview.outbound_direct"),
            ),
        },
    }
}

/// 策略组类型的界面文案标识。

fn render_runtime_card(
    app: &PureClash,
    palette: Palette,
    cx: &mut Context<PureClash>,
) -> AnyElement {
    div()
        .w(px(280.0))
        .flex_none()
        .p_4()
        .rounded_md()
        .bg(palette.surface)
        .border_1()
        .border_color(palette.border)
        .child(section_heading(
            tr("overview.runtime_status"),
            tr("overview.local_controller"),
            ICON_TERMINAL,
            palette,
        ))
        .child(info_line(
            tr("overview.kernel"),
            if app.mihomo_running() {
                tr("overview.running")
            } else {
                tr("overview.not_started")
            },
            app.mihomo_running(),
            palette,
        ))
        .child(info_line(
            tr("overview.version"),
            &format!("v{}", app.config.mihomo_version),
            true,
            palette,
        ))
        .child(info_line(
            tr("overview.bundled_file"),
            if app.kernel_available {
                tr("overview.bundled")
            } else {
                tr("overview.missing")
            },
            app.kernel_available,
            palette,
        ))
        .child(info_line(
            tr("status.system_proxy"),
            if app.system_proxy {
                tr("overview.enabled")
            } else {
                tr("overview.disabled")
            },
            app.system_proxy,
            palette,
        ))
        .child(info_line(
            tr("settings.tun"),
            if app.tun_enabled {
                tr("overview.enabled")
            } else {
                tr("overview.disabled")
            },
            app.tun_enabled,
            palette,
        ))
        .child(
            div()
                .mt_4()
                .flex()
                .gap_2()
                .child(action_button(
                    "runtime-system-proxy",
                    tr("status.system_proxy"),
                    app.system_proxy,
                    app.mihomo_running(),
                    palette,
                    cx.listener(|this, _, _, cx| this.toggle_system_proxy(cx)),
                ))
                .child(action_button(
                    "runtime-tun",
                    tr("status.tun"),
                    app.tun_enabled,
                    app.mihomo_running(),
                    palette,
                    cx.listener(|this, _, _, cx| this.toggle_tun(cx)),
                )),
        )
        .into_any_element()
}

fn action_button(
    id: &'static str,
    label: impl Into<SharedString>,
    enabled: bool,
    available: bool,
    palette: Palette,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let label = label.into();
    div()
        .id(id)
        .h(px(30.0))
        .flex_1()
        .rounded_sm()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .opacity(if available { 1.0 } else { 0.5 })
        .bg(if enabled {
            palette.success_soft
        } else {
            palette.surface_alt
        })
        .border_1()
        .border_color(if enabled {
            palette.success
        } else {
            palette.border
        })
        .text_xs()
        .text_color(if enabled {
            palette.success
        } else {
            palette.muted
        })
        .child(label)
        .on_click(handler)
        .into_any_element()
}

fn render_recent_connections(
    app: &PureClash,
    palette: Palette,
    cx: &mut Context<PureClash>,
) -> AnyElement {
    div()
        .mt_4()
        .p_4()
        .rounded_md()
        .bg(palette.surface)
        .border_1()
        .border_color(palette.border)
        .child(section_heading(
            tr("overview.recent_connections"),
            tr("overview.recent_connections_detail"),
            ICON_CHART,
            palette,
        ))
        .children(if app.connections.is_empty() {
            vec![
                div()
                    .py(px(20.0))
                    .text_xs()
                    .text_color(palette.muted)
                    .child(tr("overview.no_connections"))
                    .into_any_element(),
            ]
        } else {
            // 最新建立的三条连接；完整列表见连接页。
            app.connections
                .iter()
                .rev()
                .take(3)
                .map(|connection| connection_row(connection, palette, cx))
                .collect()
        })
        .into_any_element()
}
