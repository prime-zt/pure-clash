//! Windows 系统代理：写入当前用户 `Internet Settings` 注册表并立即生效。
//!
//! 只操作 HKCU，不需要管理员权限；写入后通过 `InternetSetOption` 广播
//! 设置变更与刷新，WinINet/WinHTTP 应用无需重启即可感知新代理。

use anyhow::{Result, anyhow};
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::Networking::WinInet::{
    INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED, InternetSetOptionW,
};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_DWORD, REG_SZ, RegCloseKey,
    RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};

use crate::platform::SystemProxySnapshot;

const INTERNET_SETTINGS_PATH: &str =
    "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings";

/// 读取当前用户系统代理状态：(是否开启, 代理服务器地址)。
pub(crate) fn read_system_proxy() -> Result<(bool, String)> {
    let mut key: HKEY = std::ptr::null_mut();
    open_settings_key(KEY_QUERY_VALUE, &mut key)?;
    let outcome = (|| {
        let enabled = read_dword(key, "ProxyEnable").unwrap_or(0) != 0;
        let server = read_string(key, "ProxyServer").unwrap_or_default();
        Ok((enabled, server))
    })();
    unsafe { RegCloseKey(key) };
    outcome
}

/// 写入当前用户系统代理并立即生效；关闭时 `server` 传空字符串。
pub(crate) fn write_system_proxy(enabled: bool, server: &str) -> Result<()> {
    let mut key: HKEY = std::ptr::null_mut();
    open_settings_key(KEY_SET_VALUE, &mut key)?;
    let outcome = (|| {
        write_dword(key, "ProxyEnable", u32::from(enabled))?;
        write_string(key, "ProxyServer", server)
    })();
    unsafe { RegCloseKey(key) };
    outcome?;
    notify_proxy_changed();
    Ok(())
}

pub(crate) fn capture_system_proxy() -> Result<SystemProxySnapshot> {
    let (prev_enabled, prev_server) = read_system_proxy()?;
    Ok(SystemProxySnapshot {
        managed: true,
        prev_enabled,
        prev_server,
        linux: None,
    })
}

pub(crate) fn set_system_proxy(server: &str) -> Result<()> {
    write_system_proxy(true, server)
}

pub(crate) fn restore_system_proxy(snapshot: &SystemProxySnapshot) -> Result<()> {
    write_system_proxy(snapshot.prev_enabled, &snapshot.prev_server)
}

fn open_settings_key(access: u32, key: &mut HKEY) -> Result<()> {
    let path = wide(INTERNET_SETTINGS_PATH);
    let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, access, key) };
    if status != ERROR_SUCCESS {
        return Err(anyhow!(
            "打开 Internet Settings 注册表项失败：status={status}"
        ));
    }
    Ok(())
}

fn read_dword(key: HKEY, name: &str) -> Result<u32> {
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegQueryValueExW(
            key,
            wide(name).as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::addr_of_mut!(value).cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(anyhow!("读取注册表值 {name} 失败：status={status}"));
    }
    Ok(value)
}

fn read_string(key: HKEY, name: &str) -> Result<String> {
    let mut buffer = [0u8; 1024];
    let mut size = buffer.len() as u32;
    let status = unsafe {
        RegQueryValueExW(
            key,
            wide(name).as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            buffer.as_mut_ptr(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(anyhow!("读取注册表值 {name} 失败：status={status}"));
    }
    let bytes = &buffer[..(size as usize).min(buffer.len())];
    // REG_SZ 以 UTF-16 存储并带结尾 NUL，截掉尾巴后成对解码。
    let pairs: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    Ok(String::from_utf16_lossy(&pairs))
}

fn write_dword(key: HKEY, name: &str, value: u32) -> Result<()> {
    let status = unsafe {
        RegSetValueExW(
            key,
            wide(name).as_ptr(),
            0,
            REG_DWORD,
            value.to_ne_bytes().as_ptr(),
            std::mem::size_of::<u32>() as u32,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(anyhow!("写入注册表值 {name} 失败：status={status}"));
    }
    Ok(())
}

fn write_string(key: HKEY, name: &str, value: &str) -> Result<()> {
    let mut data = wide(value);
    let status = unsafe {
        RegSetValueExW(
            key,
            wide(name).as_ptr(),
            0,
            REG_SZ,
            data.as_mut_ptr().cast(),
            data.len() as u32 * 2,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(anyhow!("写入注册表值 {name} 失败：status={status}"));
    }
    Ok(())
}

/// 广播设置变更并要求立即刷新，让系统代理即时生效。
fn notify_proxy_changed() {
    unsafe {
        InternetSetOptionW(
            std::ptr::null_mut(),
            INTERNET_OPTION_SETTINGS_CHANGED,
            std::ptr::null_mut(),
            0,
        );
        InternetSetOptionW(
            std::ptr::null_mut(),
            INTERNET_OPTION_REFRESH,
            std::ptr::null_mut(),
            0,
        );
    }
}

/// UTF-16 编码并以 NUL 结尾的 Windows 字符串。
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 直接操作当前用户注册表的往返验证，仅供手动运行：
    /// `cargo test system_proxy_registry_roundtrip -- --ignored`
    #[test]
    #[ignore = "会短暂修改当前用户的系统代理设置"]
    fn system_proxy_registry_roundtrip() {
        let previous = read_system_proxy().expect("应能读取系统代理");
        write_system_proxy(true, "127.0.0.1:7899").expect("应能启用系统代理");
        let current = read_system_proxy().expect("应能读回系统代理");

        // 无论后续断言如何，先恢复用户原有设置再校验结果。
        write_system_proxy(previous.0, &previous.1).expect("应能恢复系统代理");
        let restored = read_system_proxy().expect("应能读回恢复结果");

        assert_eq!(current, (true, "127.0.0.1:7899".to_string()));
        assert_eq!(restored, previous);
    }
}
