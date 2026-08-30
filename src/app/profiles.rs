//! 配置页：内置默认配置行、订阅列表与添加/更新/删除/激活操作。

use gpui::{AnyElement, Context, SharedString, Styled, div, px};
use rust_i18n::t;

use super::*;
use crate::assets::{ICON_CIRCLE_CHECK, ICON_FILE_CODE, ICON_SHIELD_CHECK};
use crate::config::ProfileMeta;
use crate::theme::{FontWeightExt, Palette};

pub(super) fn render_profiles(
    app: &PureClash,
    palette: Palette,
    cx: &mut Context<PureClash>,
) -> AnyElement {
    let busy = app.profile_busy.is_some();
    div()
        .p_6()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(section_heading(
                    tr("profiles.title"),
                    tr("profiles.detail"),
                    ICON_FILE_CODE,
                    palette,
                ))
                .child(
                    div()
                        .id("profiles-add")
                        .h(px(30.0))
                        .px_3()
                        .rounded_sm()
                        .flex()
                        .items_center()
                        .gap_1()
                        .map(|button| {
                            if busy {
                                button.opacity(0.5)
                            } else {
                                button.cursor_pointer()
                            }
                        })
                        .bg(palette.accent)
                        .text_xs()
                        .font_medium()
                        .text_color(palette.surface)
                        .child(tr("profiles.add"))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_profile_form(cx))),
                ),
        )
        .when(app.profile_form_open, |page| {
            page.child(render_profile_form(app, palette, cx))
        })
        .when_some(app.profile_error.as_ref(), |page, error| {
            page.child(
                div()
                    .mt_3()
                    .p_3()
                    .rounded_sm()
                    .bg(palette.surface_alt)
                    .text_xs()
                    .text_color(rgb(0xd15b5b))
                    .child(error.clone()),
            )
        })
        .when_some(app.profile_busy.as_ref(), |page, busy| {
            page.child(
                div()
                    .mt_3()
                    .p_3()
                    .rounded_sm()
                    .bg(palette.surface_alt)
                    .text_xs()
                    .text_color(palette.accent)
                    .child(busy.clone()),
            )
        })
        .child(
            div()
                .mt_4()
                .flex()
                .flex_col()
                .gap_3()
                // 内置默认配置常驻首行：无激活订阅时即为选中态。
                .child(default_profile_row(app, palette, cx))
                .children(
                    app.profiles
                        .iter()
                        .enumerate()
                        .map(|(index, meta)| profile_row(app, index, meta, palette, cx)),
                ),
        )
        .when(app.profiles.is_empty(), |page| {
            page.child(
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
                            .child(tr("profiles.empty_title")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.muted)
                            .child(tr("profiles.empty_detail")),
                    ),
            )
        })
        .into_any_element()
}

/// 添加订阅的内联表单：名称可选（默认取主机名），URL 必填。
fn render_profile_form(
    app: &PureClash,
    palette: Palette,
    cx: &mut Context<PureClash>,
) -> AnyElement {
    let busy = app.profile_busy.is_some();
    div()
        .mt_4()
        .p_4()
        .rounded_md()
        .flex()
        .flex_col()
        .gap_3()
        .bg(palette.surface)
        .border_1()
        .border_color(palette.accent)
        .child(
            div()
                .text_sm()
                .font_medium()
                .text_color(palette.text)
                .child(tr("profiles.form_title")),
        )
        .child(
            div()
                .p_2()
                .rounded_sm()
                .bg(palette.surface_alt)
                .border_1()
                .border_color(palette.border)
                .text_sm()
                .text_color(palette.text)
                .line_height(px(20.))
                .child(app.profile_form_name.clone()),
        )
        .child(
            div()
                .p_2()
                .rounded_sm()
                .bg(palette.surface_alt)
                .border_1()
                .border_color(palette.border)
                .text_sm()
                .text_color(palette.text)
                .line_height(px(20.))
                .child(app.profile_form_url.clone()),
        )
        .child(
            div()
                .flex()
                .gap_2()
                .child(
                    div()
                        .id("profile-form-submit")
                        .flex_1()
                        .h(px(32.0))
                        .rounded_sm()
                        .flex()
                        .items_center()
                        .justify_center()
                        .map(|button| {
                            if busy {
                                button.opacity(0.5)
                            } else {
                                button.cursor_pointer()
                            }
                        })
                        .bg(palette.accent)
                        .text_xs()
                        .font_medium()
                        .text_color(palette.surface)
                        .child(tr("profiles.form_submit"))
                        .on_click(cx.listener(|this, _, _, cx| this.add_subscription(cx))),
                )
                .child(
                    div()
                        .id("profile-form-cancel")
                        .flex_1()
                        .h(px(32.0))
                        .rounded_sm()
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .bg(palette.surface_alt)
                        .text_xs()
                        .text_color(palette.muted)
                        .child(tr("profiles.form_cancel"))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_profile_form(cx))),
                ),
        )
        .into_any_element()
}

/// 格式化更新时间为本地可读日期；时间戳为 0 时显示“未更新”。
fn format_profile_time(timestamp: u64) -> SharedString {
    if timestamp == 0 {
        return tr("profiles.never_updated");
    }
    // 无chrono 依赖，用天数推算 UTC 日期，展示足够用的粗粒度时间。
    let days = timestamp / 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    SharedString::from(format!("{year:04}-{month:02}-{day:02}"))
}

