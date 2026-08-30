use std::os::unix::process::CommandExt;
use std::process::{Child, Command};

use anyhow::Result;

use super::child_guard::apply_parent_death_guard;
use crate::platform::process_guard::terminate_unix;

/// Linux 内核进程守护：`PR_SET_PDEATHSIG`（SIGKILL）等价 Windows Job Object，
/// 主进程死亡（包括被强杀）时由内核同步回收 Mihomo。
///
/// 守护在 `prepare_command` 的 `pre_exec` 阶段注入，因此本结构体无状态；
/// `attach` 无需额外挂接动作。
pub(crate) struct KernelProcessGuard;

impl KernelProcessGuard {
    pub(crate) fn new() -> Result<Self> {
        Ok(Self)
    }

    /// 内核独立成进程组：终端信号不再直达内核，启停节奏完全由 Pure Clash 控制；
    /// 同时注入父进程死亡守护，异常退出时不残留后台内核。
    pub(crate) fn prepare_command(command: &mut Command) {
        command.process_group(0);
        apply_parent_death_guard(command);
    }

    /// pdeathsig 已在 exec 前生效，spawn 后无需再挂接。
    pub(crate) fn attach(&mut self, _child: &Child) -> Result<()> {
        Ok(())
    }

    /// Linux 提权进程由 pkexec/内部助手在 exec 前设置 pdeathsig，PID 仅用于
    /// 保持与 Windows 相同的守护接口形状。
    pub(crate) fn attach_handle(&mut self, _process_handle: isize) -> Result<()> {
        Ok(())
    }

    /// 先 SIGTERM 给内核清理机会，最多 5 秒后升级 SIGKILL。
    pub(crate) fn terminate(child: &mut Child) -> Result<()> {
        terminate_unix(child)
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn terminates_running_process_gracefully() {
        let mut child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("应启动测试进程");
        let started = Instant::now();

        KernelProcessGuard::terminate(&mut child).expect("应优雅终止测试进程");
        child.wait().expect("应回收测试进程");

        // SIGTERM 立即生效；若走到 SIGKILL 超时路径则会接近 5 秒。
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "终止应在优雅超时前完成"
        );
    }
}
