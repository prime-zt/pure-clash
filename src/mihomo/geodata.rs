//! Mihomo 配置校验前所需地理数据库的按需准备。
//!
//! Mihomo 在缺少 GeoSite/GeoIP 数据时会自行从 GitHub 下载，但该下载发生在
//! `-t` 校验子进程内，失败时只能得到不完整的内核日志。客户端在校验前按配置
//! 实际引用下载官方 MetaCubeX 数据，并用完成标记排除中断留下的半成品。

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use uuid::Uuid;

use crate::platform::AppPaths;

const MAX_GEODATA_BYTES: u64 = 64 * 1024 * 1024;
const COMPLETION_FILE: &str = ".pure-clash-geodata.json";
const SOURCE_REVISION: u32 = 1;

const GEOSITE: GeodataFile = GeodataFile {
    name: "GeoSite.dat",
    url: "https://raw.githubusercontent.com/MetaCubeX/meta-rules-dat/release/geosite.dat",
};
const GEOIP: GeodataFile = GeodataFile {
    name: "GeoIP.dat",
    url: "https://raw.githubusercontent.com/MetaCubeX/meta-rules-dat/release/geoip.dat",
};
const COUNTRY: GeodataFile = GeodataFile {
    name: "Country.mmdb",
    url: "https://raw.githubusercontent.com/MetaCubeX/meta-rules-dat/release/country.mmdb",
};

#[derive(Clone, Copy)]
struct GeodataFile {
    name: &'static str,
    url: &'static str,
}

#[derive(Default, Debug, PartialEq, Eq)]
struct Requirements {
    geosite: bool,
    geoip: bool,
    geodata_mode: bool,
}

#[derive(Default, Deserialize, Serialize)]
struct CompletionState {
    revision: u32,
    #[serde(default)]
    files: BTreeMap<String, u64>,
}

/// 根据待校验配置按需准备 Mihomo 地理数据库。
pub(super) fn prepare_for_config(paths: &AppPaths, config_file: &Path) -> Result<()> {
    let content = fs::read_to_string(config_file)
        .with_context(|| format!("无法读取待校验配置：{}", config_file.display()))?;
    let requirements = requirements_from_yaml(&content);
    if !requirements.geosite && !requirements.geoip {
        return Ok(());
    }

    fs::create_dir_all(&paths.mihomo_data_dir).with_context(|| {
        format!(
            "无法创建 Mihomo 数据目录：{}",
            paths.mihomo_data_dir.display()
        )
    })?;

    let mut state = read_completion_state(&paths.mihomo_data_dir);
    if state.revision != SOURCE_REVISION {
        state = CompletionState {
            revision: SOURCE_REVISION,
            ..Default::default()
        };
    }

    let mut files = Vec::with_capacity(2);
    if requirements.geosite {
        files.push(GEOSITE);
    }
    if requirements.geoip {
        files.push(if requirements.geodata_mode {
            GEOIP
        } else {
            COUNTRY
        });
    }

    for file in files {
        if completed_file_is_valid(&paths.mihomo_data_dir, file.name, &state) {
            continue;
        }
        let size = download_file(&paths.mihomo_data_dir, file)
            .with_context(|| format!("无法准备 Mihomo 地理数据库 {}", file.name))?;
        state.files.insert(file.name.to_owned(), size);
        write_completion_state(&paths.mihomo_data_dir, &state)?;
    }
    Ok(())
}

fn requirements_from_yaml(content: &str) -> Requirements {
    let Ok(value) = serde_yaml::from_str::<Value>(content) else {
        // YAML 语法错误仍交给 Mihomo `-t` 输出最终诊断。
        return Requirements::default();
    };
    let geodata_mode = value
        .get("geodata-mode")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut requirements = Requirements {
        geodata_mode,
        ..Default::default()
    };
    scan_value(&value, &mut requirements);
    requirements
}

