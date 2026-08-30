use std::io;
use std::os::unix::process::CommandExt;
use std::process::Command;

/// 在 fork 与 exec 之间执行：设置父进程死亡信号并关闭竞争窗口。
///
/// Linux 没有跨平台的 Job Object；`PR_SET_PDEATHSIG` 让内核在父进程死亡
/// （包括被 SIGKILL 强杀）时收到 `SIGKILL`，保证 Pure Clash 异常退出后不
/// 残留后台 Mihomo 进程。
///
/// 已知语义限制：父进程死亡信号监视的是**创建子进程的那个线程**。当前
/// `start_core` 固定在 GPUI 主线程同步执行，主线程与应用同生命周期；后续
/// 若改为后台线程 spawn，必须使用长寿命线程，否则线程退出会误杀内核。
pub(crate) fn apply_parent_death_guard(command: &mut Command) {
    // 闭包捕获创建时刻的父进程 PID，用于识别“fork 之后、设置信号之前父进程
    // 已经退出”的竞争窗口：此时 getppid 不再是创建者，直接让 spawn 失败，
    // 避免留下收养后无人管理的孤儿进程。
    let parent_pid = std::process::id();
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::getppid() as u32 != parent_pid {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "Pure Clash 父进程在内核启动前已退出",
                ));
            }
            Ok(())
        });
    }
}
