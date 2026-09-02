//! 轻量文件日志：运行日志（app.log）与内核日志（kernel.log）分文件存放。
//!
//! 不引入日志框架：`OnceLock` + `Mutex` 维护全局写入器，GPUI 线程、内核启动
//! 线程与托盘回调线程共用；初始化失败时降级为空日志，绝不因日志故障阻断
//! 应用启动。所有消息写入前统一经 [`redact`] 脱敏，订阅 URL、认证头与
//! controller secret 不得进入任何日志文件（见 AGENTS.md 红线）。
//!
//! 磁盘占用有确定上界：每个文件超过阈值即轮转为 `.1`（覆盖旧备份），
//! app.log 1MB×2 + kernel.log 3MB×2 合计约 8MB，低于 10MB 预算。
//!
//! 写入路径按频率区别对待：app.log 低频且是崩溃诊断的依据，逐行落盘；
//! kernel.log 承接内核 info 级每条连接一行的输出，用缓冲批量落盘并把
//! 轮转判断降为字节计数，避免每行一次元数据查询加一次小系统调用。

use std::{
    borrow::Cow,
    fs::{self, File, OpenOptions},
    io::{BufWriter, LineWriter, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use jiff::fmt::strtime;

/// app.log 单文件轮转阈值。
const APP_LOG_MAX_BYTES: u64 = 1024 * 1024;
/// kernel.log 单文件轮转阈值。
const KERNEL_LOG_MAX_BYTES: u64 = 3 * 1024 * 1024;

/// 需要掩码的敏感键；大小写不敏感匹配，值为下一个空白或引号前的连续串。
const SENSITIVE_KEYS: [&str; 4] = ["secret=", "token=", "password=", "authorization:"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Level {
    Error,
    Warn,
    Info,
    Debug,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Level::Error => "ERROR",
            Level::Warn => "WARN",
            Level::Info => "INFO",
            Level::Debug => "DEBUG",
        }
    }
}

/// 全局运行日志写入器；`None` 表示未初始化或初始化失败，宏静默丢弃。
static APP_LOG: OnceLock<Mutex<Option<LogSink<LineWriter<File>>>>> = OnceLock::new();

/// 初始化运行日志；目录创建或文件打开失败时降级为空日志并输出到调试控制台。
pub(crate) fn init(log_dir: &Path) {
    let sink = open_sink(log_dir, "app.log", APP_LOG_MAX_BYTES, LineWriter::new);
    if sink.is_none() {
        eprintln!("无法初始化 Pure Clash 日志目录：{}", log_dir.display());
    }
    let _ = APP_LOG.set(Mutex::new(sink));
    install_panic_hook();
}

/// 写入一条运行日志；消息统一脱敏后落盘，debug 级仅在 debug 构建生效。
pub(crate) fn log(level: Level, tag: &str, message: &str) {
    if level == Level::Debug && !cfg!(debug_assertions) {
        return;
    }
    let Some(app_log) = APP_LOG.get() else {
        return;
    };
    let redacted = redact(message);
    let line = format!(
        "{} {} [{}] {}",
        timestamp(),
        level.as_str(),
        tag,
        &*redacted
    );
    // debug 构建镜像到控制台，保留开发时的即时可见性。
    #[cfg(debug_assertions)]
    eprintln!("{line}");
    let mut sink = match app_log.lock() {
        Ok(sink) => sink,
        // 毒锁只可能由写入 panic 造成；恢复数据继续写比丢弃日志更有价值。
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(sink) = sink.as_mut() {
        sink.write_line(&line);
    }
}

/// 记录 panic 位置与消息；保留默认 hook 的控制台输出行为。
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log(Level::Error, "panic", &info.to_string());
        previous(info);
    }));
}

/// 本地时区时间戳；与 Mihomo logrus 输出、journalctl 展示一致，便于对照排查。
fn timestamp() -> String {
    let now = jiff::Zoned::now();
    strtime::format("%Y-%m-%dT%H:%M:%S%.3f%:z", &now).unwrap_or_else(|_| now.to_string())
}

