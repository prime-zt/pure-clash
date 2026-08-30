use std::os::windows::process::CommandExt;
use std::process::{Child, Command};

use anyhow::{Context, Result};
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

use super::job::JobObject;

/// Windows 内核进程守护：Job Object 启用 `KILL_ON_JOB_CLOSE`，
/// 主进程崩溃或被强制终止时由内核关闭句柄并清理 Mihomo。
pub(crate) struct KernelProcessGuard {
    job: JobObject,
}

impl KernelProcessGuard {
    /// 创建启用了 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` 的专用 Job Object。
    pub(crate) fn new() -> Result<Self> {
        let job = JobObject::new().context("无法创建 Mihomo Job Object")?;
        Ok(Self { job })
    }

    /// GUI 应用启动内核和校验进程时都不应弹出额外控制台窗口。
    pub(crate) fn prepare_command(command: &mut Command) {
        command.creation_flags(CREATE_NO_WINDOW);
    }

    /// 将已启动的内核子进程加入 Job Object。
    pub(crate) fn attach(&mut self, child: &Child) -> Result<()> {
        self.job
            .assign(child)
            .context("无法将 Mihomo 加入 Job Object")
    }

    /// 提权启动的内核经 ShellExecute 返回的原生句柄加入 Job Object。
    pub(crate) fn attach_handle(&mut self, process_handle: isize) -> Result<()> {
        self.job
            .assign_handle(process_handle as _)
            .context("无法将提权的 Mihomo 加入 Job Object")
    }

    /// Windows 的 TerminateProcess 本就无差别终止；处理退出竞态后返回。
    pub(crate) fn terminate(child: &mut Child) -> Result<()> {
        if let Err(error) = child.kill() {
            // 处理检查状态与终止之间进程自行退出的竞态。
            if child.try_wait().ok().flatten().is_some() {
                return Ok(());
            }
            return Err(error).context("无法终止 Mihomo 进程");
        }
        Ok(())
    }
}
