//! Mihomo 配置文件的本地基线、订阅合并与结构校验。
//!
//! 内核实际加载的 `runtime.yaml` 是「订阅内容 + 客户端本地基线」的合并产物：
//! 端口、controller、secret 等本机字段一律以 [`LocalBaseline`] 为准，订阅中的
//! 代理、策略组和规则原样保留；订阅不得改动监听行为，也不得开启 TUN。

use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use serde_yaml::{Mapping, Value};
use uuid::Uuid;

use crate::platform::AppPaths;

/// 客户端本地基线；保存端口、controller 地址与随机 secret 等本机字段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalBaseline {
    /// 本地代理混合监听端口。
    pub(crate) mixed_port: u16,
    /// external controller 监听地址（仅回环）。
    pub(crate) controller_addr: String,
    /// controller 访问凭据；每安装随机生成，只存在于 local.yaml 与内存。
    pub(crate) secret: String,
    /// TUN 模式开关；由用户显式授权，重启内核时经此字段注入配置。
    pub(crate) tun_enable: bool,
}

/// 读取或生成客户端本地基线文件。
pub(crate) fn ensure_baseline(paths: &AppPaths) -> Result<LocalBaseline> {
    let file = &paths.local_mihomo_config_file;
    if file.is_file() {
        let content = fs::read_to_string(file)
            .with_context(|| format!("无法读取本地基线配置：{}", file.display()))?;
        return parse_baseline(&content)
            .with_context(|| format!("本地基线配置无效：{}", file.display()));
    }

    let baseline = LocalBaseline {
        mixed_port: 7890,
        controller_addr: "127.0.0.1:9097".to_owned(),
        secret: Uuid::new_v4().to_string(),
        tun_enable: false,
    };
    crate::platform::file::atomic_write(file, baseline.to_yaml().as_bytes())
        .with_context(|| format!("无法写入本地基线配置：{}", file.display()))?;
    Ok(baseline)
}

/// 把基线写回 local.yaml；TUN 等客户端级开关更新时调用。
pub(crate) fn save_baseline(paths: &AppPaths, baseline: &LocalBaseline) -> Result<()> {
    let file = &paths.local_mihomo_config_file;
    crate::platform::file::atomic_write(file, baseline.to_yaml().as_bytes())
        .with_context(|| format!("无法写入本地基线配置：{}", file.display()))
}

impl LocalBaseline {
    /// 序列化为 YAML 文本；secret 随文件落盘，调用方不得写日志。
    fn to_yaml(&self) -> String {
        let mut mapping = Mapping::new();
        self.insert_baseline_fields(&mut mapping);
        serde_yaml::to_string(&Value::Mapping(mapping)).expect("基线配置序列化不应失败")
    }

