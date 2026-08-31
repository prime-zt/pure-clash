//! 设置页：内核与系统代理、界面语言、主题与运行目录信息。

use gpui::{AnyElement, Context, SharedString, Styled, div, px};

use super::overview::integration_error_banner;
use super::*;
use crate::assets::{ICON_FOLDER, ICON_REFRESH_CW, ICON_SETTINGS, ICON_SHIELD_CHECK};
use crate::config::Language;
use crate::theme::{FontWeightExt, Palette};

pub(super) fn render_settings(
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
                .child(section_heading(
                    tr("settings.kernel_proxy"),
                    tr("settings.integration_detail"),
                    ICON_SETTINGS,
                    palette,
                ))
                .child(setting_row(
                    "setting-autostart",
                    tr("settings.autostart"),
                    if app.autostart_available {
                        tr("settings.autostart_detail")
                    } else {
                        tr("settings.autostart_unavailable")
                    },
                    app.autostart_enabled,
                    app.autostart_available,
                    palette,
                    cx.listener(|this, _, _, cx| this.toggle_autostart(cx)),
                ))
                .child(setting_row(
                    "setting-system-proxy",
                    tr("settings.system_proxy"),
                    tr("settings.system_proxy_detail"),
                    app.system_proxy,
                    true,
                    palette,
                    cx.listener(|this, _, _, cx| this.toggle_system_proxy(cx)),
                ))
                .child(setting_row(
                    "setting-tun",
                    tr("settings.tun"),
                    tr("settings.tun_detail"),
                    app.tun_running(),
                    true,
                    palette,
                    cx.listener(|this, _, _, cx| this.toggle_tun(cx)),
                ))
                .child(setting_row(
                    "setting-dark-theme",
                    tr("settings.dark_theme"),
                    tr("settings.dark_theme_detail"),
                    app.config.theme.is_dark(),
                    true,
                    palette,
                    cx.listener(|this, _, _, cx| {
                        this.toggle_theme(cx);
                    }),
                ))
                .child(language_setting_row(app.config.language, palette, cx)),
        )
        .child(
            div()
                .mt_4()
                .p_4()
                .rounded_md()
                .bg(palette.surface)
                .border_1()
                .border_color(palette.border)
                .child(section_heading(
                    tr("settings.geodata"),
                    tr("settings.geodata_detail"),
                    ICON_SHIELD_CHECK,
                    palette,
                ))
                .child(geodata_update_row(app, palette, cx)),
        )
        .child(
            div()
                .mt_4()
                .p_4()
                .rounded_md()
                .bg(palette.surface)
                .border_1()
                .border_color(palette.border)
                .child(section_heading(
                    tr("settings.runtime_dir"),
                    tr("settings.runtime_dir_detail"),
                    ICON_FOLDER,
                    palette,
                ))
                .child(path_row(
                    tr("settings.main_config"),
                    app.paths.config_display(),
                    palette,
                ))
                .child(path_row(
                    tr("settings.data_dir"),
                    app.paths.data_display(),
                    palette,
                ))
                .child(path_row(
                    tr("settings.mihomo_config"),
                    app.paths.runtime_mihomo_config_display(),
                    palette,
                ))
                .child(path_row(
                    tr("settings.mihomo_data"),
                    app.paths.mihomo_data_display(),
                    palette,
                ))
                .child(path_row(
                    tr("settings.controller"),
                    SharedString::from(
                        app.baseline
                            .as_ref()
                            .map(|baseline| baseline.controller_addr.clone())
                            .unwrap_or_else(|| "—".to_owned()),
                    ),
                    palette,
                )),
        )
        .into_any_element()
}

