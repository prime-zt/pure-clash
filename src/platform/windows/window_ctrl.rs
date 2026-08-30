use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{IsIconic, SW_HIDE, SW_RESTORE, SW_SHOW, ShowWindowAsync},
};

/// 从 GPUI 窗口解析原始 Win32 HWND；非 Win32 后端返回 `None`。
fn hwnd_of_window(window: &Window) -> Option<HWND> {
    // GPUI 的固有 `Window::window_handle` 返回实体句柄，这里必须显式调用 trait 方法。
    let window_handle = HasWindowHandle::window_handle(window).ok()?;
    match window_handle.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as HWND),
        _ => None,
    }
}

/// 隐藏主窗口：窗口不销毁，GPUI 实体、托盘和内核子进程继续存活。
pub(crate) fn hide_main_window(window: &Window) {
    let Some(hwnd) = hwnd_of_window(window) else {
        return;
    };
    // SAFETY: `hwnd` 是当前 GPUI 窗口的有效句柄，`SW_HIDE` 只改变可见性。
    unsafe { ShowWindowAsync(hwnd, SW_HIDE) };
}

/// 显示并恢复主窗口，再交给 GPUI 置前。
///
/// GPUI 的 `activate_window` 只恢复最小化窗口；对 `SW_HIDE` 隐藏的窗口必须先显式 `SW_SHOW`。
pub(crate) fn show_main_window(window: &Window) {
    let Some(hwnd) = hwnd_of_window(window) else {
        return;
    };
    // SAFETY: `hwnd` 是当前 GPUI 窗口的有效句柄，先恢复最小化再按原尺寸显示。
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindowAsync(hwnd, SW_RESTORE);
        } else {
            ShowWindowAsync(hwnd, SW_SHOW);
        }
    }
    window.activate_window();
}
