//! Linux 用户登录自启：按 XDG Autostart 规范维护用户级 desktop entry。

use std::{env, ffi::OsStr, fs, io::ErrorKind, path::PathBuf};

use crate::platform::AutoStartStatus;
use anyhow::{Context, Result, bail};

const AUTOSTART_ARG: &str = "--autostart";
const AUTOSTART_FILE: &str = "pure-clash.desktop";

/// 读取用户级 desktop entry，并核对 `Exec`、禁用标记和当前可执行文件路径。
pub(crate) fn autostart_status() -> Result<AutoStartStatus> {
    let path = autostart_file()?;
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(AutoStartStatus::Disabled),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("无法读取 Linux 登录自启文件：{}", path.display()));
        }
    };
    let executable = autostart_executable()?;
    let expected_exec = exec_value(executable.as_os_str())?;
    Ok(status_from_contents(&contents, &expected_exec))
}

/// 启用时原子写入 XDG Autostart 条目，关闭时移除 Pure Clash 自己的条目。
pub(crate) fn set_autostart(enabled: bool) -> Result<()> {
    let path = autostart_file()?;
    if !enabled {
        return match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error)
                .with_context(|| format!("无法删除 Linux 登录自启文件：{}", path.display())),
        };
    }

    let executable = autostart_executable()?;
    let contents = desktop_entry(executable.as_os_str())?;
    crate::platform::file::atomic_write(&path, contents.as_bytes())
        .with_context(|| format!("无法写入 Linux 登录自启文件：{}", path.display()))
}

fn autostart_file() -> Result<PathBuf> {
    let config_home = env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            // XDG Base Directory 规范在变量缺失或不是绝对路径时回退到 `$HOME/.config`。
            env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".config"))
        })
        .context("无法确定 Linux 用户配置目录")?;
    Ok(config_home.join("autostart").join(AUTOSTART_FILE))
}

fn autostart_executable() -> Result<PathBuf> {
    if let Some(value) = env::var_os("APPIMAGE").filter(|value| !value.is_empty()) {
        let appimage = PathBuf::from(value);
        if !appimage.is_absolute() {
            bail!("APPIMAGE 不是绝对路径，拒绝写入不可复现的登录自启命令");
        }
        // AppImage 的 `/proc/self/exe` 指向临时挂载目录，必须记录官方提供的原始路径。
        return Ok(appimage);
    }
    env::current_exe().context("无法确定当前程序路径")
}

fn desktop_entry(executable: &OsStr) -> Result<String> {
    Ok(format!(
        "[Desktop Entry]\nType=Application\nVersion=1.0\nName=Pure Clash\nExec={}\nIcon=pure-clash\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        exec_value(executable)?
    ))
}

fn exec_value(executable: &OsStr) -> Result<String> {
    let executable = executable
        .to_str()
        .context("Linux 可执行文件路径不是有效 UTF-8，无法写入 desktop entry")?;
    if executable
        .chars()
        .any(|character| matches!(character, '\n' | '\r' | '\0'))
    {
        bail!("Linux 可执行文件路径包含 desktop entry 不允许的控制字符");
    }

    // Desktop Entry 的 Exec 字段在双引号内仍要求转义反斜杠、引号、反引号和 `$`。
    let mut quoted = String::with_capacity(executable.len() + 2);
    quoted.push('"');
    for character in executable.chars() {
        match character {
            // Desktop Entry 会先处理通用字符串转义，再处理 Exec 引号；因此实际
            // 传给 Exec 解析器的一条反斜杠要在文件中写成两条。
            '"' | '`' | '$' => {
                quoted.push_str("\\\\");
                quoted.push(character);
            }
            // 字面反斜杠经过两层解析，需要在 desktop entry 中写成四条。
            '\\' => quoted.push_str("\\\\\\\\"),
            // Exec 把 `%x` 识别为字段代码；`%%` 才代表文件名中的字面百分号。
            '%' => quoted.push_str("%%"),
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    Ok(format!("{quoted} {AUTOSTART_ARG}"))
}

fn status_from_contents(contents: &str, expected_exec: &str) -> AutoStartStatus {
    let mut in_desktop_entry = false;
    let mut exec = None;
    let mut hidden = false;
    let mut gnome_enabled = true;
    for line in contents.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "Exec" => exec = Some(value.trim()),
            "Hidden" => hidden = value.trim().eq_ignore_ascii_case("true"),
            "X-GNOME-Autostart-enabled" => {
                gnome_enabled = !value.trim().eq_ignore_ascii_case("false")
            }
            _ => {}
        }
    }
    if !hidden && gnome_enabled && exec == Some(expected_exec) {
        AutoStartStatus::Enabled
    } else {
        AutoStartStatus::Disabled
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn desktop_entry_quotes_special_exec_characters() {
        let entry = desktop_entry(Path::new("/home/test/Pure $Clash` 100%/pure-clash").as_os_str())
            .expect("应生成 desktop entry");
        assert!(
            entry.contains("Exec=\"/home/test/Pure \\\\$Clash\\\\` 100%%/pure-clash\" --autostart")
        );
    }

    #[test]
    fn disabled_or_stale_entries_are_not_reported_as_enabled() {
        let expected = "\"/opt/pure-clash/pure-clash\" --autostart";
        let active = format!("[Desktop Entry]\nExec={expected}\n");
        assert_eq!(
            status_from_contents(&active, expected),
            AutoStartStatus::Enabled
        );
        assert_eq!(
            status_from_contents(&format!("{active}Hidden=true\n"), expected),
            AutoStartStatus::Disabled
        );
        assert_eq!(
            status_from_contents(
                "[Desktop Entry]\nExec=\"/old/pure-clash\" --autostart\n",
                expected
            ),
            AutoStartStatus::Disabled
        );
    }
}
