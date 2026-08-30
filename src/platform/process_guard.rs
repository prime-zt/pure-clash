//! 内核子进程的平台守护与终止策略。
//!
//! 各平台提供同名 [`KernelProcessGuard`]，约定如下：
//! - [`KernelProcessGuard::new`]：创建守护资源；Windows 在此创建 Job Object，
//!   Linux/macOS 无状态直接返回。
//! - [`KernelProcessGuard::prepare_command`]：spawn 前配置命令的平台属性
//!   （Windows 不弹控制台；unix 独立进程组，Linux 附加父进程死亡守护）。
//!   校验等短命子进程同样适用。
//! - [`KernelProcessGuard::attach`]：spawn 后挂接守护；失败时调用方必须终止
//!   子进程，不得留下不受应用生命周期管理的后台进程。
//! - [`KernelProcessGuard::terminate`]：平台终止策略；Windows 直接
//!   TerminateProcess，unix 先 SIGTERM 最多 5 秒再升级 SIGKILL。
//! - `Drop`：释放守护资源；Windows Job 句柄必须最后关闭，兜底异常退出回收。
//!
//! 通过该抽象，`mihomo::process` 的内核生命周期语义保持平台无关。

#[cfg(unix)]
use anyhow::{Context, Result};

#[cfg(target_os = "linux")]
pub(crate) use crate::platform::linux::process_guard::KernelProcessGuard;
#[cfg(target_os = "windows")]
pub(crate) use crate::platform::windows::process_guard::KernelProcessGuard;
#[cfg(target_os = "macos")]
pub(crate) use macos_guard::KernelProcessGuard;

/// unix 共享的优雅终止流程：SIGTERM 给内核清理机会，最多等待 5 秒后升级 SIGKILL。
#[cfg(unix)]
pub(crate) fn terminate_unix(child: &mut std::process::Child) -> Result<()> {
    use std::time::{Duration, Instant};

    if let Err(error) = send_sigterm(child.id()) {
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(error).context("无法向 Mihomo 进程发送终止信号");
        }
        // 信号目标已不存在：进程在检查后恰好自行退出，交由调用方 wait 回收。
        return Ok(());
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        if child
            .try_wait()
            .context("无法读取 Mihomo 退出状态")?
            .is_some()
        {
            return Ok(());
        }
    }

    // 优雅超时，升级为无差别终止；退出竞态交由调用方 wait 收敛。
    if let Err(error) = child.kill() {
        if child.try_wait().ok().flatten().is_some() {
            return Ok(());
        }
        return Err(error).context("无法终止 Mihomo 进程");
    }
    Ok(())
}

/// 向进程发送 SIGTERM；Mihomo 会响应信号完成清理后退出。
#[cfg(unix)]
fn send_sigterm(pid: u32) -> std::io::Result<()> {
    let result = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// macOS 预留的内核守护：内核文件与生命周期尚未验证，仅保留与 Linux 一致的
/// 进程组隔离和优雅终止；异常退出兑底需在正式支持时单独实现。
#[cfg(target_os = "macos")]
mod macos_guard {
    use std::process::{Child, Command};

    use anyhow::Result;

    use super::terminate_unix;

    pub(crate) struct KernelProcessGuard;

    impl KernelProcessGuard {
        pub(crate) fn new() -> Result<Self> {
            Ok(Self)
        }

        pub(crate) fn prepare_command(command: &mut Command) {
            use std::os::unix::process::CommandExt;

            command.process_group(0);
        }

        pub(crate) fn attach(&mut self, _child: &Child) -> Result<()> {
            Ok(())
        }

        pub(crate) fn terminate(child: &mut Child) -> Result<()> {
            terminate_unix(child)
        }
    }
}