/// 从 UNIX 天数转换为民用日期（Howard Hinnant 算法）。
fn civil_from_days(days: i64) -> (i64, u64, u64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 内置默认配置行：仅含 DIRECT 出站；无激活订阅时处于选中态，点击切回。
fn default_profile_row(
    app: &PureClash,
    palette: Palette,
    cx: &mut Context<PureClash>,
) -> AnyElement {
    let active = app.active_profile.is_none();
    let busy = app.profile_busy.is_some();
    div()
        .id("profile-builtin")
        .min_h(px(76.0))
        .p_4()
        .rounded_md()
        .flex()
        .items_center()
        .gap_3()
        .map(|row| {
            if busy {
                row.opacity(0.7)
            } else {
                row.cursor_pointer()
            }
        })
        .bg(palette.surface)
        .border_1()
        .border_color(if active {
            palette.accent
        } else {
            palette.border
        })
        .child(
            div()
                .size_9()
                .rounded_md()
                .flex()
                .items_center()
                .justify_center()
                .bg(if active {
                    palette.accent_soft
                } else {
                    palette.surface_alt
                })
                .child(icon(
                    if active {
                        ICON_CIRCLE_CHECK
                    } else {
                        ICON_SHIELD_CHECK
                    },
                    if active {
                        palette.accent
                    } else {
                        palette.muted
                    },
                    17.0,
                )),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_sm()
                        .font_medium()
                        .text_color(palette.text)
                        .child(tr("profiles.builtin_name")),
                )
                .child(
                    div()
                        .mt_1()
                        .text_xs()
                        .text_color(palette.muted)
                        .child(tr("profiles.builtin_detail")),
                ),
        )
        .child(
            div().flex().items_center().gap_2().child(
                div()
                    .text_xs()
                    .text_color(if active {
                        palette.success
                    } else {
                        palette.muted
                    })
                    .child(tr(if active {
                        "profiles.active"
                    } else {
                        "profiles.activate"
                    })),
            ),
        )
        .on_click(cx.listener(|this, _, _, cx| this.activate_default_profile(cx)))
        .into_any_element()
}

fn profile_row(
    app: &PureClash,
    index: usize,
    meta: &ProfileMeta,
    palette: Palette,
    cx: &mut Context<PureClash>,
) -> AnyElement {
    let active = app.active_profile.as_deref() == Some(meta.id.as_str());
    let busy = app.profile_busy.is_some();
    let source = if meta.url.is_some() {
        tr("profiles.source_subscription")
    } else {
        tr("profiles.source_local")
    };
    div()
        .id(SharedString::from(format!("profile-{index}")))
        .min_h(px(76.0))
        .p_4()
        .rounded_md()
        .flex()
        .items_center()
        .gap_3()
        .map(|row| {
            if busy {
                row.opacity(0.7)
            } else {
                row.cursor_pointer()
            }
        })
        .bg(palette.surface)
        .border_1()
        .border_color(if active {
            palette.accent
        } else {
            palette.border
        })
        .child(
            div()
                .size_9()
                .rounded_md()
                .flex()
                .items_center()
                .justify_center()
                .bg(if active {
                    palette.accent_soft
                } else {
                    palette.surface_alt
                })
                .child(icon(
                    if active {
                        ICON_CIRCLE_CHECK
                    } else {
                        ICON_FILE_CODE
                    },
                    if active {
                        palette.accent
                    } else {
                        palette.muted
                    },
                    17.0,
                )),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_sm()
                        .font_medium()
                        .text_color(palette.text)
                        .child(meta.name.clone()),
                )
                .child(
                    div().mt_1().text_xs().text_color(palette.muted).child(
                        t!(
                            "profiles.updated",
                            source = source.to_string(),
                            updated = format_profile_time(meta.updated_at).to_string()
                        )
                        .into_owned(),
                    ),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .children(if active {
                    vec![
                        div()
                            .text_xs()
                            .text_color(palette.success)
                            .child(tr("profiles.active"))
                            .into_any_element(),
                    ]
                } else {
                    vec![
                        div()
                            .text_xs()
                            .text_color(palette.muted)
                            .child(tr("profiles.activate"))
                            .into_any_element(),
                    ]
                })
                .children(meta.url.as_ref().map(|_| {
                    div()
                        .id(SharedString::from(format!("profile-update-{index}")))
                        .px_2()
                        .h(px(26.0))
                        .rounded_sm()
                        .flex()
                        .items_center()
                        .map(|button| {
                            if busy {
                                button.opacity(0.5)
                            } else {
                                button.cursor_pointer()
                            }
                        })
                        .bg(palette.surface_alt)
                        .text_xs()
                        .text_color(palette.muted)
                        .child(tr("profiles.update"))
                        .on_click(cx.listener(move |this, _, _, cx| this.update_profile(index, cx)))
                        .into_any_element()
                }))
                .children(Some({
                    div()
                        .id(SharedString::from(format!("profile-delete-{index}")))
                        .px_2()
                        .h(px(26.0))
                        .rounded_sm()
                        .flex()
                        .items_center()
                        .map(|button| {
                            if busy {
                                button.opacity(0.5)
                            } else {
                                button.cursor_pointer()
                            }
                        })
                        .bg(palette.surface_alt)
                        .text_xs()
                        .text_color(rgb(0xd15b5b))
                        .child(tr("profiles.delete"))
                        .on_click(cx.listener(move |this, _, _, cx| this.delete_profile(index, cx)))
                        .into_any_element()
                })),
        )
        .on_click(cx.listener(move |this, _, _, cx| this.activate_profile_clicked(index, cx)))
        .into_any_element()
}
