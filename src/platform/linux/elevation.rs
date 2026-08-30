//! Linux TUN 服务客户端。
//!
//! 首次启用 TUN 时经 `pkexec` 安装 root 所有的 systemd 服务与 Mihomo 副本；
//! 后续启停通过仅授权当前 UID 的 Unix socket 完成，不再重复请求系统密码。
//! 服务参考 Clash Verge Rev，把配置物化到受保护目录并以 root 运行 Mihomo。

use std::path::Path;

use anyhow::Result;

use super::tun_service;

/// 由 Linux TUN 服务管理的 Mihomo 进程句柄。
#[derive(Debug)]
pub(crate) struct ElevatedProcess {
    pid: u32,
}

impl ElevatedProcess {
    pub(crate) fn handle(&self) -> isize {
        self.pid as isize
    }

    pub(crate) fn is_running(&mut self) -> bool {
        tun_service::is_core_running(self.pid)
    }

    pub(crate) fn terminate(&mut self) -> Result<()> {
        tun_service::stop_core(self.pid)
    }
}

/// 通过已安装的 Linux 服务启动 TUN 内核；服务缺失或协议不匹配时只在这里
/// 触发一次安装授权。
pub(crate) fn launch_elevated(
    executable: &Path,
    data_dir: &Path,
    config_file: &Path,
) -> Result<ElevatedProcess> {
    let pid = tun_service::start_core(executable, data_dir, config_file)?;
    Ok(ElevatedProcess { pid })
}

/// 在 GPUI 与单实例初始化前分流服务安装器和 systemd 服务模式。
pub(crate) fn run_elevated_helper_if_requested() -> Result<bool> {
    tun_service::run_internal_mode_if_requested()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_process_is_not_mistaken_for_internal_service_mode() {
        assert!(!tun_service::is_internal_mode(
            std::env::args_os().nth(1).as_deref()
        ));
    }
}
