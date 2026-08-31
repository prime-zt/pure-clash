use std::{
    fs,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};

use crate::{
    kernel,
    platform::{AppPaths, ElevatedProcess, KernelProcessGuard, launch_elevated},
};

/// 当前由 Pure Clash 管理的 Mihomo 子进程。
///
/// 平台差异（Windows Job Object、Linux pdeathsig、unix 优雅终止）全部收敛在
/// [`KernelProcessGuard`]；本类型只描述内核生命周期语义，不含任何平台分支。
pub(crate) struct MihomoProcess {
    child: KernelChild,
    /// 平台守护资源；不直接读取，仅为 Drop 语义持有，且声明在 `child` 之后，
    /// 保证 Drop 时守护资源最后释放（Windows Job 句柄关闭是异常退出回收的最后防线）。
    _guard: KernelProcessGuard,
}

/// 内核进程的实际承载：普通权限走标准子进程，TUN 等能力需要提权时走平台启动器。
enum KernelChild {
    Normal(Child),
    Elevated(ElevatedProcess),
}

impl KernelChild {
    /// 进程是否已经退出；状态读取失败按已退出处理，由调用方兜底终止。
    fn exited(&mut self) -> Result<bool> {
        match self {
            KernelChild::Normal(child) => Ok(child.try_wait()?.is_some()),
            KernelChild::Elevated(process) => Ok(!process.is_running()),
        }
    }

    fn terminate(&mut self) -> Result<()> {
        match self {
            KernelChild::Normal(child) => {
                KernelProcessGuard::terminate(child)?;
                // 终止路径结束后子进程必然已退出；重复回收时 std 会返回缓存状态。
                child.wait().context("无法回收 Mihomo 进程")?;
                Ok(())
            }
            KernelChild::Elevated(process) => process.terminate(),
        }
    }
}

impl MihomoProcess {
    /// 使用指定配置启动指定版本内核；启动前必须先通过同版本的 `-t` 校验。
    ///
    /// `elevated` 服务于 TUN 等需要系统网络权限的能力：Windows 经 UAC，
    /// Linux 经一次 polkit 授权安装的 root 服务；用户拒绝授权即返回错误。
    /// `allow_interactive_elevation` 由启动场景决定是否允许 UAC/polkit；禁止时平台
    /// 授权不可静默完成就直接返回错误，由上层关闭 TUN 并回退普通内核。
    pub(crate) fn start(
        paths: &AppPaths,
        version: &str,
        config_file: &Path,
        elevated: bool,
        allow_interactive_elevation: bool,
    ) -> Result<Self> {
        let executable = kernel::bundled_path(paths, version)
            .with_context(|| format!("Mihomo 版本目录无效：{version}"))?;
        if !executable.is_file() {
            bail!("Mihomo 内核文件不存在：{}", executable.display());
        }
        if !config_file.is_file() {
            bail!("内核配置文件不存在：{}", config_file.display());
        }
        fs::create_dir_all(&paths.mihomo_data_dir).with_context(|| {
            format!(
                "无法创建 Mihomo 数据目录：{}",
                paths.mihomo_data_dir.display()
            )
        })?;

        validate_config(&executable, paths, config_file)?;

        let mut guard = KernelProcessGuard::new()?;
        let mut child = if elevated {
            let process = launch_elevated(
                &executable,
                &paths.mihomo_data_dir,
                config_file,
                allow_interactive_elevation,
            )
            .context("无法以所需系统权限启动 Mihomo 内核")?;
            if let Err(error) = guard.attach_handle(process.handle()) {
                // 提权进程已经创建，守护挂接失败时必须主动终止，不能只关闭句柄。
                let _ = process.terminate();
                return Err(error).context("无法为提权的内核进程挂接平台守护");
            }
            KernelChild::Elevated(process)
        } else {
            let mut command = mihomo_command(&executable, paths, config_file);
            command.stdout(Stdio::null()).stderr(Stdio::null());
            let mut child = command
                .spawn()
                .with_context(|| format!("无法启动 Mihomo 内核：{}", executable.display()))?;

            if let Err(error) = guard.attach(&child) {
                // 挂接守护失败时不得留下不受应用生命周期管理的后台进程。
                let _ = child.kill();
                let _ = child.wait();
                return Err(error).context("无法为内核进程挂接平台守护");
            }
            KernelChild::Normal(child)
        };

        // 配置校验无法发现端口占用等运行期错误，短暂确认进程没有立即退出。
        thread::sleep(Duration::from_millis(150));
        if child.exited().context("无法读取 Mihomo 启动状态")? {
            let _ = child.terminate();
            bail!("Mihomo 启动后立即退出（提权被拒绝或配置存在运行期错误）");
        }

        Ok(Self {
            child,
            _guard: guard,
        })
    }

