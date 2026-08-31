//! Windows 当前用户登录自启：使用官方 Run 注册表项，不请求管理员权限。

use std::{os::windows::ffi::OsStrExt, path::Path};

use anyhow::{Context, Result, anyhow};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
    System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegGetValueW, RegSetValueExW,
    },
};

use crate::platform::AutoStartStatus;

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "PureClash";
const AUTOSTART_ARG: &str = "--autostart";
const MAX_RUN_COMMAND_UNITS: usize = 260;

/// 读取 Run 项并核对完整命令；旧安装路径不会被误报为已启用。
pub(crate) fn autostart_status() -> Result<AutoStartStatus> {
    let expected =
        command_for_executable(&std::env::current_exe().context("无法确定当前程序路径")?);
    Ok(match read_run_value()? {
        Some(current) if current == expected => AutoStartStatus::Enabled,
        _ => AutoStartStatus::Disabled,
    })
}

/// 启用时写入带引号的绝对 EXE 路径，关闭时删除当前用户的注册值。
pub(crate) fn set_autostart(enabled: bool) -> Result<()> {
    if enabled {
        let executable = std::env::current_exe().context("无法确定当前程序路径")?;
        let command = command_for_executable(&executable);
        if command.len() > MAX_RUN_COMMAND_UNITS {
            return Err(anyhow!(
                "Windows 登录自启命令超过 {MAX_RUN_COMMAND_UNITS} 个 UTF-16 单元限制"
            ));
        }
        write_run_value(&command)
    } else {
        delete_run_value()
    }
}

fn command_for_executable(executable: &Path) -> Vec<u16> {
    // 直接保留 Windows 原生 UTF-16 路径，避免 display 的有损转换破坏少见文件名。
    std::iter::once('"' as u16)
        .chain(executable.as_os_str().encode_wide())
        .chain(format!("\" {AUTOSTART_ARG}").encode_utf16())
        .collect()
}

fn read_run_value() -> Result<Option<Vec<u16>>> {
    let key = wide(RUN_KEY);
    let name = wide(RUN_VALUE);
    let mut byte_len = 0u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut byte_len,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if status != ERROR_SUCCESS {
        return Err(anyhow!("读取 Windows 登录自启注册值失败：status={status}"));
    }

    let mut data = vec![0u16; (byte_len as usize).div_ceil(2)];
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            data.as_mut_ptr().cast(),
            &mut byte_len,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(anyhow!("读取 Windows 登录自启注册值失败：status={status}"));
    }
    let end = data
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(data.len());
    data.truncate(end);
    Ok(Some(data))
}

fn write_run_value(value: &[u16]) -> Result<()> {
    let key_path = wide(RUN_KEY);
    let mut key: HKEY = std::ptr::null_mut();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            key_path.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(anyhow!(
            "打开 Windows 登录自启注册表项失败：status={status}"
        ));
    }

    let name = wide(RUN_VALUE);
    let mut data = value.to_vec();
    data.push(0);
    let status = unsafe {
        RegSetValueExW(
            key,
            name.as_ptr(),
            0,
            REG_SZ,
            data.as_ptr().cast(),
            (data.len() * std::mem::size_of::<u16>()) as u32,
        )
    };
    unsafe { RegCloseKey(key) };
    if status != ERROR_SUCCESS {
        return Err(anyhow!("写入 Windows 登录自启注册值失败：status={status}"));
    }
    Ok(())
}

fn delete_run_value() -> Result<()> {
    let key_path = wide(RUN_KEY);
    let mut key: HKEY = std::ptr::null_mut();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            key_path.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(anyhow!(
            "打开 Windows 登录自启注册表项失败：status={status}"
        ));
    }
    let status = unsafe { RegDeleteValueW(key, wide(RUN_VALUE).as_ptr()) };
    unsafe { RegCloseKey(key) };
    if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
        return Err(anyhow!("删除 Windows 登录自启注册值失败：status={status}"));
    }
    Ok(())
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_quotes_executable_and_adds_background_flag() {
        let executable = Path::new(r"C:\Users\Test User\Pure Clash\pure-clash.exe");
        assert_eq!(
            String::from_utf16_lossy(&command_for_executable(executable)),
            r#""C:\Users\Test User\Pure Clash\pure-clash.exe" --autostart"#
        );
    }

    /// 直接操作当前用户 Run 项的往返验证，仅供手动运行，避免常规测试改变系统状态。
    #[test]
    #[ignore = "会短暂修改当前用户的登录自启注册值"]
    fn registry_roundtrip() {
        let previous = read_run_value().expect("应能读取自启注册值");
        let test_value: Vec<u16> =
            r#""C:\Pure Clash\pure-clash.exe" --autostart"#.encode_utf16().collect();
        write_run_value(&test_value).expect("应能写入自启注册值");
        let current = read_run_value();
        let restored = match previous {
            Some(value) => write_run_value(&value),
            None => delete_run_value(),
        };

        // 所有断言都放在恢复之后，失败时也不把测试值遗留到用户登录项。
        restored.expect("应能恢复测试前的注册值");
        assert!(String::from_utf16_lossy(&current.unwrap().unwrap()).contains("--autostart"));
    }
}
