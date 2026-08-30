/// 托盘向 GPUI 主线程发送的用户操作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrayAction {
    /// 单击托盘图标或选择菜单项，显示并激活应用主窗口。
    OpenMainWindow,
    /// 选择菜单项退出，真实结束应用并回收内核。
    Quit,
}

#[cfg(target_os = "linux")]
pub(crate) use crate::platform::linux::SystemTray;
/// 各平台提供同名 `SystemTray`：`new` / `set_tooltip` / `set_menu_texts`，
/// 实例存活期间托盘图标保持可见，释放时自动移除。
#[cfg(target_os = "windows")]
pub(crate) use crate::platform::windows::SystemTray;