/// 打开一个日志文件；打开前先把上一段归档为 `.1`，因此 app.log 恒为本次
/// 会话、kernel.log 恒为当前内核的输出，备份合计占用有硬上界。
///
/// `new_writer` 决定冲刷策略：app.log 传 [`LineWriter::new`] 逐行落盘，
/// kernel.log 传 [`BufWriter::new`] 批量落盘。
fn open_sink<W: Write>(
    log_dir: &Path,
    name: &str,
    max_bytes: u64,
    new_writer: fn(File) -> W,
) -> Option<LogSink<W>> {
    fs::create_dir_all(log_dir).ok()?;
    let path = log_dir.join(name);
    let rotated_path = log_dir.join(format!("{name}.1"));
    if path.exists() {
        // Windows 的 rename 不会覆盖已有文件，先删除旧备份再归档本次会话。
        // 归档失败（如备份被占用）时继续追加原文件，只影响分段不影响写入。
        let _ = replace_rotated_file(&path, &rotated_path);
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()?;
    // 启动归档可能因文件占用失败；按实际长度初始化，避免把旧文件误当成空文件。
    // 元数据读取失败时按已达阈值处理，让下一行优先重试轮转而不是突破上限。
    let written = file
        .metadata()
        .map(|metadata| metadata.len())
        .unwrap_or(max_bytes);
    Some(LogSink {
        file: Some(new_writer(file)),
        path,
        rotated_path,
        max_bytes,
        written,
        new_writer,
    })
}

/// 按大小轮转的单文件追加写入器；句柄为 `None` 表示轮转后重开失败。
///
/// 轮转判断用累计字节数而非查询文件元数据：内核 info 级日志每条连接一行，
/// 逐行 `metadata()` 会退化为每行一次额外系统调用。缓冲中的行随写入计数，
/// 轮转在 rename 前显式冲刷，不丢行。
struct LogSink<W: Write> {
    file: Option<W>,
    path: PathBuf,
    rotated_path: PathBuf,
    max_bytes: u64,
    /// 当前文件累计写入的字节数（含仍在缓冲中未落盘的部分）。
    written: u64,
    /// 重开文件后的包装方式；与构造时一致。
    new_writer: fn(File) -> W,
}

impl<W: Write> LogSink<W> {
    fn write_line(&mut self, line: &str) {
        if self.written >= self.max_bytes {
            self.rotate();
        }
        if let Some(file) = self.file.as_mut() {
            if writeln!(file, "{line}").is_ok() {
                self.written = self
                    .written
                    .saturating_add((line.len() as u64).saturating_add(1));
            }
        }
    }

    /// 先冲刷并释放句柄再同目录 rename（避免 Windows 对已打开文件的共享
    /// 冲突）；重开失败时日志停写，但绝不能影响调用方流程。
    fn rotate(&mut self) {
        if let Some(mut writer) = self.file.take() {
            let _ = writer.flush();
        }
        let _ = replace_rotated_file(&self.path, &self.rotated_path);
        let new_writer = self.new_writer;
        let reopened = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .ok();
        self.written = reopened
            .as_ref()
            .and_then(|file| file.metadata().ok())
            .map(|metadata| metadata.len())
            .unwrap_or(self.max_bytes);
        self.file = reopened.map(new_writer);
    }
}

/// 用当前日志替换 `.1` 备份；显式删除旧文件以兼容 Windows rename 语义。
fn replace_rotated_file(path: &Path, rotated_path: &Path) -> std::io::Result<()> {
    match fs::remove_file(rotated_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::rename(path, rotated_path)
}

/// 内核日志写入器：内核 stdout/stderr 泵线程逐行追加 kernel.log。
///
/// 与运行日志的 [`APP_LOG`] 相互独立，内核高频输出不会冲掉客户端自身的
/// 关键生命周期记录；同样按大小轮转并脱敏。
pub(crate) struct KernelLogWriter {
    sink: Mutex<LogSink<BufWriter<File>>>,
}

impl KernelLogWriter {
    /// 打开内核日志；失败返回 `None`，调用方仍需排空内核管道防止阻塞。
    pub(crate) fn open(log_dir: &Path) -> Option<Arc<Self>> {
        let sink = open_sink(log_dir, "kernel.log", KERNEL_LOG_MAX_BYTES, BufWriter::new)?;
        Some(Arc::new(Self {
            sink: Mutex::new(sink),
        }))
    }

    /// 追加一行内核输出；内核行同样过脱敏，避免 provider 订阅 URL 落盘。
    pub(crate) fn write_line(&self, line: &str) {
        let redacted = redact(line);
        let mut sink = match self.sink.lock() {
            Ok(sink) => sink,
            Err(poisoned) => poisoned.into_inner(),
        };
        sink.write_line(&redacted);
    }
}

/// 日志脱敏：URL 只保留 scheme 与 host（内嵌凭据与路径/查询丢弃），
/// `secret=`/`token=` 等键值整体掩码。
///
/// 无敏感内容的行直接借用原文返回——内核日志绝大多数行不含 URL 或凭据，
/// 不为它们分配结果字符串。
pub(crate) fn redact(message: &str) -> Cow<'_, str> {
    if next_redaction(message).is_none() {
        return Cow::Borrowed(message);
    }
    let mut output = String::with_capacity(message.len());
    let mut cursor = 0;
    while cursor < message.len() {
        let rest = &message[cursor..];
        match next_redaction(rest) {
            Some((keep_until, suffix, next)) => {
                output.push_str(&rest[..keep_until]);
                output.push_str(suffix);
                cursor += next;
            }
            None => {
                output.push_str(rest);
                break;
            }
        }
    }
    Cow::Owned(output)
}

/// 返回 `rest` 中最早出现的敏感片段：`(保留到此为止的字节数, 追加的替代串,
/// 继续扫描的偏移)`；偏移必须大于 0，保证扫描始终前进。
fn next_redaction(rest: &str) -> Option<(usize, &'static str, usize)> {
    match (url_redaction(rest), key_redaction(rest)) {
        (Some(url), Some(key)) => Some(if url.0 <= key.0 { url } else { key }),
        (Some(url), None) => Some(url),
        (None, Some(key)) => Some(key),
        (None, None) => None,
    }
}

/// URL 脱敏：`scheme://host` 保留，凭据与路径/查询（可能携带 token）丢弃。
fn url_redaction(rest: &str) -> Option<(usize, &'static str, usize)> {
    let separator = rest.find("://")?;
    // 回溯 scheme 起点；scheme 至少一个字符，前一个字符必须是 ASCII 单词字符。
    let scheme_start = rest[..separator]
        .rfind(|c: char| !c.is_ascii_alphanumeric() && !matches!(c, '+' | '-' | '.'))
        .map(|index| index + 1)
        .unwrap_or(0);
    let authority_start = separator + 3;
    if scheme_start == separator {
        // "://" 前没有 scheme（如裸 "://x"），跳过该片段继续扫描。
        return Some((authority_start, "", authority_start));
    }

    let authority_end = rest[authority_start..]
        .char_indices()
        .find(|(_, c)| !is_authority_char(*c))
        .map(|(index, _)| authority_start + index)
        .unwrap_or(rest.len());
    let has_path = rest[authority_end..].starts_with(['/', '?', '#']);
    // URL token 的结束：下一个空白或引号（logrus 输出中的 URL 常被引号包裹）。
    let token_end = rest[authority_end..]
        .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\''))
        .map(|index| authority_end + index)
        .unwrap_or(rest.len());

    let authority = &rest[authority_start..authority_end];
    if authority.contains('@') {
        // 带凭据的 URL（user:pass@host）掩码整段 authority 并丢弃路径/查询。
        return Some((authority_start, "***", token_end));
    }
    if !has_path {
        // 无路径的 URL（如 controller 地址）保持原样。
        return Some((authority_end, "", authority_end));
    }
    Some((authority_end, "…", token_end))
}

