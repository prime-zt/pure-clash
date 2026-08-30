use gpui::Window;

/// Linux 没有跨合成器的“隐藏窗口”协议；关闭到托盘时最小化到概览是当前等价视觉。
/// 窗口实体、GPUI 状态、托盘和内核子进程全部继续存活。
pub(crate) fn hide_main_window(window: &Window) {
    window.minimize_window();
}

/// 显示并恢复主窗口：X11 通过 EWMH 激活可取消最小化；Wayland 依赖 xdg_activation，
/// 部分合成器（如 GNOME Mutter）可能拒绝托盘来源的自激活请求，此时需从概览手动恢复。
pub(crate) fn show_main_window(window: &Window) {
    window.activate_window();
}
