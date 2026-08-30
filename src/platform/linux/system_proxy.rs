//! Linux 桌面系统代理：通过 GNOME/Cinnamon 的 GSettings schema 管理当前会话。
//!
//! GSettings 是 GNOME、Cinnamon 及使用 GLib 代理解析器的应用读取系统代理的
//! 标准入口。写入前保存所有会修改键的原始 GVariant 文本，恢复时逐项原样写回，
//! 因而能够保留用户原来的 `none` / `manual` / `auto` 模式及各协议端点。

use std::{env, net::SocketAddr, process::Command};

use anyhow::{Context, Result, anyhow, bail};

use super::super::{LinuxProxySetting, LinuxSystemProxySnapshot, SystemProxySnapshot};

const SCHEMA_CANDIDATES: [&str; 2] = ["org.gnome.system.proxy", "org.cinnamon.system.proxy"];
const PROXY_KEYS: [(&str, &str); 11] = [
    ("", "mode"),
    ("", "use-same-proxy"),
    ("http", "host"),
    ("http", "port"),
    ("http", "use-authentication"),
    ("https", "host"),
    ("https", "port"),
    ("ftp", "host"),
    ("ftp", "port"),
    ("socks", "host"),
    ("socks", "port"),
];

/// 捕获当前桌面会话的完整代理设置，供关闭和异常恢复时原样还原。
pub(crate) fn capture_system_proxy() -> Result<SystemProxySnapshot> {
    let prefix = detect_schema()?;
    let settings = PROXY_KEYS
        .iter()
        .map(|(suffix, key)| {
            let schema = schema_name(&prefix, suffix);
            Ok(LinuxProxySetting {
                value: gsettings_get(&schema, key)?,
                schema,
                key: (*key).to_owned(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(SystemProxySnapshot {
        managed: true,
        prev_enabled: false,
        prev_server: String::new(),
        linux: Some(LinuxSystemProxySnapshot {
            schema_prefix: prefix,
            settings,
        }),
    })
}

/// 将 HTTP、HTTPS、FTP 与 SOCKS 全部指向 Mihomo mixed-port，最后再切换为
/// manual，避免写入中途让桌面应用看到半配置状态。
pub(crate) fn set_system_proxy(server: &str) -> Result<()> {
    let prefix = detect_schema()?;
    let (host, port) = parse_proxy_server(server)?;
    let host = gvariant_string(&host);
    let port = port.to_string();

    for protocol in ["http", "https", "ftp", "socks"] {
        let schema = schema_name(&prefix, protocol);
        gsettings_set(&schema, "host", &host)?;
        gsettings_set(&schema, "port", &port)?;
    }
    gsettings_set(&schema_name(&prefix, "http"), "use-authentication", "false")?;
    gsettings_set(&prefix, "use-same-proxy", "false")?;
    gsettings_set(&prefix, "mode", "'manual'")
}

/// 按状态文件中的原始 GVariant 文本恢复全部设置；模式最后恢复，降低切换期间
/// 使用错误端点的概率。
pub(crate) fn restore_system_proxy(snapshot: &SystemProxySnapshot) -> Result<()> {
    let linux = snapshot
        .linux
        .as_ref()
        .ok_or_else(|| anyhow!("系统代理状态文件缺少 Linux GSettings 快照"))?;
    if !SCHEMA_CANDIDATES.contains(&linux.schema_prefix.as_str()) {
        bail!("系统代理状态文件包含未知的 Linux schema");
    }
    validate_snapshot(linux)?;

    let mode = linux
        .settings
        .iter()
        .find(|setting| setting.schema == linux.schema_prefix && setting.key == "mode")
        .expect("完整快照必须包含 mode")
        .value
        .as_str();
    for setting in &linux.settings {
        if setting.schema != linux.schema_prefix || setting.key != "mode" {
            gsettings_set(&setting.schema, &setting.key, &setting.value)?;
        }
    }
    gsettings_set(&linux.schema_prefix, "mode", mode)
}

fn detect_schema() -> Result<String> {
    let desktop = [
        env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
        env::var("XDG_SESSION_DESKTOP").unwrap_or_default(),
        env::var("DESKTOP_SESSION").unwrap_or_default(),
    ]
    .join(":")
    .to_ascii_lowercase();
    let candidates = schema_candidates_for_desktop(&desktop)?;
    let output = Command::new("gsettings")
        .arg("list-schemas")
        .output()
        .context("无法执行 gsettings；当前桌面不支持 GNOME/Cinnamon 系统代理")?;
    if !output.status.success() {
        bail!("gsettings 无法列出当前桌面 schema");
    }
    let schemas = String::from_utf8_lossy(&output.stdout);
    candidates
        .into_iter()
        .find(|candidate| schemas.lines().any(|schema| schema == *candidate))
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("当前桌面没有可用的 GNOME/Cinnamon 系统代理 schema"))
}

fn schema_candidates_for_desktop(desktop: &str) -> Result<[&'static str; 2]> {
    if desktop.contains("cinnamon") {
        Ok([SCHEMA_CANDIDATES[1], SCHEMA_CANDIDATES[0]])
    } else if ["gnome", "ubuntu", "unity", "budgie"]
        .iter()
        .any(|name| desktop.contains(name))
    {
        Ok(SCHEMA_CANDIDATES)
    } else {
        bail!("当前 Linux 桌面暂不支持系统代理；目前支持 GNOME/Cinnamon 兼容会话")
    }
}

fn schema_name(prefix: &str, suffix: &str) -> String {
    if suffix.is_empty() {
        prefix.to_owned()
    } else {
        format!("{prefix}.{suffix}")
    }
}

fn gsettings_get(schema: &str, key: &str) -> Result<String> {
    let output = Command::new("gsettings")
        .args(["get", schema, key])
        .output()
        .with_context(|| format!("无法读取系统代理设置 {schema}.{key}"))?;
    if !output.status.success() {
        bail!(
            "读取系统代理设置 {schema}.{key} 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let value = String::from_utf8(output.stdout)
        .context("gsettings 返回了非 UTF-8 数据")?
        .trim()
        .to_owned();
    if value.is_empty() {
        bail!("系统代理设置 {schema}.{key} 返回空值");
    }
    Ok(value)
}

fn gsettings_set(schema: &str, key: &str, value: &str) -> Result<()> {
    let output = Command::new("gsettings")
        .args(["set", schema, key, value])
        .output()
        .with_context(|| format!("无法写入系统代理设置 {schema}.{key}"))?;
    if !output.status.success() {
        bail!(
            "写入系统代理设置 {schema}.{key} 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn parse_proxy_server(server: &str) -> Result<(String, u16)> {
    let addr: SocketAddr = server
        .parse()
        .with_context(|| format!("系统代理地址无效：{server}"))?;
    if !addr.ip().is_loopback() {
        bail!("系统代理只允许指向本机回环地址");
    }
    Ok((addr.ip().to_string(), addr.port()))
}

fn gvariant_string(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn validate_snapshot_setting(prefix: &str, setting: &LinuxProxySetting) -> Result<()> {
    let allowed = PROXY_KEYS
        .iter()
        .any(|(suffix, key)| setting.schema == schema_name(prefix, suffix) && setting.key == *key);
    if !allowed || setting.value.trim().is_empty() || setting.value.contains(['\n', '\r']) {
        bail!("Linux 系统代理快照包含无效设置");
    }
    Ok(())
}

fn validate_snapshot(snapshot: &LinuxSystemProxySnapshot) -> Result<()> {
    if snapshot.settings.len() != PROXY_KEYS.len() {
        bail!("Linux 系统代理快照不完整");
    }
    for setting in &snapshot.settings {
        validate_snapshot_setting(&snapshot.schema_prefix, setting)?;
    }
    for (suffix, key) in PROXY_KEYS {
        let schema = schema_name(&snapshot.schema_prefix, suffix);
        if snapshot
            .settings
            .iter()
            .filter(|setting| setting.schema == schema && setting.key == key)
            .count()
            != 1
        {
            bail!("Linux 系统代理快照不完整或包含重复设置");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_loopback_proxy_servers() {
        assert_eq!(
            parse_proxy_server("127.0.0.1:7890").unwrap(),
            ("127.0.0.1".to_owned(), 7890)
        );
        assert_eq!(
            parse_proxy_server("[::1]:7890").unwrap(),
            ("::1".to_owned(), 7890)
        );
        assert!(parse_proxy_server("192.0.2.1:7890").is_err());
        assert!(parse_proxy_server("localhost:7890").is_err());
    }

    #[test]
    fn snapshot_keys_are_restricted_to_managed_schemas() {
        let valid = LinuxProxySetting {
            schema: "org.gnome.system.proxy.http".into(),
            key: "host".into(),
            value: "'example.test'".into(),
        };
        assert!(validate_snapshot_setting("org.gnome.system.proxy", &valid).is_ok());

        let mut invalid = valid.clone();
        invalid.schema = "org.gnome.desktop.interface".into();
        assert!(validate_snapshot_setting("org.gnome.system.proxy", &invalid).is_err());
    }

    #[test]
    fn rejects_incomplete_or_duplicate_snapshots_before_restore() {
        let snapshot = LinuxSystemProxySnapshot {
            schema_prefix: "org.gnome.system.proxy".into(),
            settings: PROXY_KEYS
                .iter()
                .map(|(suffix, key)| LinuxProxySetting {
                    schema: schema_name("org.gnome.system.proxy", suffix),
                    key: (*key).into(),
                    value: "false".into(),
                })
                .collect(),
        };
        assert!(validate_snapshot(&snapshot).is_ok());

        let mut incomplete = snapshot.clone();
        incomplete.settings.pop();
        assert!(validate_snapshot(&incomplete).is_err());

        let mut duplicate = snapshot;
        duplicate.settings[1] = duplicate.settings[0].clone();
        assert!(validate_snapshot(&duplicate).is_err());
    }

    #[test]
    fn chooses_schema_for_supported_desktops() {
        assert_eq!(
            schema_candidates_for_desktop("ubuntu:gnome").unwrap()[0],
            "org.gnome.system.proxy"
        );
        assert_eq!(
            schema_candidates_for_desktop("x-cinnamon").unwrap()[0],
            "org.cinnamon.system.proxy"
        );
        assert!(schema_candidates_for_desktop("kde").is_err());
    }

    /// 会短暂修改当前桌面会话的系统代理，手动验证时显式运行：
    /// `cargo test linux_system_proxy_roundtrip -- --ignored`
    #[test]
    #[ignore = "会短暂修改当前桌面会话的系统代理"]
    fn linux_system_proxy_roundtrip() {
        let snapshot = capture_system_proxy().expect("应捕获系统代理");
        set_system_proxy("127.0.0.1:7899").expect("应设置系统代理");
        restore_system_proxy(&snapshot).expect("应恢复系统代理");
        let restored = capture_system_proxy().expect("应再次捕获系统代理");
        assert_eq!(
            restored.linux.unwrap().settings,
            snapshot.linux.unwrap().settings
        );
    }
}