    /// 终止并回收当前 Mihomo 子进程；进程已经退出时同样视为成功。
    pub(crate) fn stop(&mut self) -> Result<()> {
        // 无法读取状态时仍尝试终止，优先保证后台进程不会残留。
        if self.child.exited().unwrap_or(true) {
            return Ok(());
        }
        self.child.terminate()
    }

    /// 非阻塞读取受管进程是否仍存活；查询失败按已退出处理，让上层及时撤销
    /// 指向失效内核的系统集成状态。
    pub(crate) fn is_running(&mut self) -> bool {
        !self.child.exited().unwrap_or(true)
    }
}

impl Drop for MihomoProcess {
    fn drop(&mut self) {
        // 正常关闭窗口和状态销毁时主动回收；异常退出由平台守护兜底
        // （Windows Job Object / Linux pdeathsig）。
        let _ = self.stop();
    }
}

pub(crate) fn validate_config(
    executable: &std::path::Path,
    paths: &AppPaths,
    config_file: &Path,
) -> Result<()> {
    super::geodata::prepare_for_config(paths, config_file)?;

    let mut command = mihomo_command(executable, paths, config_file);
    command
        .arg("-t")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = command
        .output()
        .with_context(|| format!("无法执行 Mihomo 配置校验：{}", executable.display()))?;
    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = validation_error_detail(&stdout, &stderr, 2_048);
    bail!("Mihomo 配置校验失败：{detail}")
}

fn mihomo_command(executable: &std::path::Path, paths: &AppPaths, config_file: &Path) -> Command {
    let mut command = Command::new(executable);
    // 参数逐项传递，路径不经过 shell 解析，能够安全处理空格和非 ASCII 字符。
    command
        .arg("-d")
        .arg(&paths.mihomo_data_dir)
        .arg("-f")
        .arg(config_file)
        .current_dir(&paths.mihomo_data_dir)
        .stdin(Stdio::null());
    KernelProcessGuard::prepare_command(&mut command);
    command
}

fn truncate_output(output: &str, max_chars: usize) -> String {
    let mut truncated: String = output.chars().take(max_chars).collect();
    if output.chars().count() > max_chars {
        truncated.push('…');
    }
    truncated
}

fn truncate_output_tail(output: &str, max_chars: usize) -> String {
    let count = output.chars().count();
    if count <= max_chars {
        return output.to_owned();
    }
    let mut truncated = String::from("…");
    truncated.extend(output.chars().skip(count - max_chars));
    truncated
}

/// Mihomo 会先输出大量 info，真正失败原因位于末尾的 `level=error` 日志。
/// 优先提取最后一条结构化错误；无法识别时保留输出尾部。
fn validation_error_detail(stdout: &str, stderr: &str, max_chars: usize) -> String {
    for output in [stderr, stdout] {
        if let Some(line) = output
            .lines()
            .rev()
            .find(|line| line.contains("level=error"))
        {
            let message = log_message(line).unwrap_or_else(|| line.trim().to_owned());
            return truncate_output(&message, max_chars);
        }
    }

    let detail = match (stdout.trim(), stderr.trim()) {
        ("", "") => "Mihomo 未返回错误详情".to_owned(),
        (stdout, "") => stdout.to_owned(),
        ("", stderr) => stderr.to_owned(),
        (stdout, stderr) => format!("{stdout}\n{stderr}"),
    };
    truncate_output_tail(&detail, max_chars)
}

