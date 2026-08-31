//! 应用启动入口的模式解析。

use std::ffi::OsString;

const AUTOSTART_ARG: &str = "--autostart";

/// 区分用户主动打开与桌面登录自启，决定是否创建初始主窗口及通知已有实例。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StartupMode {
    /// 用户主动启动：显示主窗口，若已有实例则请求其恢复窗口。
    Interactive,
    /// 系统登录自启：只启动常驻业务与托盘，若已有实例则静默退出。
    Autostart,
}

impl StartupMode {
    pub(crate) fn from_env() -> Self {
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        {
            Self::from_args(std::env::args_os())
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            // macOS 尚无托盘与登录项实现，不能进入无法恢复的零窗口模式。
            Self::Interactive
        }
    }

    fn from_args(args: impl IntoIterator<Item = OsString>) -> Self {
        if args
            .into_iter()
            .skip(1)
            .any(|argument| argument == AUTOSTART_ARG)
        {
            Self::Autostart
        } else {
            Self::Interactive
        }
    }

    /// 只有用户主动启动才立即创建窗口；后台自启等待托盘或第二实例唤起。
    pub(crate) fn show_initial_window(self) -> bool {
        self == Self::Interactive
    }

    /// 后台自启不能干扰已经运行的实例，尤其不能在登录时意外弹出窗口。
    pub(crate) fn notify_existing_instance(self) -> bool {
        self == Self::Interactive
    }

    pub(crate) fn is_autostart(self) -> bool {
        self == Self::Autostart
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_autostart_flag_without_affecting_normal_launches() {
        assert_eq!(
            StartupMode::from_args(["pure-clash".into(), AUTOSTART_ARG.into()]),
            StartupMode::Autostart
        );
        assert_eq!(
            StartupMode::from_args(["pure-clash".into()]),
            StartupMode::Interactive
        );
    }
}
