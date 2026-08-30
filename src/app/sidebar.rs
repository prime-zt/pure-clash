//! 侧边栏：品牌区、页面导航与底部内核状态卡片。

#[cfg(target_os = "linux")]
use gpui::Decorations;
use gpui::{AnyElement, Context, Styled, Window, div, px};
use rust_i18n::t;

use super::*;
use crate::assets::{ICON_INFO, ICON_TERMINAL};
use crate::theme::{FontWeightExt, Palette};

pub(super) fn render_sidebar(
    app: &PureClash,
    palette: Palette,
    window: &mut Window,
    cx: &mut Context<PureClash>,
) -> AnyElement {
    let sidebar = div()
        .w(px(220.0))
        .h_full()
        .flex_none()
        .flex()
        .flex_col()
        .bg(palette.surface)
        .border_r_1()
        .border_color(palette.border)
        .child(
            div().px_4().pt_5().pb_4().child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(palette.muted)
                    .child("PURE CLASH"),
            ),
        )
        .child(
            div().px_2().flex().flex_col().gap_1().children(
                Page::all()
                    .into_iter()
                    .map(|page| nav_item(page, app.page, palette, cx)),
            ),
        )
        .child(div().flex_1())
        .child(sidebar_runtime(app, palette, cx))
        .child(
            div()
                .px_3()
                .pb_3()
                .pt_2()
                .border_t_1()
                .border_color(palette.border)
                .child(
                    div()
                        .id("sidebar-about")
                        .h(px(34.0))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .cursor_pointer()
                        .text_xs()
                        .text_color(palette.muted)
                        .hover(|style| style.bg(palette.surface_alt).text_color(palette.text))
                        .child(icon(ICON_INFO, palette.muted, 14.0))
                        .child(tr("page.about"))
                        .on_click(cx.listener(|this, _, _, cx| this.select_page(Page::About, cx))),
                ),
        );

    #[cfg(target_os = "linux")]
    let sidebar = match window.window_decorations() {
        Decorations::Client { tiling } if !tiling.bottom && !tiling.left => {
            sidebar.rounded_bl(crate::platform::CLIENT_SIDE_DECORATION_SIZE)
        }
        _ => sidebar,
    };

    #[cfg(not(target_os = "linux"))]
    let _ = window;

    sidebar.into_any_element()
}

fn nav_item(
    page: Page,
    selected: Page,
    palette: Palette,
    cx: &mut Context<PureClash>,
) -> AnyElement {
    let active = page == selected;
    div()
        .id(page.label())
        .h(px(38.0))
        .px_3()
        .rounded_md()
        .flex()
        .items_center()
        .gap_3()
        .cursor_pointer()
        .bg(if active {
            palette.accent_soft
        } else {
            palette.surface
        })
        .text_sm()
        .font_medium()
        .text_color(if active {
            palette.accent
        } else {
            palette.muted
        })
        .hover(|style| style.bg(palette.surface_alt).text_color(palette.text))
        .child(icon(
            page.icon(),
            if active {
                palette.accent
            } else {
                palette.muted
            },
            16.0,
        ))
        .child(page.label())
        .on_click(cx.listener(move |this, _, _, cx| this.select_page(page, cx)))
        .into_any_element()
}

fn sidebar_runtime(app: &PureClash, palette: Palette, cx: &mut Context<PureClash>) -> AnyElement {
    div()
        .mx_3()
        .mb_3()
        .p_3()
        .rounded_md()
        .bg(palette.surface_alt)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(icon(ICON_TERMINAL, palette.muted, 14.0))
                        .child(
                            div()
                                .text_xs()
                                .font_medium()
                                .text_color(palette.text)
                                .child(tr("app.core")),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(match app.core_state {
                            CoreState::Running => palette.success,
                            CoreState::Starting => palette.accent,
                            CoreState::Stopped => palette.muted,
                        })
                        .child(match app.core_state {
                            CoreState::Running => tr("app.ready"),
                            CoreState::Starting => tr("app.core_starting"),
                            CoreState::Stopped => tr("app.stopped"),
                        }),
                ),
        )
        .child(
            div().mt_2().text_xs().text_color(palette.muted).child(
                t!(
                    "app.version_controller",
                    version = app.config.mihomo_version.clone()
                )
                .into_owned(),
            ),
        )
        .child(
            div()
                .id("toggle-core-sidebar")
                .mt_3()
                .h(px(30.0))
                .rounded_sm()
                .flex()
                .items_center()
                .justify_center()
                .map(|button| {
                    if app.core_operable() {
                        button.cursor_pointer()
                    } else {
                        button.opacity(0.6)
                    }
                })
                .bg(if app.mihomo_running() {
                    palette.surface
                } else {
                    palette.accent
                })
                .text_xs()
                .font_medium()
                .text_color(if app.mihomo_running() {
                    palette.text
                } else {
                    palette.surface
                })
                .child(if app.mihomo_running() {
                    tr("app.stop_core")
                } else {
                    tr("app.start_core")
                })
                .on_click(cx.listener(|this, _, _, cx| this.toggle_core(cx))),
        )
        .when_some(app.mihomo_error.as_ref(), |card, error| {
            card.child(
                div()
                    .mt_2()
                    .text_xs()
                    .text_color(rgb(0xd15b5b))
                    .child(error.clone()),
            )
        })
        .into_any_element()
}
