//! Windows 内核提权启动：TUN 需要管理员权限，经 UAC 弹窗获得用户显式授权。
//!
//! 应用本体保持普通权限运行（不静默提权）；只有启用 TUN 需要拉起内核时
//! 才通过 `runas` 动词触发 UAC，用户拒绝或取消即启动失败并回退。

use std::{os::windows::ffi::OsStrExt, path::Path};

use anyhow::{Context, Result, anyhow, ensure};
use windows_sys::Win32::Foundation::{CloseHandle, S_FALSE, S_OK, WAIT_TIMEOUT};
use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessId, TerminateProcess, WaitForSingleObject,
};
use windows_sys::Win32::UI::Shell::{
    SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SEE_MASK_UNICODE, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

/// 提权助手进程；助手用独立 Job 守护内核，终止助手也会回收内核。
/// 持有原始 HANDLE 的数值形式，Drop 时关闭句柄。
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
        let running = unsafe { WaitForSingleObject(self.handle as _, 0) == WAIT_TIMEOUT };
        if !running {
            let mut code = 0;
            if unsafe { GetExitCodeProcess(self.handle as _, &mut code) } != 0 {
                log_warn!(
                    "kernel",
                    "提权助手已退出（pid {}，退出码 0x{code:08X}），详细输出见 log/tun-kernel.log",
                    self.pid
                );
            }
        }
        running
    }

    /// 终止进程并等待内核真正退出，避免句柄与资源清理竞态。
    pub(crate) fn terminate(&self) -> Result<()> {
        log_info!(
            "kernel",
            "回收提权助手及其内核（helper pid {}），诊断输出保留在 log/tun-kernel.log",
            self.pid
        );
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
    // ShellExecute 无法直接继承 stdout/stderr 管道，改为提权当前程序的专用
    // 助手模式。只传版本目录名，避免拼接任意路径/命令；助手自行解析固定路径。
    let paths = crate::platform::AppPaths::from_current_exe()?;
    let version = exe
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .context("无法从内核路径解析版本")?;
    let expected_exe = crate::kernel::bundled_path(&paths, version)?;
    ensure!(
        expected_exe == exe
            && paths.mihomo_data_dir == data_dir
            && paths.runtime_mihomo_config_file == config_file,
        "提权助手只允许使用当前安装的内核与运行配置"
    );
    let args = format!("{} {version}", super::elevated_helper::HELPER_ARG);
    let helper = std::env::current_exe()?;
    let verb = wide("runas");
    let file: Vec<u16> = helper.as_os_str().encode_wide().chain(Some(0)).collect();
    let parameters = wide(&args);
    let directory: Vec<u16> = data_dir.as_os_str().encode_wide().chain(Some(0)).collect();
    log_info!(
        "kernel",
        "发起 UAC 提权助手（内核版本 {version}），stdout/stderr 将逐行写入 log/tun-kernel.log"
    );
    let started = std::time::Instant::now();
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
    log_info!(
        "kernel",
        "UAC 提权助手已创建（helper pid {pid}，授权/启动耗时 {}ms）",
        started.elapsed().as_millis()
    );
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