fn log_message(line: &str) -> Option<String> {
    let value = line.split_once("msg=")?.1.trim_start();
    if !value.starts_with('"') {
        return value.split_whitespace().next().map(str::to_owned);
    }

    // logrus 的双引号消息通常兼容 JSON 字符串；逐字符寻找结束引号，避免消息
    // 中的转义引号提前截断。若出现 Go 特有转义则回退到去引号原文。
    let mut escaped = false;
    for (offset, character) in value[1..].char_indices() {
        if character == '"' && !escaped {
            let quoted = &value[..offset + 2];
            return serde_json::from_str(quoted)
                .ok()
                .or_else(|| Some(quoted.trim_matches('"').to_owned()));
        }
        escaped = character == '\\' && !escaped;
        if character != '\\' {
            escaped = false;
        }
    }
    Some(value.trim_matches('"').to_owned())
}

#[cfg(test)]
mod tests {
    #[cfg(any(target_os = "windows", unix))]
    use std::{
        net::TcpListener,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[test]
    fn managed_process_can_return_from_platform_start_thread() {
        fn assert_send<T: Send>() {}
        assert_send::<MihomoProcess>();
    }

    #[test]
    fn truncates_validation_output_at_character_boundary() {
        assert_eq!(truncate_output("配置错误", 2), "配置…");
        assert_eq!(truncate_output("short", 10), "short");
    }

    #[test]
    fn validation_error_prefers_last_structured_error() {
        let stdout = "time=1 level=info msg=\"加载配置\"\n\
time=2 level=info msg=\"下载 GeoSite.dat\"\n\
time=3 level=error msg=\"rules[0] error: can't download GeoSite.dat\"\n";
        assert_eq!(
            validation_error_detail(stdout, "", 2_048),
            "rules[0] error: can't download GeoSite.dat"
        );
    }

    #[test]
    fn validation_error_falls_back_to_output_tail() {
        assert_eq!(
            validation_error_detail("first line\nfinal detail", "", 12),
            "…final detail"
        );
    }

    #[cfg(any(target_os = "windows", unix))]
    #[test]
    #[ignore = "使用随包 Mihomo 执行真实内核进程生命周期测试"]
    fn starts_and_stops_bundled_kernel() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时间应晚于 UNIX_EPOCH")
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("pure-clash-process-{}-{nonce}", std::process::id()));
        let mut paths = AppPaths::portable(&root);
        paths.kernel_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("kernel");
        fs::create_dir_all(&paths.mihomo_config_dir).expect("应创建测试配置目录");
        fs::create_dir_all(&paths.mihomo_data_dir).expect("应创建测试数据目录");

        // 先让系统分配空闲端口，再释放给 Mihomo，降低与本机代理软件冲突的概率。
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("应分配本机测试端口");
        let port = listener.local_addr().expect("应读取测试端口").port();
        drop(listener);
        fs::write(
            &paths.default_mihomo_config_file,
            format!(
                "mixed-port: {port}\nallow-lan: false\nbind-address: 127.0.0.1\nmode: direct\nlog-level: silent\n"
            ),
        )
        .expect("应写入测试配置");

        let mut process = MihomoProcess::start(
            &paths,
            env!("PURE_CLASH_DEFAULT_MIHOMO_VERSION"),
            &paths.default_mihomo_config_file,
            false,
            true,
        )
        .expect("应启动随包 Mihomo");
        process.stop().expect("应停止并回收随包 Mihomo");

        fs::remove_dir_all(root).expect("应清理测试目录");
    }
}