fn scan_value(value: &Value, requirements: &mut Requirements) {
    match value {
        Value::String(text) => {
            let upper = text.to_ascii_uppercase();
            requirements.geosite |= upper.contains("GEOSITE,") || upper.contains("GEOSITE:");
            requirements.geoip |= upper.contains("GEOIP,") || upper.contains("GEOIP:");
        }
        Value::Sequence(items) => {
            for item in items {
                scan_value(item, requirements);
            }
        }
        Value::Mapping(mapping) => {
            for (key, item) in mapping {
                scan_value(key, requirements);
                scan_value(item, requirements);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Tagged(_) => {}
    }
}

fn read_completion_state(data_dir: &Path) -> CompletionState {
    fs::read(data_dir.join(COMPLETION_FILE))
        .ok()
        .and_then(|content| serde_json::from_slice(&content).ok())
        .unwrap_or_default()
}

fn completed_file_is_valid(data_dir: &Path, name: &str, state: &CompletionState) -> bool {
    let Some(expected_size) = state.files.get(name).copied().filter(|size| *size > 0) else {
        return false;
    };
    fs::metadata(data_dir.join(name))
        .map(|metadata| metadata.is_file() && metadata.len() == expected_size)
        .unwrap_or(false)
}

fn download_file(data_dir: &Path, file: GeodataFile) -> Result<u64> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .build();
    let response = agent
        .get(file.url)
        .set("User-Agent", "pure-clash/geodata")
        .call()
        .map_err(|error| anyhow::anyhow!("{error}"))
        .context("官方数据下载失败")?;

    if let Some(length) = response
        .header("Content-Length")
        .and_then(|length| length.parse::<u64>().ok())
        && length > MAX_GEODATA_BYTES
    {
        bail!("响应超过 64MB 体积上限");
    }

    let temp = temporary_path(data_dir, file.name, "download");
    let result = (|| {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .with_context(|| format!("无法创建临时文件：{}", temp.display()))?;
        let mut reader = response.into_reader().take(MAX_GEODATA_BYTES + 1);
        let size = std::io::copy(&mut reader, &mut output).context("官方数据下载中断")?;
        if size == 0 {
            bail!("官方数据响应为空");
        }
        if size > MAX_GEODATA_BYTES {
            bail!("响应超过 64MB 体积上限");
        }
        output.flush().context("无法刷新地理数据库临时文件")?;
        output.sync_all().context("无法同步地理数据库临时文件")?;
        drop(output);

        replace_file(&temp, &data_dir.join(file.name))?;
        Ok(size)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn write_completion_state(data_dir: &Path, state: &CompletionState) -> Result<()> {
    let content = serde_json::to_vec_pretty(state).context("无法序列化地理数据库完成标记")?;
    let temp = temporary_path(data_dir, COMPLETION_FILE, "state");
    let result = (|| {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .with_context(|| format!("无法创建完成标记临时文件：{}", temp.display()))?;
        output
            .write_all(&content)
            .context("无法写入地理数据库完成标记")?;
        output.flush().context("无法刷新地理数据库完成标记")?;
        output.sync_all().context("无法同步地理数据库完成标记")?;
        drop(output);
        replace_file(&temp, &data_dir.join(COMPLETION_FILE))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// 跨平台替换目标文件；Windows 不允许 `rename` 覆盖既有文件，因此先把旧文件
/// 移到同目录备份，替换失败时再恢复。
fn replace_file(temp: &Path, target: &Path) -> Result<()> {
    if !target.exists() {
        return fs::rename(temp, target)
            .with_context(|| format!("无法安装下载文件：{}", target.display()));
    }

    let backup = temporary_path(
        target.parent().unwrap_or_else(|| Path::new(".")),
        target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("data"),
        "backup",
    );
    fs::rename(target, &backup)
        .with_context(|| format!("无法备份既有文件：{}", target.display()))?;
    if let Err(error) = fs::rename(temp, target) {
        let _ = fs::rename(&backup, target);
        return Err(error).with_context(|| format!("无法安装下载文件：{}", target.display()));
    }
    let _ = fs::remove_file(backup);
    Ok(())
}

fn temporary_path(directory: &Path, name: &str, purpose: &str) -> PathBuf {
    directory.join(format!(".{name}.{purpose}-{}", Uuid::new_v4().simple()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_only_required_geodata() {
        let requirements = requirements_from_yaml(
            "rules:\n- GEOSITE,private,DIRECT\n- GEOIP,CN,DIRECT\n- MATCH,PROXY\n",
        );
        assert_eq!(
            requirements,
            Requirements {
                geosite: true,
                geoip: true,
                geodata_mode: false,
            }
        );

        let requirements = requirements_from_yaml(
            "geodata-mode: true\ndns:\n  nameserver-policy:\n    geosite:cn: 223.5.5.5\nrules:\n  - geoip:private\n",
        );
        assert_eq!(
            requirements,
            Requirements {
                geosite: true,
                geoip: true,
                geodata_mode: true,
            }
        );
    }

    #[test]
    fn ignores_configs_without_geodata_rules() {
        assert_eq!(
            requirements_from_yaml("rules:\n- DOMAIN-SUFFIX,example.com,DIRECT\n- MATCH,DIRECT\n"),
            Requirements::default()
        );
    }

    #[test]
    fn completion_requires_matching_nonempty_file() {
        let root = std::env::temp_dir().join(format!(
            "pure-clash-geodata-state-{}-{}",
            std::process::id(),
            Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).expect("应创建测试目录");
        fs::write(root.join(GEOSITE.name), b"complete").expect("应写入测试文件");

        let mut state = CompletionState {
            revision: SOURCE_REVISION,
            ..Default::default()
        };
        assert!(!completed_file_is_valid(&root, GEOSITE.name, &state));
        state.files.insert(GEOSITE.name.to_owned(), 8);
        assert!(completed_file_is_valid(&root, GEOSITE.name, &state));
        state.files.insert(GEOSITE.name.to_owned(), 9);
        assert!(!completed_file_is_valid(&root, GEOSITE.name, &state));

        fs::remove_dir_all(root).expect("应清理测试目录");
    }
}
