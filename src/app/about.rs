//! 关于页：应用标识、版本、源码仓库、开源组件清单与更新检查。

use gpui::{AnyElement, Context, Styled, div, px};
use rust_i18n::t;

use super::*;
use crate::assets::{ICON_APP, ICON_GIT_BRANCH};
use crate::theme::{FontWeightExt, Palette};

/// 源码仓库地址；发布仓库变动时请同步更新 [`RELEASES_API_URL`]。
const SOURCE_CODE_URL: &str = "https://github.com/prime-zt/pure-clash";
/// 最新版本查询端点（GitHub Releases API）。
const RELEASES_API_URL: &str = "https://api.github.com/repos/prime-zt/pure-clash/releases/latest";

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
                            // 发现新版本时用主题色强调，其余结果保持弱化展示。
                            let highlighted = app.update_available;
                            div()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .map(|text| {
                                    if highlighted {
                                        text.font_medium().text_color(palette.accent)
                                    } else {
                                        text.text_color(palette.muted)
                                    }
                                })
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

/// 查询仓库最新发布版本号（去除 tag 前缀 v/V）。
///
/// 返回 `Ok(None)` 表示仓库还没有任何已发布版本（GitHub API 404）；
/// 其余失败以错误返回，供界面展示原因。
pub(super) fn latest_release_version() -> anyhow::Result<Option<String>> {
    use anyhow::{Context, anyhow};

    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(10))
        .build();
    let value: serde_json::Value = match agent
        .get(RELEASES_API_URL)
        .set("User-Agent", "pure-clash/update-check")
        .call()
    {
        Ok(response) => response
            .into_json()
            .map_err(|error| anyhow!("{error}"))
            .context("更新源响应格式无效")?,
        // 仓库尚无 release 时 GitHub 返回 404，属于正常情况而非故障。
        Err(ureq::Error::Status(404, _)) => return Ok(None),
        Err(error) => return Err(anyhow!("{error}")).context("无法连接更新源"),
    };
    let tag = value
        .get("tag_name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("更新源响应缺少 tag_name"))?
        .trim_start_matches(['v', 'V'])
        .to_owned();
    if tag.is_empty() {
        anyhow::bail!("更新源返回了空版本号");
    }
    Ok(Some(tag))
}

/// 判断 `latest` 是否严格新于 `current`；版本号按数值逐段比较，
/// 段数不足补零，非数字后缀（如 `-rc1`）截断忽略。
pub(super) fn is_newer_version(latest: &str, current: &str) -> bool {
    fn components(version: &str) -> Vec<u64> {
        version
            .split('.')
            .map(|part| {
                part.chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0)
            })
            .collect()
    }

    let mut latest = components(latest);
    let mut current = components(current);
    let length = latest.len().max(current.len());
    latest.resize(length, 0);
    current.resize(length, 0);
    latest > current
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_version_compares_numerically_per_segment() {
        assert!(is_newer_version("0.2.0", "0.1.0"));
        assert!(is_newer_version("0.1.10", "0.1.9"));
        assert!(is_newer_version("1.0.0", "0.9.9"));
        assert!(is_newer_version("0.2", "0.1.9"));
        // 缺失段补零、字符串前缀比较被避免（0.1.0 不小于 0.1.0-rc 视为相同段）。
        assert!(!is_newer_version("0.1.0", "0.1.0"));
        assert!(!is_newer_version("0.1", "0.1.0"));
        assert!(!is_newer_version("0.0.9", "0.1.0"));
        // 预发布后缀只取数字主体：v0.2.0-rc1 视作 0.2.0。
        assert!(is_newer_version("0.2.0-rc1", "0.1.9"));
        assert!(!is_newer_version("0.1.0-rc1", "0.1.0"));
    }
}
