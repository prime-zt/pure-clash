use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};

pub(crate) const ICON_APP: &str = "icons/app.svg";
pub(crate) const ICON_SEARCH: &str = "icons/search.svg";
pub(crate) const ICON_FOLDER: &str = "icons/folder.svg";
pub(crate) const ICON_LAYOUT: &str = "icons/layout-dashboard.svg";
pub(crate) const ICON_CHART: &str = "icons/chart-no-axes-combined.svg";
pub(crate) const ICON_MESSAGE: &str = "icons/message-square.svg";
pub(crate) const ICON_PLUS: &str = "icons/plus.svg";
pub(crate) const ICON_SETTINGS: &str = "icons/settings.svg";
pub(crate) const ICON_INFO: &str = "icons/info.svg";
pub(crate) const ICON_EYE: &str = "icons/eye.svg";
pub(crate) const ICON_EYE_OFF: &str = "icons/eye-off.svg";
pub(crate) const ICON_CHEVRON_DOWN: &str = "icons/chevron-down.svg";
pub(crate) const ICON_CHEVRON_RIGHT: &str = "icons/chevron-right.svg";
pub(crate) const ICON_MORE: &str = "icons/more-horizontal.svg";
pub(crate) const ICON_PANEL_LEFT: &str = "icons/panel-left.svg";
pub(crate) const ICON_PANEL_RIGHT: &str = "icons/panel-right.svg";
pub(crate) const ICON_GIT_BRANCH: &str = "icons/git-branch.svg";
pub(crate) const ICON_TERMINAL: &str = "icons/terminal.svg";
pub(crate) const ICON_FILE_CODE: &str = "icons/file-code.svg";
pub(crate) const ICON_SHIELD_CHECK: &str = "icons/shield-check.svg";
pub(crate) const ICON_CIRCLE_CHECK: &str = "icons/circle-check.svg";
pub(crate) const ICON_PAPERCLIP: &str = "icons/paperclip.svg";
pub(crate) const ICON_SEND: &str = "icons/send.svg";
pub(crate) const ICON_STOP: &str = "icons/square.svg";
pub(crate) const ICON_REFRESH_CW: &str = "icons/refresh-cw.svg";
pub(crate) const ICON_SUN: &str = "icons/sun.svg";
pub(crate) const ICON_MOON: &str = "icons/moon.svg";
pub(crate) const ICON_WINDOW_MINIMIZE: &str = "icons/window-minimize.svg";
pub(crate) const ICON_WINDOW_MAXIMIZE: &str = "icons/window-maximize.svg";
pub(crate) const ICON_WINDOW_CLOSE: &str = "icons/window-close.svg";

/// 将 SVG 图标编译进可执行文件，安装后不需要额外资源目录。
pub(crate) struct Assets;

impl Assets {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let bytes: &'static [u8] = match path {
            ICON_APP => include_bytes!("../assets/icons/app.svg"),
            ICON_SEARCH => include_bytes!("../assets/icons/search.svg"),
            ICON_FOLDER => include_bytes!("../assets/icons/folder.svg"),
            ICON_LAYOUT => include_bytes!("../assets/icons/layout-dashboard.svg"),
            ICON_CHART => include_bytes!("../assets/icons/chart-no-axes-combined.svg"),
            ICON_MESSAGE => include_bytes!("../assets/icons/message-square.svg"),
            ICON_PLUS => include_bytes!("../assets/icons/plus.svg"),
            ICON_SETTINGS => include_bytes!("../assets/icons/settings.svg"),
            ICON_INFO => include_bytes!("../assets/icons/info.svg"),
            ICON_EYE => include_bytes!("../assets/icons/eye.svg"),
            ICON_EYE_OFF => include_bytes!("../assets/icons/eye-off.svg"),
            ICON_CHEVRON_DOWN => include_bytes!("../assets/icons/chevron-down.svg"),
            ICON_CHEVRON_RIGHT => include_bytes!("../assets/icons/chevron-right.svg"),
            ICON_MORE => include_bytes!("../assets/icons/more-horizontal.svg"),
            ICON_PANEL_LEFT => include_bytes!("../assets/icons/panel-left.svg"),
            ICON_PANEL_RIGHT => include_bytes!("../assets/icons/panel-right.svg"),
            ICON_GIT_BRANCH => include_bytes!("../assets/icons/git-branch.svg"),
            ICON_TERMINAL => include_bytes!("../assets/icons/terminal.svg"),
            ICON_FILE_CODE => include_bytes!("../assets/icons/file-code.svg"),
            ICON_SHIELD_CHECK => include_bytes!("../assets/icons/shield-check.svg"),
            ICON_CIRCLE_CHECK => include_bytes!("../assets/icons/circle-check.svg"),
            ICON_PAPERCLIP => include_bytes!("../assets/icons/paperclip.svg"),
            ICON_SEND => include_bytes!("../assets/icons/send.svg"),
            ICON_STOP => include_bytes!("../assets/icons/square.svg"),
            ICON_REFRESH_CW => include_bytes!("../assets/icons/refresh-cw.svg"),
            ICON_SUN => include_bytes!("../assets/icons/sun.svg"),
            ICON_MOON => include_bytes!("../assets/icons/moon.svg"),
            ICON_WINDOW_MINIMIZE => include_bytes!("../assets/icons/window-minimize.svg"),
            ICON_WINDOW_MAXIMIZE => include_bytes!("../assets/icons/window-maximize.svg"),
            ICON_WINDOW_CLOSE => include_bytes!("../assets/icons/window-close.svg"),
            _ => return Ok(None),
        };
        Ok(Some(Cow::Borrowed(bytes)))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(vec![
            ICON_APP.into(),
            ICON_SEARCH.into(),
            ICON_FOLDER.into(),
            ICON_LAYOUT.into(),
            ICON_CHART.into(),
            ICON_MESSAGE.into(),
            ICON_PLUS.into(),
            ICON_SETTINGS.into(),
            ICON_INFO.into(),
            ICON_EYE.into(),
            ICON_EYE_OFF.into(),
            ICON_CHEVRON_DOWN.into(),
            ICON_CHEVRON_RIGHT.into(),
            ICON_MORE.into(),
            ICON_PANEL_LEFT.into(),
            ICON_PANEL_RIGHT.into(),
            ICON_GIT_BRANCH.into(),
            ICON_TERMINAL.into(),
            ICON_FILE_CODE.into(),
            ICON_SHIELD_CHECK.into(),
            ICON_CIRCLE_CHECK.into(),
            ICON_PAPERCLIP.into(),
            ICON_SEND.into(),
            ICON_STOP.into(),
            ICON_REFRESH_CW.into(),
            ICON_SUN.into(),
            ICON_MOON.into(),
            ICON_WINDOW_MINIMIZE.into(),
            ICON_WINDOW_MAXIMIZE.into(),
            ICON_WINDOW_CLOSE.into(),
        ])
    }
}