/// URL authority（含 userinfo、host、port）中的合法字符。
fn is_authority_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '@' | '[' | ']' | '-' | '_' | '%' | '~')
}

/// 敏感键值脱敏：`secret=`、`token=`、`password=`、`authorization:` 后的值
/// 掩码到下一个空白；引号值掩码到闭合引号（含引号）。
fn key_redaction(rest: &str) -> Option<(usize, &'static str, usize)> {
    let mut best: Option<(usize, &'static str, usize)> = None;
    for key in SENSITIVE_KEYS {
        let Some(start) = find_ignore_ascii_case(rest, key) else {
            continue;
        };
        // 键后的空白（如 "Authorization: Bearer x"）不属于值。
        let after_key = start + key.len();
        let Some(value_start) = rest[after_key..]
            .char_indices()
            .find(|(_, c)| !c.is_whitespace())
            .map(|(index, _)| after_key + index)
        else {
            continue;
        };
        let mut value_end = if rest[value_start..].starts_with('"') {
            rest[value_start + 1..]
                .find('"')
                .map(|index| value_start + 1 + index + 1)
                .unwrap_or(rest.len())
        } else {
            rest[value_start..]
                .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\''))
                .map(|index| value_start + index)
                .unwrap_or(rest.len())
        };
        if key == "authorization:" && !rest[value_start..].starts_with('"') {
            // Authorization 常见格式为 `Bearer <token>` 或 `Basic <credentials>`；
            // 只掩码认证方案会把真正凭据原样留下，因此连同第二个字段一起处理。
            if let Some(credential_start) = rest[value_end..]
                .char_indices()
                .find(|(_, c)| !c.is_whitespace())
                .map(|(index, _)| value_end + index)
            {
                value_end = rest[credential_start..]
                    .find(char::is_whitespace)
                    .map(|index| credential_start + index)
                    .unwrap_or(rest.len());
            }
        }
        if value_end == value_start {
            // 空值没有可掩码内容。
            continue;
        }
        if best.is_none_or(|(existing, _, _)| start < existing) {
            best = Some((value_start, "***", value_end));
        }
    }
    best
}