    /// 把全部基线字段写入目标 mapping；这是合并时"本地为准"的唯一来源。
    fn insert_baseline_fields(&self, mapping: &mut Mapping) {
        let mut set = |key: &str, value: Value| {
            mapping.insert(Value::from(key), value);
        };
        // 本地监听字段：订阅不得打开额外端口或允许局域网访问。
        set("mixed-port", Value::from(self.mixed_port));
        set("port", Value::from(0));
        set("socks-port", Value::from(0));
        set("redir-port", Value::from(0));
        set("tproxy-port", Value::from(0));
        set("allow-lan", Value::from(false));
        set("bind-address", Value::from("127.0.0.1"));
        // 行为默认值由客户端控制。
        set("mode", Value::from("rule"));
        set("log-level", Value::from("info"));
        // 统一延迟口径，并让 Mihomo 并发尝试目标 IP；这两个产品默认适用于
        // 内置配置和所有订阅，避免不同配置来源产生不一致的连接行为。
        set("unified-delay", Value::from(true));
        set("tcp-concurrent", Value::from(true));
        // TUN 开启时放行 IPv6，由 Mihomo 的默认双栈 TUN 地址和 auto-route 接管；
        // 关闭时保持 false。
        set("ipv6", Value::from(self.tun_enable));
        // TUN 由客户端开关控制：启用时写入推荐栈配置，否则强制关闭；
        // 该字段涉及提权与路由变更，订阅不得自行开启。
        let mut tun = Mapping::new();
        tun.insert(Value::from("enable"), Value::from(self.tun_enable));
        if self.tun_enable {
            tun.insert(
                Value::from("stack"),
                Value::from(crate::platform::tun_stack()),
            );
            tun.insert(Value::from("auto-route"), Value::from(true));
            tun.insert(Value::from("strict-route"), Value::from(false));
            tun.insert(Value::from("auto-detect-interface"), Value::from(true));
            tun.insert(
                Value::from("dns-hijack"),
                Value::Sequence(vec![Value::from("any:53")]),
            );
            if let Some(address) = crate::platform::tun_inet6_address() {
                tun.insert(
                    Value::from("inet6-address"),
                    Value::Sequence(vec![Value::from(address)]),
                );
            }
        }
        set("tun", Value::Mapping(tun));
        // dns-hijack 会把系统 DNS 全部劫进内核：开启 TUN 时必须由客户端注入
        // 完整的 fake-ip DNS 配置，否则内核没有上游可查、所有域名解析失败；
        // 未开 TUN 时不注入，保留订阅自带的 DNS 配置。
        if self.tun_enable {
            let dns_filter: Vec<Value> = [
                "*.lan",
                "*.local",
                "*.localhost",
                "*.test",
                "*.home.arpa",
                "time.windows.com",
                "time.*.apple.com",
                "time1.cloud.tencent.com",
                "+.market.xiaomi.com",
            ]
            .iter()
            .map(|entry| Value::from(*entry))
            .collect();
            let mut dns = Mapping::new();
            dns.insert(Value::from("enable"), Value::from(true));
            dns.insert(Value::from("enhanced-mode"), Value::from("fake-ip"));
            // Linux 对齐同机 Clash Verge Rev 的已验证双栈 fake-IP 配置；Windows
            // 保持现有 v4 fake-IP 行为，避免改变已工作的 Wintun 路径。
            dns.insert(
                Value::from("ipv6"),
                Value::from(crate::platform::tun_dns_ipv6()),
            );
            dns.insert(Value::from("fake-ip-range"), Value::from("198.18.0.1/16"));
            if crate::platform::tun_dns_ipv6() {
                dns.insert(
                    Value::from("fake-ip-range6"),
                    Value::from("fdfe:dcba:9876::1/64"),
                );
            }
            dns.insert(Value::from("fake-ip-filter"), Value::Sequence(dns_filter));
            dns.insert(
                Value::from("default-nameserver"),
                Value::Sequence(vec![Value::from("223.5.5.5"), Value::from("119.29.29.29")]),
            );
            dns.insert(
                Value::from("nameserver"),
                Value::Sequence(vec![
                    Value::from("https://doh.pub/dns-query"),
                    Value::from("https://dns.alidns.com/dns-query"),
                ]),
            );
            set("dns", Value::Mapping(dns));
        }
        // external controller 只监听回环，secret 随机生成。
        set(
            "external-controller",
            Value::from(self.controller_addr.clone()),
        );
        set("secret", Value::from(self.secret.clone()));
    }
}

