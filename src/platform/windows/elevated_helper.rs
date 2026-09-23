//! UAC 提权助手：只启动当前安装的内核，并从第一行开始捕获 stdout/stderr。
//! 独立 Job Object 保证助手被主进程回收或崩溃时，内核不会变成孤儿进程。

use std::{
    io::{BufRead, BufReader, Read},
    process::{Command, Stdio},
    sync::Arc,
    thread,
};

use anyhow::{Context, Result, bail};
use windows_sys::Win32::System::Registry::{
    HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
};

use crate::{
    kernel,
    logging::TunLogWriter,
    platform::{AppPaths, KernelProcessGuard},
};

pub(super) const HELPER_ARG: &str = "--run-elevated-kernel";

/// 内部 CLI：唯一参数是安全的内核版本目录名；路径均从当前 EXE 推导，
/// 不接受任意命令、日志路径或配置路径。返回 None 表示普通启动，否则为退出码。
pub(crate) fn run_if_requested() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(HELPER_ARG)) {
        return None;
    }
    let paths = match AppPaths::from_current_exe() {
        Ok(paths) => paths,
        Err(_) => return Some(1),
    };
    // 不初始化 app.log，避免助手抢占主程序的会话日志与单实例锁。
    let writer = TunLogWriter::open(&paths.log_dir);
    let result = (|| {
        let version = args.next().context("提权助手缺少内核版本")?;
        let version = version.to_str().context("内核版本不是有效文本")?;
        if args.next().is_some() {
            bail!("提权助手参数数量无效");
        }
        let executable = kernel::bundled_path(&paths, version)?;
        write(
            &writer,
            &format!(
                "Pure Clash {} TUN 诊断助手启动，helper pid={}，内核版本={version}",
                env!("CARGO_PKG_VERSION"),
                std::process::id()
            ),
        );
        log_environment(&writer);
        let mut command = Command::new(executable);
        command
            .arg("-d")
            .arg(&paths.mihomo_data_dir)
            .arg("-f")
            .arg(&paths.runtime_mihomo_config_file)
            .current_dir(&paths.mihomo_data_dir);
        capture_kernel(command, writer.clone())
    })();
    Some(match result {
        Ok(code) => code,
        Err(error) => {
            write(&writer, &format!("提权助手失败：{error:#}"));
            1
        }
    })
}

/// 只记录系统版本和 IPv6 全局策略，不枚举用户名、网卡地址或订阅内容。
fn log_environment(writer: &Option<Arc<TunLogWriter>>) {
    let version_key = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    for name in ["ProductName", "DisplayVersion", "CurrentBuildNumber", "UBR"] {
        write(
            writer,
            &format!(
                "Windows {name}={}",
                registry_value(version_key, name, name == "UBR")
            ),
        );
    }
    write(
        writer,
        &format!(
            "IPv6 DisabledComponents={}（缺失表示系统默认值 0）",
            registry_value(
                r"SYSTEM\CurrentControlSet\Services\Tcpip6\Parameters",
                "DisabledComponents",
                true
            )
        ),
    );
}

/// 固定白名单注册值的只读采样；查询失败记录 Win32 状态，不影响内核启动。
fn registry_value(key: &str, name: &str, dword: bool) -> String {
    let key: Vec<u16> = key.encode_utf16().chain(Some(0)).collect();
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let mut data = [0u32; 128];
    let mut size = std::mem::size_of_val(&data) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            name.as_ptr(),
            if dword {
                RRF_RT_REG_DWORD
            } else {
                RRF_RT_REG_SZ
            },
            std::ptr::null_mut(),
            data.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != 0 {
        return format!("未读取到（Win32={status}）");
    }
    if dword {
        format!("{} (0x{:08X})", data[0], data[0])
    } else {
        let units: Vec<u16> = data
            .iter()
            .flat_map(|value| [*value as u16, (*value >> 16) as u16])
            .take(size as usize / 2)
            .take_while(|unit| *unit != 0)
            .collect();
        String::from_utf16_lossy(&units)
    }
}

fn write(writer: &Option<Arc<TunLogWriter>>, message: &str) {
    if let Some(writer) = writer {
        writer.write_line("helper", message);
    }
}

/// 助手持有子进程守护直到退出，并排空两条输出管道；日志失败仍持续读取。
fn capture_kernel(mut command: Command, writer: Option<Arc<TunLogWriter>>) -> Result<i32> {
    let mut guard = KernelProcessGuard::new()?;
    KernelProcessGuard::prepare_command(&mut command);
    let started = std::time::Instant::now();
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("无法创建提权内核子进程")?;
    if let Err(error) = guard.attach(&child) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    write(
        &writer,
        &format!(
            "内核已创建，kernel pid={}，开始捕获 stdout/stderr",
            child.id()
        ),
    );
    let stdout = child.stdout.take().context("内核 stdout 管道缺失")?;
    let stderr = child.stderr.take().context("内核 stderr 管道缺失")?;
    let out = pump(stdout, "stdout", writer.clone())?;
    let err = pump(stderr, "stderr", writer.clone())?;
    let status = child.wait().context("等待提权内核退出失败")?;
    let _ = out.join();
    let _ = err.join();
    write(
        &writer,
        &format!(
            "内核退出：{status}，运行 {}ms",
            started.elapsed().as_millis()
        ),
    );
    Ok(status.code().unwrap_or(1))
}

/// 按行保留 stdout/stderr 来源；非 UTF-8 字节有损解码，避免一行乱码中断后续日志。
fn pump(
    stream: impl Read + Send + 'static,
    source: &'static str,
    writer: Option<Arc<TunLogWriter>>,
) -> Result<thread::JoinHandle<()>> {
    Ok(thread::Builder::new()
        .name(format!("tun-{source}"))
        .spawn(move || {
            let mut reader = BufReader::new(stream);
            let mut line = Vec::new();
            loop {
                line.clear();
                match reader.read_until(b'\n', &mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if let Some(writer) = &writer {
                            writer.write_line(
                                source,
                                String::from_utf8_lossy(&line).trim_end_matches(['\r', '\n']),
                            );
                        }
                    }
                    Err(error) => {
                        write(&writer, &format!("读取 {source} 失败：{error}"));
                        break;
                    }
                }
            }
        })
        .context("无法创建 TUN 日志读取线程")?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_both_streams_redacts_and_preserves_exit_code() {
        let root =
            std::env::temp_dir().join(format!("pure-clash-tun-log-{}", uuid::Uuid::new_v4()));
        let writer = TunLogWriter::open(&root).unwrap();
        // 仅运行系统 cmd 输出测试文本，不启动真实内核、不改变网络配置。
        let mut command = Command::new(std::env::var_os("COMSPEC").unwrap());
        command.args([
            "/D",
            "/C",
            "echo token=private-token & echo tun-init-failed 1>&2 & exit /b 7",
        ]);
        assert_eq!(capture_kernel(command, Some(writer.clone())).unwrap(), 7);
        let content = std::fs::read_to_string(root.join("tun-kernel.log")).unwrap();
        assert!(content.contains("[stdout] token=***"));
        assert!(content.contains("[stderr] tun-init-failed"));
        assert!(content.contains("内核退出"));
        assert!(!content.contains("private-token"));
        drop(writer);
        std::fs::remove_dir_all(root).unwrap();
    }
}