/// 大小写不敏感的子串搜索，返回字节偏移。
///
/// 每行日志都要扫描敏感键，不能先分配整行小写拷贝再比较。键均为 ASCII，
/// UTF-8 多字节字符的后续字节（>=0x80）不会与 ASCII 键相等，匹配起点必然
/// 落在字符边界上，按字节窗口比较并返回字节偏移是安全的。
fn find_ignore_ascii_case(haystack: &str, needle: &str) -> Option<usize> {
    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&start| {
        haystack[start..start + needle.len()]
            .iter()
            .zip(needle)
            .all(|(byte, expected)| byte.eq_ignore_ascii_case(expected))
    })
}

/// 日志宏：第一个参数为模块标签（app/core/kernel/proxy/tun/profile/tray 等），
/// 其余参数同 `format!`；写入前统一脱敏，debug 级仅在 debug 构建生效。
macro_rules! log_error {
    ($tag:expr, $($arg:tt)+) => {
        $crate::logging::log($crate::logging::Level::Error, $tag, &format!($($arg)+))
    };
}

macro_rules! log_warn {
    ($tag:expr, $($arg:tt)+) => {
        $crate::logging::log($crate::logging::Level::Warn, $tag, &format!($($arg)+))
    };
}

macro_rules! log_info {
    ($tag:expr, $($arg:tt)+) => {
        $crate::logging::log($crate::logging::Level::Info, $tag, &format!($($arg)+))
    };
}

