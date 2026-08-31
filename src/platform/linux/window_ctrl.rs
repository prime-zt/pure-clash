use gpui::Window;

/// 显示并恢复主窗口：X11 通过 EWMH 激活可取消最小化；Wayland 依赖 xdg_activation，
/// 部分合成器（如 GNOME Mutter）可能拒绝托盘来源的自激活请求，此时需从概览手动恢复。
pub(crate) fn show_main_window(window: &Window) {
    window.activate_window();
}