/// Geo 数据是随包资源的用户态副本；更新按钮只替换数据目录，不修改安装目录。
fn geodata_update_row(
    app: &PureClash,
    palette: Palette,
    cx: &mut Context<PureClash>,
) -> AnyElement {
    let revision = short_revision(&app.geodata_info.revision);
    let updated = super::profiles::format_profile_time(app.geodata_info.updated_at);
    div()
        .mt_2()
        .min_h(px(64.0))
        .flex()
        .items_center()
        .gap_3()
        .border_b_1()
        .border_color(palette.border)
        .child(
            div()
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_medium()
                        .text_color(palette.text)
                        .child(tr("settings.geodata_files")),
                )
                .child(
                    div().mt_1().text_xs().text_color(palette.muted).child(
                        t!(
                            "settings.geodata_version",
                            revision = revision,
                            updated = updated.to_string()
                        )
                        .into_owned(),
                    ),
                )
                .children(app.geodata_status.as_ref().map(|status| {
                    div()
                        .mt_1()
                        .text_xs()
                        .text_color(palette.muted)
                        .child(status.clone())
                })),
        )
        .child(
            div()
                .id("update-geodata")
                .h(px(30.0))
                .px_3()
                .rounded_sm()
                .flex()
                .items_center()
                .gap_2()
                .map(|button| {
                    if app.geodata_updating {
                        button.opacity(0.6)
                    } else {
                        button.cursor_pointer()
                    }
                })
                .bg(palette.accent)
                .text_xs()
                .font_medium()
                .text_color(palette.surface)
                .child(icon(ICON_REFRESH_CW, palette.surface, 13.0))
                .child(if app.geodata_updating {
                    tr("settings.geodata_updating")
                } else {
                    tr("settings.geodata_update")
                })
                .on_click(cx.listener(|this, _, _, cx| this.update_geodata(cx))),
        )
        .into_any_element()
}

fn setting_row(
    id: &'static str,
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    enabled: bool,
    available: bool,
    palette: Palette,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let title = title.into();
    let detail = detail.into();
    div()
        .min_h(px(62.0))
        .flex()
        .items_center()
        .gap_3()
        .border_b_1()
        .border_color(palette.border)
        .opacity(if available { 1.0 } else { 0.5 })
        .child(
            div()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_medium()
                        .text_color(palette.text)
                        .child(title),
                )
                .child(
                    div()
                        .mt_1()
                        .text_xs()
                        .text_color(palette.muted)
                        .child(detail),
                ),
        )
        .child(
            div()
                .id(id)
                .w(px(38.0))
                .h(px(22.0))
                .p(px(3.0))
                .rounded_full()
                .flex()
                .items_center()
                .bg(if enabled {
                    palette.accent
                } else {
                    palette.border
                })
                .when(enabled, |toggle| toggle.justify_end())
                .when(!enabled, |toggle| toggle.justify_start())
                .child(div().size_4().rounded_full().bg(palette.surface))
                .when(available, |toggle| {
                    toggle.cursor_pointer().on_click(handler)
                }),
        )
        .into_any_element()
}

fn language_setting_row(
    language: Language,
    palette: Palette,
    cx: &mut Context<PureClash>,
) -> AnyElement {
    div()
        .min_h(px(62.0))
        .flex()
        .items_center()
        .gap_3()
        .border_b_1()
        .border_color(palette.border)
        .child(
            div()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_medium()
                        .text_color(palette.text)
                        .child(tr("settings.language")),
                )
                .child(
                    div()
                        .mt_1()
                        .text_xs()
                        .text_color(palette.muted)
                        .child(tr("settings.language_detail")),
                ),
        )
        .child(
            div()
                .flex()
                .p_1()
                .rounded_sm()
                .bg(palette.surface_alt)
                .border_1()
                .border_color(palette.border)
                .child(language_option(
                    Language::Chinese,
                    tr("settings.language_chinese"),
                    language,
                    palette,
                    cx,
                ))
                .child(language_option(
                    Language::English,
                    tr("settings.language_english"),
                    language,
                    palette,
                    cx,
                )),
        )
        .into_any_element()
}

fn language_option(
    language: Language,
    label: SharedString,
    selected: Language,
    palette: Palette,
    cx: &mut Context<PureClash>,
) -> AnyElement {
    let active = language == selected;
    div()
        .id(match language {
            Language::Chinese => "language-zh-cn",
            Language::English => "language-en-us",
        })
        .h(px(28.0))
        .px_3()
        .rounded_sm()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .text_xs()
        .font_medium()
        .bg(if active {
            palette.surface
        } else {
            palette.surface_alt
        })
        .text_color(if active {
            palette.accent
        } else {
            palette.muted
        })
        .child(label)
        .on_click(cx.listener(move |this, _, _, cx| this.set_language(language, cx)))
        .into_any_element()
}

fn path_row(
    label: impl Into<SharedString>,
    path: impl Into<SharedString>,
    palette: Palette,
) -> AnyElement {
    let label = label.into();
    let path = path.into();
    div()
        .min_h(px(42.0))
        .flex()
        .items_center()
        .justify_between()
        .border_b_1()
        .border_color(palette.border)
        .child(div().text_sm().text_color(palette.text).child(label))
        .child(
            div()
                .max_w(px(380.0))
                .text_xs()
                .text_color(palette.muted)
                .text_ellipsis()
                .whitespace_nowrap()
                .child(path),
        )
        .into_any_element()
}