/// 解析并校验本地基线 YAML。
fn parse_baseline(content: &str) -> Result<LocalBaseline> {
    let value: Value = serde_yaml::from_str(content).context("基线配置不是有效的 YAML")?;
    let mapping = value
        .as_mapping()
        .ok_or_else(|| anyhow::anyhow!("基线配置必须是 YAML 映射"))?;

    let mixed_port = mapping
        .get(Value::from("mixed-port"))
        .and_then(Value::as_u64)
        .filter(|port| *port > 0 && *port <= u16::MAX as u64)
        .ok_or_else(|| anyhow::anyhow!("基线配置缺少有效的 mixed-port"))?;
    let controller_addr = mapping
        .get(Value::from("external-controller"))
        .and_then(Value::as_str)
        .filter(|addr| !addr.is_empty())
        .ok_or_else(|| anyhow::anyhow!("基线配置缺少有效的 external-controller"))?;
    let secret = mapping
        .get(Value::from("secret"))
        .and_then(Value::as_str)
        .filter(|secret| !secret.is_empty())
        .ok_or_else(|| anyhow::anyhow!("基线配置缺少有效的 secret"))?;
    // 旧版本基线文件没有 tun 字段，缺省视为关闭。
    let tun_enable = mapping
        .get(Value::from("tun"))
        .and_then(|tun| tun.get("enable"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Ok(LocalBaseline {
        mixed_port: mixed_port as u16,
        controller_addr: controller_addr.to_owned(),
        secret: secret.to_owned(),
        tun_enable,
    })
}

/// 把订阅/导入的配置与本地基线合并为 `runtime.yaml` 文本。
///
/// 返回合并后的 YAML 文本供内核 `-t` 校验；不写任何文件。
pub(crate) fn merge_runtime(profile_yaml: &str, baseline: &LocalBaseline) -> Result<String> {
    let profile = parse_profile(profile_yaml)?;
    let mut mapping = profile;
    baseline.insert_baseline_fields(&mut mapping);
    // 外部 UI 由订阅提供时可能指向任意目录，客户端不提供该能力，直接剔除。
    mapping.remove(Value::from("external-ui"));
    mapping.remove(Value::from("external-ui-name"));
    // controller IPC 路径属于本机边界，订阅不得让 root TUN 服务在任意位置建 socket。
    mapping.remove(Value::from("external-controller-unix"));
    mapping.remove(Value::from("external-controller-pipe"));

    serde_yaml::to_string(&Value::Mapping(mapping)).context("无法序列化运行时配置")
}

/// 解析并结构校验订阅配置；返回顶层 mapping。
fn parse_profile(profile_yaml: &str) -> Result<Mapping> {
    // 部分订阅服务会在文件头带 UTF-8 BOM，serde_yaml 无法处理，先剥离。
    let profile_yaml = profile_yaml
        .strip_prefix('\u{feff}')
        .unwrap_or(profile_yaml);
    let value: Value = serde_yaml::from_str(profile_yaml).context("配置不是有效的 YAML")?;
    let Value::Mapping(mapping) = value else {
        bail!("配置顶层必须是 YAML 映射");
    };

    // 结构预检只看类型是否正确，引用完整性交给内核 `-t` 终审。
    for key in [
        "proxies",
        "proxy-groups",
        "rules",
        "proxy-providers",
        "rule-providers",
    ] {
        if let Some(item) = mapping.get(Value::from(key))
            && !item.is_null()
            && !item.is_sequence()
        {
            bail!("配置字段 {key} 必须是列表");
        }
    }

    Ok(mapping)
}

/// 把合并产物写入 `runtime.yaml`；只在内容校验通过后调用。
pub(crate) fn write_runtime(paths: &AppPaths, runtime_yaml: &str) -> Result<()> {
    let file = &paths.runtime_mihomo_config_file;
    crate::platform::file::atomic_write(file, runtime_yaml.as_bytes())
        .with_context(|| format!("无法写入运行时配置：{}", file.display()))
}

/// 读取 `runtime.yaml` 中 `proxy-groups` 的组名顺序。
///
/// 订阅里策略组的定义顺序即代理页期望的展示顺序；文件缺失、解析失败
/// 或没有分组时返回空列表，由调用方回退到快照原序。
pub(crate) fn read_proxy_group_order(file: &Path) -> Vec<String> {
    let content = fs::read_to_string(file).unwrap_or_default();
    proxy_group_order_from_yaml(&content)
}

/// 从运行时配置文本提取 `proxy-groups` 的组名定义顺序。
fn proxy_group_order_from_yaml(runtime_yaml: &str) -> Vec<String> {
    // 部分订阅服务会在文件头带 UTF-8 BOM，与 parse_profile 同样先剥离。
    let runtime_yaml = runtime_yaml
        .strip_prefix('\u{feff}')
        .unwrap_or(runtime_yaml);
    let Ok(value) = serde_yaml::from_str::<Value>(runtime_yaml) else {
        return Vec::new();
    };
    value
        .get("proxy-groups")
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(|group| group.get("name").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

/// 节点接入信息，供概览页展示当前出站详情。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NodeEndpoint {
    /// 协议类型，如 `vless`、`ss`。
    pub(crate) kind: String,
    /// 服务器地址（域名或 IP）。
    pub(crate) server: String,
    /// 服务器端口。
    pub(crate) port: u16,
}

/// 读取 `runtime.yaml` 中全部节点的接入信息，按节点名索引。
///
/// 文件缺失或解析失败时返回空映射，调用方按"详情不可用"处理。
pub(crate) fn read_node_endpoints(file: &Path) -> std::collections::HashMap<String, NodeEndpoint> {
    let content = fs::read_to_string(file).unwrap_or_default();
    node_endpoints_from_yaml(&content)
}

/// 从运行时配置文本提取 `proxies` 列表的节点接入信息。
fn node_endpoints_from_yaml(runtime_yaml: &str) -> std::collections::HashMap<String, NodeEndpoint> {
    let runtime_yaml = runtime_yaml
        .strip_prefix('\u{feff}')
        .unwrap_or(runtime_yaml);
    let Ok(value) = serde_yaml::from_str::<Value>(runtime_yaml) else {
        return std::collections::HashMap::new();
    };
    value
        .get("proxies")
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(|proxy| {
            let name = proxy.get("name")?.as_str()?.to_owned();
            let kind = proxy.get("type")?.as_str()?.to_owned();
            let server = proxy.get("server")?.as_str()?.to_owned();
            let port = proxy.get("port")?.as_u64()? as u16;
            Some((name, NodeEndpoint { kind, server, port }))
        })
        .collect()
}

/// 校验配置文件是否能被内核接受；`config` 为完整 YAML 文本。
///
/// 会生成一个随包内核 `-t` 的短命子进程，可安全运行在后台线程。
pub(crate) fn validate_kernel_config(paths: &AppPaths, version: &str, config: &str) -> Result<()> {
    let executable = crate::kernel::bundled_path(paths, version)
        .with_context(|| format!("Mihomo 版本目录无效：{version}"))?;
    if !executable.is_file() {
        bail!("Mihomo 内核文件不存在：{}", executable.display());
    }

    // 校验产物先写临时文件；-t 通过与否都不影响 runtime.yaml。
    let temp = paths
        .mihomo_config_dir
        .join(format!("validate-{}.yaml", Uuid::new_v4().simple()));
    fs::write(&temp, config).with_context(|| format!("无法写入校验配置：{}", temp.display()))?;
    let result = crate::mihomo::process::validate_config(&executable, paths, &temp);
    let _ = fs::remove_file(&temp);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn baseline() -> LocalBaseline {
        LocalBaseline {
            mixed_port: 7890,
            controller_addr: "127.0.0.1:9097".to_owned(),
            secret: "test-secret".to_owned(),
            tun_enable: false,
        }
    }

    #[test]
    fn merge_keeps_profile_content_and_overrides_local_fields() {
        let profile = "\
mixed-port: 18080
allow-lan: true
external-controller: 0.0.0.0:9090
secret: leaked-from-subscription
unified-delay: false
tcp-concurrent: false
tun:
  enable: true
dns:
  enable: true
  nameserver:
  - 1.1.1.1
proxies:
- name: 节点一
  type: vless
  server: 1.2.3.4
  port: 443
  uuid: 00000000-0000-0000-0000-000000000000
proxy-groups:
- name: PROXY
  type: select
  proxies:
  - 节点一
rules:
- MATCH,PROXY
";
        let merged = merge_runtime(profile, &baseline()).expect("应完成合并");
        let value: Value = serde_yaml::from_str(&merged).expect("合并产物应是有效 YAML");
        let mapping = value.as_mapping().expect("合并产物应是映射");

        // 本地字段必须覆盖订阅。
        assert_eq!(
            mapping.get(Value::from("mixed-port")),
            Some(&Value::from(7890))
        );
        assert_eq!(
            mapping.get(Value::from("allow-lan")),
            Some(&Value::from(false))
        );
        assert_eq!(
            mapping.get(Value::from("bind-address")),
            Some(&Value::from("127.0.0.1"))
        );
        assert_eq!(
            mapping.get(Value::from("external-controller")),
            Some(&Value::from("127.0.0.1:9097"))
        );
        assert_eq!(
            mapping.get(Value::from("secret")),
            Some(&Value::from("test-secret"))
        );
        assert_eq!(
            mapping.get(Value::from("unified-delay")),
            Some(&Value::from(true))
        );
        assert_eq!(
            mapping.get(Value::from("tcp-concurrent")),
            Some(&Value::from(true))
        );
        // 额外监听端口必须关闭。
        assert_eq!(mapping.get(Value::from("port")), Some(&Value::from(0)));
        assert_eq!(
            mapping.get(Value::from("socks-port")),
            Some(&Value::from(0))
        );
        // TUN 必须被强制关闭。
        assert_eq!(
            mapping.get(Value::from("tun")).and_then(Value::as_mapping),
            Some(&{
                let mut tun = Mapping::new();
                tun.insert(Value::from("enable"), Value::from(false));
                tun
            })
        );
        // 行为字段与业务内容必须原样保留。
        assert_eq!(
            mapping
                .get(Value::from("dns"))
                .and_then(|dns| dns.get("enable")),
            Some(&Value::from(true))
        );
        assert!(mapping.contains_key(Value::from("proxies")));
        assert!(mapping.contains_key(Value::from("proxy-groups")));
        assert!(mapping.contains_key(Value::from("rules")));
    }

    #[test]
    fn proxy_group_order_follows_definition() {
        let runtime = "\
proxies: []
proxy-groups:
- name: 美国
  type: select
- name: 香港
  type: url-test
- name: 自动选择
  type: fallback
rules:
- MATCH,DIRECT
";
        assert_eq!(
            proxy_group_order_from_yaml(runtime),
            vec![
                "美国".to_string(),
                "香港".to_string(),
                "自动选择".to_string()
            ]
        );

        // BOM、无分组、非法 YAML 都不产生顺序，调用方回退快照原序。
        assert_eq!(
            proxy_group_order_from_yaml(&format!("\u{feff}{runtime}")),
            vec![
                "美国".to_string(),
                "香港".to_string(),
                "自动选择".to_string()
            ]
        );
        assert!(proxy_group_order_from_yaml("rules:\n- MATCH,DIRECT").is_empty());
        assert!(proxy_group_order_from_yaml(":: 不是映射").is_empty());
    }

    #[test]
    fn node_endpoints_follow_proxies_list() {
        let runtime = "\
mixed-port: 7890
proxies:
- name: 美国主力
  type: vless
  server: us-1.example.com
  port: 443
- name: 缺字段的坏节点
  type: ss
  server: us-2.example.com
- name: 备用
  type: shadowsocks
  server: 203.0.113.7
  port: 8388
proxy-groups:
- name: PROXY
  type: select
";
        let endpoints = node_endpoints_from_yaml(runtime);
        assert_eq!(endpoints.len(), 2);
        let main = endpoints.get("美国主力").expect("主力节点应有接入信息");
        assert_eq!(
            main,
            &NodeEndpoint {
                kind: "vless".into(),
                server: "us-1.example.com".into(),
                port: 443,
            }
        );
        // server 或 port 缺失的节点不收录，避免展示半截信息。
        assert!(!endpoints.contains_key("缺字段的坏节点"));
        assert_eq!(endpoints.get("备用").unwrap().port, 8388);
        assert!(node_endpoints_from_yaml(":: 不是映射").is_empty());
    }

    #[test]
    fn tun_baseline_controls_merged_config() {
        // 开启：注入推荐栈配置。
        let mut enabled = baseline();
        enabled.tun_enable = true;
        let merged = merge_runtime("rules:\n- MATCH,DIRECT", &enabled).expect("应完成合并");
        let value: Value = serde_yaml::from_str(&merged).expect("合并产物应是有效 YAML");
        let tun = value
            .get("tun")
            .and_then(Value::as_mapping)
            .expect("应注入 TUN 配置");
        assert_eq!(tun.get(Value::from("enable")), Some(&Value::from(true)));
        #[cfg(target_os = "linux")]
        assert_eq!(tun.get(Value::from("stack")), Some(&Value::from("gvisor")));
        #[cfg(not(target_os = "linux"))]
        assert_eq!(tun.get(Value::from("stack")), Some(&Value::from("mixed")));
        assert_eq!(tun.get(Value::from("auto-route")), Some(&Value::from(true)));
        assert_eq!(
            tun.get(Value::from("strict-route")),
            Some(&Value::from(false))
        );
        // 沿用 Mihomo 默认设备名；Linux 与同机 Clash Verge Rev 的工作配置一致。
        assert!(tun.get(Value::from("device")).is_none());
        assert_eq!(value.get("ipv6"), Some(&Value::from(true)));
        #[cfg(target_os = "linux")]
        assert!(tun.get(Value::from("inet6-address")).is_none());
        #[cfg(not(target_os = "linux"))]
        assert_eq!(
            tun.get(Value::from("inet6-address")),
            Some(&Value::Sequence(vec![Value::from("fdfe:dcba:9876::1/126")]))
        );
        // 劫持系统 DNS 后必须有上游可查，否则所有域名解析失败。
        let dns = value
            .get("dns")
            .and_then(Value::as_mapping)
            .expect("开启 TUN 应注入 DNS 配置");
        assert_eq!(dns.get(Value::from("enable")), Some(&Value::from(true)));
        assert_eq!(
            dns.get(Value::from("enhanced-mode")),
            Some(&Value::from("fake-ip"))
        );
        #[cfg(target_os = "linux")]
        {
            assert_eq!(dns.get(Value::from("ipv6")), Some(&Value::from(true)));
            assert_eq!(
                dns.get(Value::from("fake-ip-range6")),
                Some(&Value::from("fdfe:dcba:9876::1/64"))
            );
        }
        #[cfg(not(target_os = "linux"))]
        {
            assert_eq!(dns.get(Value::from("ipv6")), Some(&Value::from(false)));
            assert!(dns.get(Value::from("fake-ip-range6")).is_none());
        }
        assert!(dns.get(Value::from("nameserver")).is_some());

        // 关闭：订阅即使自带 TUN 配置也会被覆盖为禁用。
        let profile = "tun:\n  enable: true\nrules:\n- MATCH,DIRECT";
        let merged = merge_runtime(profile, &baseline()).expect("应完成合并");
        let value: Value = serde_yaml::from_str(&merged).expect("合并产物应是有效 YAML");
        let tun = value
            .get("tun")
            .and_then(Value::as_mapping)
            .expect("TUN 配置节应存在");
        assert_eq!(tun.get(Value::from("enable")), Some(&Value::from(false)));
        // 关闭 TUN 后 IPv6 回到禁用，也不再注入 DNS，保留订阅自带配置。
        assert_eq!(value.get("ipv6"), Some(&Value::from(false)));
        assert!(value.get("dns").is_none());

        // 基线文件序列化往返保留 TUN 开关。
        let roundtrip = parse_baseline(&enabled.to_yaml()).expect("基线往返应成功");
        assert_eq!(roundtrip, enabled);
    }

    #[test]
    fn merge_strips_external_ui() {
        let profile = "external-ui: ui\nrules:\n- MATCH,DIRECT\n";
        let merged = merge_runtime(profile, &baseline()).expect("应完成合并");
        let value: Value = serde_yaml::from_str(&merged).expect("应是有效 YAML");
        let mapping = value.as_mapping().expect("应是映射");
        assert!(!mapping.contains_key(Value::from("external-ui")));
        assert!(!mapping.contains_key(Value::from("external-ui-name")));
    }

    #[test]
    fn merge_rejects_invalid_profiles() {
        assert!(merge_runtime("", &baseline()).is_err(), "空配置应被拒绝");
        assert!(
            merge_runtime("- a\n- b\n", &baseline()).is_err(),
            "顶层列表应被拒绝"
        );
        assert!(
            merge_runtime("proxies: not-a-list\n", &baseline()).is_err(),
            "proxies 类型错误应被拒绝"
        );
        // 空 proxies 属合法配置（仅内置 DIRECT），由内核 -t 终审。
        assert!(merge_runtime("proxies: []\n", &baseline()).is_ok());
    }

    #[test]
    fn merge_tolerates_bom_and_rules_only_profiles() {
        let profile = "\u{feff}rules:\n- MATCH,DIRECT\n";
        let merged = merge_runtime(profile, &baseline()).expect("带 BOM 的合法配置应通过");
        assert!(merged.contains("MATCH,DIRECT"));
    }

    #[test]
    fn baseline_roundtrip_through_yaml() {
        let original = baseline();
        let yaml = original.to_yaml();
        let parsed = parse_baseline(&yaml).expect("基线 YAML 应回读");
        assert_eq!(parsed, original);
    }

    #[test]
    fn baseline_parse_rejects_missing_fields() {
        assert!(parse_baseline("mixed-port: 7890\n").is_err());
        assert!(
            parse_baseline("mixed-port: 0\nexternal-controller: 127.0.0.1:9097\nsecret: s\n")
                .is_err()
        );
        assert!(parse_baseline("not-a-mapping").is_err());
    }
}
