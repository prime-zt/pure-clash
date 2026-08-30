//! 关于页：应用标识、版本、源码仓库、开源组件清单与更新检查。

use gpui::{AnyElement, Context, Styled, div, px};
use rust_i18n::t;

use super::*;
use crate::assets::{ICON_APP, ICON_GIT_BRANCH};
use crate::theme::{FontWeightExt, Palette};

/// 源码仓库地址；发布仓库变动时请同步更新 [`RELEASES_API_URL`]。
const SOURCE_CODE_URL: &str = "https://github.com/ztion/pure-clash";
/// 最新版本查询端点（GitHub Releases API）。
const RELEASES_API_URL: &str = "https://api.github.com/repos/ztion/pure-clash/releases/latest";

/// 开源组件清单：(名称, 在项目中的用途, 许可证)。
const OPEN_SOURCE_COMPONENTS: &[(&str, &str, &str)] = &[
    ("Mihomo 内核", "独立代理核心进程", "GPL-3.0"),
    ("Wintun", "TUN 虚拟网卡驱动", "随官方分发包许可"),
    ("GPUI", "界面框架（Zed）", "Apache-2.0"),
    (
        "tray-icon / ksni",
        "Windows / Linux 系统托盘",
        "MIT OR Apache-2.0 · Unlicense",
    ),
    ("ureq", "订阅下载与控制器 REST 调用", "MIT OR Apache-2.0"),
    ("rust-i18n", "界面多语言", "MIT"),
    (
        "serde / serde_yaml / serde_json",
        "配置解析与序列化",
        "MIT OR Apache-2.0",
    ),
    ("anyhow", "错误处理与上下文", "MIT OR Apache-2.0"),
    ("uuid", "随机 secret 与配置标识", "MIT OR Apache-2.0"),
];

pub(super) fn render_about(
    app: &PureClash,
    palette: Palette,
    cx: &mut Context<PureClash>,
) -> AnyElement {
    div()
        .p_6()
        .flex()
        .flex_col()
        .gap_4()
        // 应用标识：大图标 + 名称 + 版本。
        .child(
            div()
                .p_6()
                .rounded_md()
                .flex()
                .flex_col()
                .items_center()
                .gap_2()
                .bg(palette.surface)
                .border_1()
                .border_color(palette.border)
                .child(gpui::img(ICON_APP).size(px(72.0)))
                .child(
                    div()
                        .mt_1()
                        .text_xl()
                        .font_semibold()
                        .text_color(palette.text)
                        .child("Pure Clash"),
                )
                .child(div().text_xs().text_color(palette.muted).child(
                    t!("about.version_line", version = PureClash::CURRENT_VERSION).into_owned(),
                )),
        )
        // 仓库与更新。
        .child(
            div()
                .p_4()
                .rounded_md()
                .flex()
                .flex_col()
                .gap_2()
                .bg(palette.surface)
                .border_1()
                .border_color(palette.border)
                .child(section_heading(
                    tr("about.source_repo"),
                    tr("about.source_repo_detail"),
                    ICON_GIT_BRANCH,
                    palette,
                ))
                .child(
                    div()
                        .id("about-repo-link")
                        .mt_2()
                        .px_3()
                        .h(px(32.0))
                        .rounded_sm()
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .hover(|style| style.bg(palette.surface_alt))
                        .text_sm()
                        .text_color(palette.accent)
                        .child(SOURCE_CODE_URL)
                        .on_click(|_, _, _| open_source_code_url()),
                )
                .child(
                    div()
                        .mt_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .id("about-check-updates")
                                .h(px(30.0))
                                .px_3()
                                .rounded_sm()
                                .flex()
                                .items_center()
                                .gap_2()
                                .map(|button| {
                                    if app.update_checking {
                                        button.opacity(0.6)
                                    } else {
                                        button.cursor_pointer()
                                    }
                                })
                                .bg(palette.accent)
                                .text_xs()
                                .font_medium()
                                .text_color(palette.surface)
                                .child(tr("about.check_updates"))
                                .on_click(cx.listener(|this, _, _, cx| this.check_for_updates(cx))),
                        )
                        .children(app.update_checking.then(|| {
                            div()
                                .text_xs()
                                .text_color(palette.muted)
                                .child(tr("about.checking"))
                        }))
                        .children(app.update_status.as_ref().map(|status| {
                            div()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(palette.muted)
                                .child(status.clone())
                        })),
                ),
        )
        // 开源组件清单。
        .child(
            div()
                .p_4()
                .rounded_md()
                .flex()
                .flex_col()
                .bg(palette.surface)
                .border_1()
                .border_color(palette.border)
                .child(section_heading(
                    tr("about.components"),
                    tr("about.components_detail"),
                    ICON_GIT_BRANCH,
                    palette,
                ))
                .child(
                    div().mt_3().flex().flex_col().gap_1().children(
                        OPEN_SOURCE_COMPONENTS
                            .iter()
                            .map(|(name, purpose, license)| {
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .px_3()
                                    .h(px(34.0))
                                    .rounded_sm()
                                    .hover(|style| style.bg(palette.surface_alt))
                                    .child(
                                        div()
                                            .w(px(240.0))
                                            .min_w_0()
                                            .truncate()
                                            .text_sm()
                                            .text_color(palette.text)
                                            .child(*name),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_xs()
                                            .text_color(palette.muted)
                                            .child(*purpose),
                                    )
                                    .child(
                                        div().text_xs().text_color(palette.muted).child(*license),
                                    )
                            }),
                    ),
                ),
        )
        .into_any_element()
}

/// 在系统默认浏览器中打开源码仓库。
fn open_source_code_url() {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", SOURCE_CODE_URL])
            .spawn();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg(SOURCE_CODE_URL)
            .spawn();
    }
}

/// 查询仓库最新发布版本号（去除 tag 前缀 v/V）；失败时返回错误供界面展示。
pub(super) fn latest_release_version() -> anyhow::Result<String> {
    use anyhow::{Context, anyhow, bail};

    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(10))
        .build();
    let value: serde_json::Value = agent
        .get(RELEASES_API_URL)
        .set("User-Agent", "pure-clash/update-check")
        .call()
        .map_err(|error| anyhow!("{error}"))
        .context("无法连接更新源")?
        .into_json()
        .map_err(|error| anyhow!("{error}"))
        .context("更新源响应格式无效")?;
    let tag = value
        .get("tag_name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("更新源响应缺少 tag_name"))?
        .trim_start_matches(['v', 'V'])
        .to_owned();
    if tag.is_empty() {
        bail!("更新源返回了空版本号");
    }
    Ok(tag)
}
