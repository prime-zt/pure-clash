//! 连接页：controller 实时连接列表、会话累计流量与连接关闭操作。

use gpui::{AnyElement, Context, SharedString, Styled, div, px};
use rust_i18n::t;

use super::overview::integration_error_banner;
use super::*;
use crate::assets::{ICON_MESSAGE, ICON_WINDOW_CLOSE};
use crate::mihomo::controller::ConnectionItem;
use crate::theme::Palette;

pub(super) fn render_connections(
    app: &PureClash,
    palette: Palette,
    cx: &mut Context<PureClash>,
) -> AnyElement {
    div()
        .p_6()
        .when_some(app.integration_error.as_ref(), |page, error| {
            page.child(integration_error_banner(error, palette))
        })
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
                            tr("connections.title"),
                            tr("connections.detail"),
                            ICON_MESSAGE,
                            palette,
                        ))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(div().text_xs().text_color(palette.muted).child(
                                    SharedString::from(format!(
                                        "{} ↓ {} · ↑ {}",
                                        tr("connections.total"),
                                        format_bytes(app.download_total),
                                        format_bytes(app.upload_total),
                                    )),
                                ))
                                .child(close_all_button(app, palette, cx)),
                        ),
                )
                .child(connections_header(palette))
                .children(if app.connections.is_empty() {
                    vec![connections_empty(app, palette)]
                } else {
                    // 按建立时间倒序展示，最新连接在最前；超出上限的只渲染条数提示。
                    let mut rows: Vec<AnyElement> = app
                        .connections
                        .iter()
                        .rev()
                        .take(CONNECTIONS_RENDER_LIMIT)
                        .map(|connection| connection_row(connection, palette, cx))
                        .collect();
                    let remaining = app.connections.len() - rows.len();
                    if remaining > 0 {
                        rows.push(
                            div()
                                .h(px(36.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_xs()
                                .text_color(palette.muted)
                                .child(
                                    t!("connections.more", count = remaining.to_string())
                                        .into_owned(),
                                )
                                .into_any_element(),
                        );
                    }
                    rows
                }),
        )
        .into_any_element()
}

fn close_all_button(app: &PureClash, palette: Palette, cx: &mut Context<PureClash>) -> AnyElement {
    let available = app.mihomo_running() && !app.connections.is_empty();
    div()
        .id("connections-close-all")
        .px_3()
        .h(px(28.0))
        .rounded_sm()
        .flex()
        .items_center()
        .map(|button| {
            if available {
                button.cursor_pointer()
            } else {
                button.opacity(0.5)
            }
        })
        .bg(palette.surface_alt)
        .text_xs()
        .text_color(palette.muted)
        .child(tr("connections.close_all"))
        .on_click(cx.listener(|this, _, _, cx| {
            if this.mihomo_running() && !this.connections.is_empty() {
                this.close_all_connections(cx);
            }
        }))
        .into_any_element()
}

fn connections_header(palette: Palette) -> AnyElement {
    div()
        .mt_4()
        .flex()
        .items_center()
        .gap_3()
        .px_3()
        .h(px(34.0))
        .bg(palette.surface_alt)
        .text_xs()
        .text_color(palette.muted)
        .child(div().w(px(110.0)).child(tr("connections.process")))
        .child(div().flex_1().child(tr("connections.target")))
        .child(div().w(px(80.0)).child(tr("connections.chain")))
        .child(div().w(px(110.0)).child(tr("connections.rule")))
        .child(div().w(px(76.0)).child(tr("connections.download")))
        .child(div().w(px(76.0)).child(tr("connections.upload")))
        .child(div().w(px(36.0)))
        .into_any_element()
}

/// 内核未运行或暂无连接时的引导空态。
fn connections_empty(app: &PureClash, palette: Palette) -> AnyElement {
    div()
        .py(px(36.0))
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .child(
            div()
                .text_sm()
                .text_color(palette.text)
                .child(if app.mihomo_running() {
                    tr("connections.empty")
                } else {
                    tr("connections.empty_no_core")
                }),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette.muted)
                .child(tr("connections.empty_hint")),
        )
        .into_any_element()
}

pub(super) fn connection_row(
    connection: &ConnectionItem,
    palette: Palette,
    cx: &mut Context<PureClash>,
) -> AnyElement {
    let process = if connection.metadata.process.is_empty() {
        "—".to_owned()
    } else {
        connection.metadata.process.clone()
    };
    div()
        .min_h(px(52.0))
        .px_3()
        .flex()
        .items_center()
        .gap_3()
        .border_b_1()
        .border_color(palette.border)
        .text_xs()
        .child(
            div()
                .w(px(110.0))
                .text_color(palette.text)
                .text_ellipsis()
                .whitespace_nowrap()
                .child(process),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(palette.text)
                .text_ellipsis()
                .whitespace_nowrap()
                .child(SharedString::from(connection.target())),
        )
        .child(
            div()
                .w(px(80.0))
                .text_ellipsis()
                .whitespace_nowrap()
                .text_color(palette.muted)
                .child(SharedString::from(connection.chain().to_owned())),
        )
        .child(
            div()
                .w(px(110.0))
                .text_ellipsis()
                .whitespace_nowrap()
                .text_color(palette.muted)
                .child(SharedString::from(connection.rule_label())),
        )
        .child(
            div()
                .w(px(76.0))
                .text_color(palette.muted)
                .child(SharedString::from(format_bytes(connection.download))),
        )
        .child(
            div()
                .w(px(76.0))
                .text_color(palette.muted)
                .child(SharedString::from(format_bytes(connection.upload))),
        )
        .child({
            let id = connection.id.clone();
            div()
                .id(SharedString::from(format!(
                    "connection-close-{}",
                    connection.id
                )))
                .w(px(36.0))
                .flex()
                .justify_center()
                .cursor_pointer()
                .child(icon(ICON_WINDOW_CLOSE, palette.muted, 13.0))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.close_connection(id.clone(), cx);
                }))
        })
        .into_any_element()
}
