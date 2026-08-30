//! 页面容器与页头：页面路由、标题副标题、系统代理 / TUN 开关芯片、内核按钮。

#[cfg(target_os = "linux")]
use gpui::Decorations;
use gpui::{AnyElement, App, ClickEvent, Context, SharedString, Styled, Window, div, px};

use super::*;
use crate::assets::ICON_TERMINAL;
use crate::theme::{FontWeightExt, Palette};

use super::about::render_about;
use super::connections::render_connections;
use super::overview::render_overview;
use super::profiles::render_profiles;
use super::proxies::render_proxies;
use super::settings::render_settings;

pub(super) fn render_page(
    app: &PureClash,
    palette: Palette,
    window: &mut Window,
    cx: &mut Context<PureClash>,
) -> AnyElement {
    let page = div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .bg(palette.background)
        .child(render_page_header(app, palette, cx))
        .child(
            div()
                .id("page-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(match app.page {
                    Page::Overview => render_overview(app, palette, cx),
                    Page::Proxies => render_proxies(app, palette, cx),
                    Page::Connections => render_connections(app, palette, cx),
                    Page::Profiles => render_profiles(app, palette, cx),
                    Page::Settings => render_settings(app, palette, cx),
                    Page::About => render_about(app, palette, cx),
                }),
        );

    #[cfg(target_os = "linux")]
    let page = match window.window_decorations() {
        Decorations::Client { tiling } if !tiling.bottom && !tiling.right => {
            page.rounded_br(crate::platform::CLIENT_SIDE_DECORATION_SIZE)
        }
        _ => page,
    };

    #[cfg(not(target_os = "linux"))]
    let _ = window;

    page.into_any_element()
}

fn render_page_header(
    app: &PureClash,
    palette: Palette,
    cx: &mut Context<PureClash>,
) -> AnyElement {
    div()
        .h(px(68.0))
        .px_6()
        .flex_none()
        .flex()
        .items_center()
        .justify_between()
        .border_b_1()
        .border_color(palette.border)
        .child(
            div()
                .child(
                    div()
                        .text_lg()
                        .font_semibold()
                        .text_color(palette.text)
                        .child(app.page.label()),
                )
                .child(
                    div()
                        .mt_1()
                        .text_xs()
                        .text_color(palette.muted)
                        .child(page_subtitle(app.page)),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(header_status_toggle(
                    "header-system-proxy",
                    tr("status.system_proxy"),
                    app.system_proxy,
                    palette,
                    cx.listener(|this, _, _, cx| this.toggle_system_proxy(cx)),
                ))
                .child(header_status_toggle(
                    "header-tun",
                    tr("status.tun"),
                    app.tun_enabled,
                    palette,
                    cx.listener(|this, _, _, cx| this.toggle_tun(cx)),
                ))
                .child(
                    div()
                        .id("toggle-core-header")
                        .h(px(30.0))
                        .px_3()
                        .rounded_sm()
                        .flex()
                        .items_center()
                        .gap_2()
                        .cursor_pointer()
                        .bg(if app.mihomo_running() {
                            palette.surface
                        } else {
                            palette.accent
                        })
                        .border_1()
                        .border_color(if app.mihomo_running() {
                            palette.border
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
                        .child(icon(ICON_TERMINAL, palette.muted, 13.0))
                        .child(if app.mihomo_running() {
                            tr("app.stop")
                        } else {
                            tr("app.start")
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_core(cx))),
                ),
        )
        .into_any_element()
}

fn page_subtitle(page: Page) -> SharedString {
    match page {
        Page::Overview => tr("page.overview_subtitle"),
        Page::Proxies => tr("page.proxies_subtitle"),
        Page::Connections => tr("page.connections_subtitle"),
        Page::Profiles => tr("page.profiles_subtitle"),
        Page::Settings => tr("page.settings_subtitle"),
        Page::About => tr("page.about_subtitle"),
    }
}

/// 页头的系统代理 / TUN 开关芯片：状态一眼可见，点击直接启停。
#[allow(clippy::type_complexity)]
fn header_status_toggle(
    id: &'static str,
    label: SharedString,
    enabled: bool,
    palette: Palette,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .h(px(28.0))
        .px_2()
        .rounded_sm()
        .flex()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(|style| style.bg(palette.surface_alt))
        .bg(if enabled {
            palette.success_soft
        } else {
            palette.surface
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
        .child(div().size_2().rounded_full().bg(if enabled {
            palette.success
        } else {
            palette.border
        }))
        .child(format!(
            "{} {}",
            label,
            if enabled {
                tr("status.on")
            } else {
                tr("status.off")
            }
        ))
        .on_click(on_toggle)
        .into_any_element()
}
