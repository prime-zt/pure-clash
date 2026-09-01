//! Windows 内核提权启动：TUN 需要管理员权限，经 UAC 弹窗获得用户显式授权。
//!
//! 应用本体保持普通权限运行（不静默提权）；只有启用 TUN 需要拉起内核时
//! 才通过 `runas` 动词触发 UAC，用户拒绝或取消即启动失败并回退。

use std::path::Path;

use anyhow::{Result, anyhow};
use windows_sys::Win32::Foundation::{CloseHandle, S_FALSE, S_OK, WAIT_TIMEOUT};
use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows_sys::Win32::System::Threading::{GetProcessId, TerminateProcess, WaitForSingleObject};
use windows_sys::Win32::UI::Shell::{
    SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SEE_MASK_UNICODE, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

/// 提权启动的内核进程；持有原始 HANDLE 的数值形式，Drop 时关闭句柄。
#[derive(Debug)]
pub(crate) struct ElevatedProcess {
    handle: isize,
    pid: u32,
}

impl ElevatedProcess {
    /// 以原始句柄形式提供给 Job Object 与终止逻辑使用。
    pub(crate) fn handle(&self) -> isize {
        self.handle
    }

    /// 进程 ID；供运行日志记录。
    pub(crate) fn pid(&self) -> u32 {
        self.pid
    }

    /// 进程是否仍然存活；`WaitForSingleObject` 立即返回且未触发即存活。
    pub(crate) fn is_running(&self) -> bool {
        unsafe { WaitForSingleObject(self.handle as _, 0) == WAIT_TIMEOUT }
    }

    /// 终止进程并等待内核真正退出，避免句柄与资源清理竞态。
    pub(crate) fn terminate(&self) -> Result<()> {
        unsafe { TerminateProcess(self.handle as _, 1) };
        unsafe { WaitForSingleObject(self.handle as _, 5000) };
        Ok(())
    }
}

impl Drop for ElevatedProcess {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle as _) };
    }
}

/// 经 UAC 弹窗以管理员身份启动内核；用户取消 UAC 即返回错误。
pub(crate) fn launch_elevated(
    exe: &Path,
    data_dir: &Path,
    config_file: &Path,
    allow_interactive: bool,
) -> Result<ElevatedProcess> {
    if !allow_interactive {
        return Err(anyhow!("当前启动阶段禁止弹出 Windows UAC"));
    }
    // ShellExecuteExW 可能依赖 Shell 扩展和 COM；专用启动线程显式使用 STA，
    // 避免未初始化 COM 时出现不稳定的 UAC/文件关联行为。
    let _com = ComApartment::initialize_sta()?;
    let args = format!(
        "-d \"{}\" -f \"{}\"",
        data_dir.display(),
        config_file.display()
    );
    let verb = wide("runas");
    let file = wide(&exe.as_os_str().to_string_lossy());
    let parameters = wide(&args);
    let directory = wide(&data_dir.as_os_str().to_string_lossy());
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        // NOCLOSEPROCESS 让结构体带回内核句柄；NOASYNC 确保调用返回时已拿到结果。
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_UNICODE,
        lpVerb: verb.as_ptr(),
        lpFile: file.as_ptr(),
        lpParameters: parameters.as_ptr(),
        lpDirectory: directory.as_ptr(),
        nShow: SW_HIDE,
        ..Default::default()
    };
    let launched = unsafe { ShellExecuteExW(&mut info) };
    if launched == 0 || info.hProcess.is_null() {
        return Err(anyhow!(
            "内核提权启动被取消或失败（最后系统错误 {}）",
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
        ));
    }
    let pid = unsafe { GetProcessId(info.hProcess) };
    Ok(ElevatedProcess {
        handle: info.hProcess as isize,
        pid,
    })
}

/// 当前线程的 COM STA 生命周期；每次成功初始化都必须配对 CoUninitialize。
struct ComApartment;

impl ComApartment {
    fn initialize_sta() -> Result<Self> {
        let result = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
        if result != S_OK && result != S_FALSE {
            return Err(anyhow!(
                "无法初始化 Windows Shell STA（HRESULT 0x{result:08X}）"
            ));
        }
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

/// UTF-16 编码并以 NUL 结尾的 Windows 字符串。
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