macro_rules! log_debug {
    ($tag:expr, $($arg:tt)+) => {
        $crate::logging::log($crate::logging::Level::Debug, $tag, &format!($($arg)+))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_strips_url_paths_and_queries() {
        assert_eq!(
            redact("下载 https://example.com/a/b?token=abc 失败"),
            "下载 https://example.com… 失败"
        );
        assert_eq!(
            redact("rule provider https://cdn.test/x.yml 下载超时"),
            "rule provider https://cdn.test… 下载超时"
        );
    }

    #[test]
    fn redact_masks_url_credentials_entirely() {
        assert_eq!(
            redact("源 http://user:pass@host:8080/path"),
            "源 http://***"
        );
    }

    #[test]
    fn redact_masks_sensitive_key_values() {
        assert_eq!(redact("secret=abcdef123"), "secret=***");
        assert_eq!(
            redact("header Authorization: Bearer abc123 请求失败"),
            "header Authorization: *** 请求失败"
        );
        assert_eq!(
            redact("响应包含 token=\"quoted-value\" 字段"),
            "响应包含 token=*** 字段"
        );
    }

    #[test]
    fn redact_keeps_plain_text_and_host_only_urls() {
        assert_eq!(
            redact("启动 Mihomo 失败：端口被占用"),
            "启动 Mihomo 失败：端口被占用"
        );
        // 无路径的 controller 地址保持原样，便于排查。
        assert_eq!(
            redact("controller http://127.0.0.1:9090"),
            "controller http://127.0.0.1:9090"
        );
        // 空值与普通单词不误伤。
        assert_eq!(redact("the secret is missing"), "the secret is missing");
        // 干净行借用原文，不为内核日志的绝大多数行分配结果字符串。
        assert!(matches!(redact("plain kernel line"), Cow::Borrowed(_)));
    }

    #[test]
    fn log_sink_rotates_when_size_threshold_reached() {
        let root = std::env::temp_dir().join(format!(
            "pure-clash-log-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("系统时间应晚于 UNIX_EPOCH")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("应创建测试日志目录");

        let path = root.join("app.log");
        let rotated_path = root.join("app.log.1");
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .expect("应打开测试日志文件");
        // 测试用 LineWriter 逐行落盘，写完即可直接读回，无需显式冲刷。
        let mut sink = LogSink {
            file: Some(LineWriter::new(file)),
            path: path.clone(),
            rotated_path: rotated_path.clone(),
            max_bytes: 16,
            written: 0,
            new_writer: LineWriter::new,
        };
        for index in 0..10 {
            sink.write_line(&format!("line-{index}-padding-padding"));
        }

        assert!(rotated_path.is_file(), "超阈值后应生成轮转备份");
        assert!(path.is_file(), "轮转后应重新打开当前文件");
        let current = fs::read_to_string(&path).expect("应读取当前日志");
        assert!(!current.is_empty(), "轮转后仍应继续写入");

        fs::remove_dir_all(root).expect("应清理测试日志目录");
    }

    #[test]
    fn log_rotation_replaces_existing_backup() {
        let root = std::env::temp_dir().join(format!(
            "pure-clash-log-replace-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("系统时间应晚于 UNIX_EPOCH")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("应创建测试日志目录");
        let path = root.join("app.log");
        let rotated_path = root.join("app.log.1");
        fs::write(&path, "current-session").expect("应写入当前日志");
        fs::write(&rotated_path, "stale-backup").expect("应写入旧备份");

        replace_rotated_file(&path, &rotated_path).expect("已有备份时仍应完成轮转");

        assert!(!path.exists(), "当前日志应已归档");
        assert_eq!(
            fs::read_to_string(&rotated_path).expect("应读取新备份"),
            "current-session"
        );
        fs::remove_dir_all(root).expect("应清理测试日志目录");
    }

    #[test]
    fn log_sink_preserves_existing_size_when_rotation_fails() {
        let root = std::env::temp_dir().join(format!(
            "pure-clash-log-rotation-failure-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("系统时间应晚于 UNIX_EPOCH")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("应创建测试日志目录");
        let path = root.join("app.log");
        let rotated_path = root.join("app.log.1");
        let existing = "existing-log-content";
        fs::write(&path, existing).expect("应写入现有日志");
        // 用同名目录稳定制造 remove_file 失败，模拟 Windows 文件占用等归档故障。
        fs::create_dir(&rotated_path).expect("应创建阻止归档的同名目录");

        let mut sink = open_sink(&root, "app.log", 16, LineWriter::new)
            .expect("归档失败后仍应继续打开当前日志");
        assert_eq!(sink.written, existing.len() as u64);

        sink.rotate();
        assert_eq!(sink.written, existing.len() as u64);

        drop(sink);
        fs::remove_dir_all(root).expect("应清理测试日志目录");
    }

    #[test]
    fn timestamp_uses_local_offset_format() {
        let stamp = timestamp();
        assert!(stamp.contains('T'), "时间戳应为 RFC3339 风格：{stamp}");
        assert!(stamp.len() >= 6, "时间戳应包含时区偏移：{stamp}");
        let tail = &stamp[stamp.len() - 6..];
        assert!(
            (tail.starts_with('+') || tail.starts_with('-')) && tail.as_bytes()[3] == b':',
            "时间戳应以本地时区偏移（±hh:mm）结尾：{stamp}"
        );
    }
}
