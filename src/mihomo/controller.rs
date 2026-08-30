//! Mihomo external controller 的 REST 客户端。
//!
//! controller 由内核启动配置注入，只监听本机回环地址；客户端以随机 secret
//! 通过 `Authorization: Bearer` 认证。所有方法都是阻塞 HTTP 调用，调用方
//! 必须放在 GPUI 后台线程执行，不得阻塞主线程。

use std::{collections::BTreeMap, io::Read, time::Duration};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::mihomo::config::LocalBaseline;

/// 下载订阅时的体积上限；正常订阅远小于该值，超出视为异常响应。
pub(crate) const MAX_DOWNLOAD_BYTES: u64 = 10 * 1024 * 1024;

/// 延迟测试默认探测地址；与主流 Clash 面板一致，返回 204。
pub(crate) const DELAY_TEST_URL: &str = "https://www.gstatic.com/generate_204";

/// 本机 controller 客户端；实例可跨线程共享，方法均可重入。
#[derive(Clone)]
pub(crate) struct Controller {
    base_url: String,
    secret: String,
    agent: ureq::Agent,
    /// 延迟测试的内核侧超时可达数秒，HTTP 超时必须宽于探测时长。
    slow_agent: ureq::Agent,
}

impl Controller {
    /// 根据本地基线构造 controller 客户端。
    pub(crate) fn new(baseline: &LocalBaseline) -> Self {
        let agent = ureq::AgentBuilder::new()
            // 本机回环调用，短超时足够；失败由调用方转为界面错误。
            .timeout(Duration::from_secs(3))
            .build();
        let slow_agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(10))
            .build();
        Self {
            base_url: format!("http://{}", baseline.controller_addr),
            secret: baseline.secret.clone(),
            agent,
            slow_agent,
        }
    }

    /// 探测内核 controller 是否就绪，返回内核版本号。
    pub(crate) fn version(&self) -> Result<String> {
        #[derive(Deserialize)]
        struct VersionResponse {
            version: String,
        }

        let value: VersionResponse = self
            .agent
            .get(&format!("{}/version", self.base_url))
            .set("Authorization", &self.authorization())
            .call()
            .map_err(|error| anyhow::anyhow!("{error}"))
            .context("无法连接 Mihomo controller")?
            .into_json()
            .context("controller 版本响应格式无效")?;
        Ok(value.version)
    }

    /// 读取当前运行配置摘要；目前客户端只关心运行模式。
    pub(crate) fn configs(&self) -> Result<ConfigSnapshot> {
        #[derive(Deserialize)]
        struct ConfigsResponse {
            #[serde(default)]
            mode: String,
            #[serde(default)]
            tun: Option<TunState>,
        }

        #[derive(Deserialize)]
        struct TunState {
            #[serde(default)]
            enable: bool,
        }

        let value: ConfigsResponse = self
            .agent
            .get(&format!("{}/configs", self.base_url))
            .set("Authorization", &self.authorization())
            .call()
            .map_err(|error| anyhow::anyhow!("{error}"))
            .context("无法读取 Mihomo 运行配置")?
            .into_json()
            .context("controller 配置响应格式无效")?;
        Ok(ConfigSnapshot {
            mode: parse_mode(&value.mode),
            // 内核可能在启动时静默降级 TUN（缺权限或驱动），以此核对真实生效状态。
            tun_enabled: value.tun.is_some_and(|tun| tun.enable),
        })
    }

    /// 切换运行模式：`rule` / `global` / `direct`。
    pub(crate) fn patch_mode(&self, mode: &str) -> Result<()> {
        self.agent
            .patch(&format!("{}/configs", self.base_url))
            .set("Authorization", &self.authorization())
            .set("Content-Type", "application/json")
            .send_json(ureq::json!({ "mode": mode }))
            .map_err(|error| anyhow::anyhow!("{error}"))
            .with_context(|| format!("无法切换运行模式到 {mode}"))?;
        Ok(())
    }

    /// 读取全部代理与策略组；`mode` 用于决定是否包含 GLOBAL 组。
    pub(crate) fn proxies(&self, mode: Mode) -> Result<ProxiesSnapshot> {
        let value: ProxiesResponse = self
            .agent
            .get(&format!("{}/proxies", self.base_url))
            .set("Authorization", &self.authorization())
            .call()
            .map_err(|error| anyhow::anyhow!("{error}"))
            .context("无法读取代理列表")?
            .into_json()
            .context("controller 代理响应格式无效")?;

        Ok(ProxiesSnapshot {
            groups: group_snapshots(&value.proxies, mode),
        })
    }

    /// 在策略组内选择当前节点；只对 Selector 组有效。
    pub(crate) fn select_proxy(&self, group: &str, node: &str) -> Result<()> {
        self.agent
            .put(&format!(
                "{}/proxies/{}",
                self.base_url,
                uri_path_segment(group)
            ))
            .set("Authorization", &self.authorization())
            .set("Content-Type", "application/json")
            .send_json(ureq::json!({ "name": node }))
            .map_err(|error| anyhow::anyhow!("{error}"))
            .with_context(|| format!("无法在策略组 {group} 中选择节点"))?;
        Ok(())
    }

    /// 读取连接快照：活跃连接列表与会话累计流量，用于连接页与实时网速。
    pub(crate) fn connections(&self) -> Result<ConnectionsSnapshot> {
        let value: ConnectionsResponse = self
            .agent
            .get(&format!("{}/connections", self.base_url))
            .set("Authorization", &self.authorization())
            .call()
            .map_err(|error| anyhow::anyhow!("{error}"))
            .context("无法读取连接数据")?
            .into_json()
            .context("controller 连接响应格式无效")?;
        Ok(value.into())
    }

    /// 关闭单个连接。
    pub(crate) fn close_connection(&self, id: &str) -> Result<()> {
        self.agent
            .delete(&format!(
                "{}/connections/{}",
                self.base_url,
                uri_path_segment(id)
            ))
            .set("Authorization", &self.authorization())
            .call()
            .map_err(|error| anyhow::anyhow!("{error}"))
            .context("无法关闭连接")?;
        Ok(())
    }

    /// 关闭全部连接。
    pub(crate) fn close_all_connections(&self) -> Result<()> {
        self.agent
            .delete(&format!("{}/connections", self.base_url))
            .set("Authorization", &self.authorization())
            .call()
            .map_err(|error| anyhow::anyhow!("{error}"))
            .context("无法关闭全部连接")?;
        Ok(())
    }

    /// 对单个代理执行延迟测试；失败（超时/拒绝）返回错误描述。
    pub(crate) fn proxy_delay(&self, name: &str) -> Result<u64> {
        let value: DelayResponse = self
            .slow_agent
            .get(&format!(
                "{}/proxies/{}/delay?url={}&timeout={DELAY_TIMEOUT_MS}",
                self.base_url,
                uri_path_segment(name),
                uri_path_segment(DELAY_TEST_URL)
            ))
            .set("Authorization", &self.authorization())
            .call()
            .map_err(|error| anyhow::anyhow!("{error}"))
            .with_context(|| format!("节点 {name} 延迟测试失败"))?
            .into_json()
            .context("延迟测试响应格式无效")?;
        let message = value
            .message
            .unwrap_or_else(|| "内核未返回延迟数据".to_owned());
        value.delay.with_context(|| message)
    }

    /// 对策略组内全部节点执行延迟测试；返回节点名到延迟毫秒的映射。
    pub(crate) fn group_delay(&self, name: &str) -> Result<BTreeMap<String, u64>> {
        let value: BTreeMap<String, u64> = self
            .slow_agent
            .get(&format!(
                "{}/group/{}/delay?url={}&timeout={DELAY_TIMEOUT_MS}",
                self.base_url,
                uri_path_segment(name),
                uri_path_segment(DELAY_TEST_URL)
            ))
            .set("Authorization", &self.authorization())
            .call()
            .map_err(|error| anyhow::anyhow!("{error}"))
            .with_context(|| format!("策略组 {name} 延迟测试失败"))?
            .into_json()
            .context("策略组延迟测试响应格式无效")?;
        Ok(value)
    }

    /// 下载订阅内容；带体积上限，返回 UTF-8 文本。
    pub(crate) fn download_subscription(url: &str) -> Result<String> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build();
        let response = agent
            .get(url)
            .set("User-Agent", "clash.meta/pure-clash")
            .call()
            .map_err(|error| anyhow::anyhow!("{error}"))
            .context("订阅下载失败")?;

        let length = response
            .header("Content-Length")
            .and_then(|length| length.parse::<u64>().ok());
        if let Some(length) = length
            && length > MAX_DOWNLOAD_BYTES
        {
            bail!("订阅内容超过 10MB 体积上限");
        }

        let mut reader = response.into_reader().take(MAX_DOWNLOAD_BYTES + 1);
        let mut body = Vec::new();
        reader.read_to_end(&mut body).context("订阅下载中断")?;
        if body.len() as u64 > MAX_DOWNLOAD_BYTES {
            bail!("订阅内容超过 10MB 体积上限");
        }

        String::from_utf8(body).map_err(|_| anyhow::anyhow!("订阅内容不是有效的 UTF-8 文本"))
    }

    fn authorization(&self) -> String {
        format!("Bearer {}", self.secret)
    }
}

/// 运行模式；与 controller 的 `mode` 字段一一对应。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Rule,
    Global,
    Direct,
}

impl Mode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Global => "global",
            Self::Direct => "direct",
        }
    }

    pub(crate) fn from_str(value: &str) -> Self {
        match value {
            "global" => Self::Global,
            "direct" => Self::Direct,
            _ => Self::Rule,
        }
    }
}

fn parse_mode(value: &str) -> Mode {
    Mode::from_str(value)
}

/// 运行配置摘要。
#[derive(Clone, Copy, Debug)]
pub(crate) struct ConfigSnapshot {
    pub(crate) mode: Mode,
    /// 内核侧 TUN 的真实生效状态。
    pub(crate) tun_enabled: bool,
}

/// 延迟测试的内核侧超时；与主流面板默认值一致。
pub(crate) const DELAY_TIMEOUT_MS: u64 = 5000;

/// controller `/connections` 响应。
#[derive(Deserialize)]
struct ConnectionsResponse {
    #[serde(rename = "downloadTotal", default)]
    download_total: Option<u64>,
    #[serde(rename = "uploadTotal", default)]
    upload_total: Option<u64>,
    #[serde(default)]
    connections: Option<Vec<ConnectionItem>>,
}

/// 连接与流量快照。
#[derive(Clone, Debug, Default)]
pub(crate) struct ConnectionsSnapshot {
    /// 内核启动以来的累计下载字节数。
    pub(crate) download_total: u64,
    /// 内核启动以来的累计上传字节数。
    pub(crate) upload_total: u64,
    pub(crate) connections: Vec<ConnectionItem>,
}

impl From<ConnectionsResponse> for ConnectionsSnapshot {
    fn from(response: ConnectionsResponse) -> Self {
        Self {
            download_total: response.download_total.unwrap_or(0),
            upload_total: response.upload_total.unwrap_or(0),
            connections: response.connections.unwrap_or_default(),
        }
    }
}

/// 单条活跃连接。
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ConnectionItem {
    #[serde(default)]
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) upload: u64,
    #[serde(default)]
    pub(crate) download: u64,
    #[serde(default)]
    pub(crate) start: String,
    /// 代理链路数组；首个元素为实际出口节点。
    #[serde(default)]
    pub(crate) chains: Vec<String>,
    #[serde(default)]
    pub(crate) rule: String,
    #[serde(default)]
    pub(crate) rule_payload: String,
    #[serde(default)]
    pub(crate) metadata: ConnectionMetadata,
}

impl ConnectionItem {
    /// 界面展示的目标地址：优先嗅探到的域名，回退目标 IP:端口。
    pub(crate) fn target(&self) -> String {
        let metadata = &self.metadata;
        if !metadata.host.is_empty() {
            return format!("{}:{}", metadata.host, metadata.destination_port);
        }
        if !metadata.destination_ip.is_empty() {
            return format!("{}:{}", metadata.destination_ip, metadata.destination_port);
        }
        "—".to_owned()
    }

    /// 出口节点；无链路时回退 DIRECT。
    pub(crate) fn chain(&self) -> &str {
        self.chains.first().map(String::as_str).unwrap_or("DIRECT")
    }

    /// 命中规则与内容；规则为空（如直连探测）时显示占位。
    pub(crate) fn rule_label(&self) -> String {
        if self.rule.is_empty() {
            return "—".to_owned();
        }
        if self.rule_payload.is_empty() {
            self.rule.clone()
        } else {
            format!("{} · {}", self.rule, self.rule_payload)
        }
    }
}

/// 连接元数据；字段来自内核实测响应，未知字段直接忽略。
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct ConnectionMetadata {
    #[serde(default)]
    pub(crate) network: String,
    #[serde(default)]
    pub(crate) host: String,
    #[serde(rename = "destinationIP", default)]
    pub(crate) destination_ip: String,
    #[serde(rename = "destinationPort", default)]
    pub(crate) destination_port: String,
    #[serde(default)]
    pub(crate) process: String,
}

/// 延迟测试响应；失败时内核可能只返回 message。
#[derive(Deserialize)]
struct DelayResponse {
    #[serde(default)]
    delay: Option<u64>,
    #[serde(default)]
    message: Option<String>,
}

/// controller `/proxies` 响应。
#[derive(Deserialize)]
struct ProxiesResponse {
    #[serde(default)]
    proxies: std::collections::HashMap<String, ProxyItem>,
}

/// controller `/proxies` 响应中的单个代理或策略组条目。
#[derive(Deserialize)]
struct ProxyItem {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    now: Option<String>,
    #[serde(default)]
    all: Vec<String>,
    /// 延迟测试历史；取最近一次有效值作为界面展示的缓存延迟。
    #[serde(default)]
    history: Vec<DelayHistory>,
}

/// 延迟测试历史条目。
#[derive(Deserialize)]
struct DelayHistory {
    #[serde(default)]
    delay: u64,
}

/// 将 controller 的代理映射整理为分组快照；组名排序、GLOBAL 置顶。
fn group_snapshots(
    proxies: &std::collections::HashMap<String, ProxyItem>,
    mode: Mode,
) -> Vec<GroupSnapshot> {
    let group_names: std::collections::HashSet<&str> = proxies
        .iter()
        .filter(|(_, item)| is_group_type(&item.r#type))
        .map(|(name, _)| name.as_str())
        .collect();
    let mut groups: Vec<GroupSnapshot> = proxies
        .iter()
        .filter(|(_, item)| is_group_type(&item.r#type))
        // GLOBAL 只在全局模式下参与分流，规则/直连模式不展示。
        .filter(|(name, _)| mode == Mode::Global || name.as_str() != "GLOBAL")
        .map(|(name, item)| {
            let nodes = item
                .all
                .iter()
                // GLOBAL 由内核自动生成，成员是全部代理与策略组的并集；
                // 策略组在页面已有独立分区，这里不再作为 GLOBAL 的节点展示。
                .filter(|node| name != "GLOBAL" || !group_names.contains(node.as_str()))
                .map(|node| {
                    proxies.get(node).map_or_else(
                        || NodeSnapshot {
                            name: node.clone(),
                            kind: String::new(),
                            delay: None,
                        },
                        |proxy| NodeSnapshot {
                            name: node.clone(),
                            kind: proxy.r#type.clone(),
                            delay: last_delay(&proxy.history),
                        },
                    )
                })
                .collect();
            GroupSnapshot {
                name: name.clone(),
                now: item.now.clone().unwrap_or_default(),
                // 只有 Selector 允许用户手动选择节点。
                selectable: item.r#type == "Selector",
                nodes,
            }
        })
        .collect();
    // 控制器返回的是无序映射，组名排序保证界面稳定。
    groups.sort_by(|left, right| left.name.cmp(&right.name));
    // GLOBAL 组置顶，便于全局模式下快速定位。
    if let Some(position) = groups.iter().position(|group| group.name == "GLOBAL") {
        let global = groups.remove(position);
        groups.insert(0, global);
    }
    groups
}

/// 代理组快照。
#[derive(Clone, Debug)]
pub(crate) struct GroupSnapshot {
    pub(crate) name: String,
    /// 当前选中的节点名；自动组同样提供该字段。
    pub(crate) now: String,
    /// 是否允许用户手动选择（Selector）。
    pub(crate) selectable: bool,
    pub(crate) nodes: Vec<NodeSnapshot>,
}

/// 代理节点快照。
#[derive(Clone, Debug)]
pub(crate) struct NodeSnapshot {
    pub(crate) name: String,
    /// 节点协议类型，如 `vless`、`ss`；未知时为空。
    pub(crate) kind: String,
    /// 最近一次延迟测试结果（毫秒）；从未测试或测试失败时为空。
    pub(crate) delay: Option<u64>,
}

/// 取延迟历史中最近一次有效延迟；内核会把失败记为 0，需要跳过。
fn last_delay(history: &[DelayHistory]) -> Option<u64> {
    history
        .iter()
        .rev()
        .find(|entry| entry.delay > 0)
        .map(|entry| entry.delay)
}

/// 代理页数据快照。
#[derive(Clone, Debug, Default)]
pub(crate) struct ProxiesSnapshot {
    pub(crate) groups: Vec<GroupSnapshot>,
}

/// 判断代理类型是否为策略组；普通节点类型不在分组视图展示。
fn is_group_type(kind: &str) -> bool {
    matches!(
        kind,
        "Selector" | "URLTest" | "Fallback" | "LoadBalance" | "Relay"
    )
}

/// 组名作为 URL 路径段时做百分号编码，避免中文或特殊字符破坏路由。
fn uri_path_segment(segment: &str) -> String {
    let mut encoded = String::new();
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                encoded.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_roundtrip() {
        for mode in [Mode::Rule, Mode::Global, Mode::Direct] {
            assert_eq!(Mode::from_str(mode.as_str()), mode);
        }
        // 未知值回退到规则模式。
        assert_eq!(Mode::from_str("unknown"), Mode::Rule);
    }

    #[test]
    fn group_type_detection() {
        assert!(is_group_type("Selector"));
        assert!(is_group_type("URLTest"));
        assert!(!is_group_type("Vless"));
        assert!(!is_group_type("Shadowsocks"));
    }

    #[test]
    fn uri_segment_encodes_special_characters() {
        assert_eq!(uri_path_segment("PROXY"), "PROXY");
        assert_eq!(
            uri_path_segment("美国 主力"),
            "%E7%BE%8E%E5%9B%BD%20%E4%B8%BB%E5%8A%9B"
        );
        assert_eq!(uri_path_segment("a/b?c"), "a%2Fb%3Fc");
    }

    #[test]
    fn global_group_hides_nested_policy_groups() {
        // 模拟内核响应：GLOBAL 成员是全部代理与策略组的并集。
        let mut proxies = std::collections::HashMap::new();
        proxies.insert(
            "GLOBAL".to_string(),
            ProxyItem {
                r#type: "Selector".into(),
                now: Some("PROXY".into()),
                all: vec![
                    "DIRECT".into(),
                    "REJECT".into(),
                    "美国主力".into(),
                    "PROXY".into(),
                ],
                history: vec![],
            },
        );
        proxies.insert(
            "PROXY".to_string(),
            ProxyItem {
                r#type: "Selector".into(),
                now: Some("美国主力".into()),
                all: vec!["美国主力".into(), "美国备用".into()],
                history: vec![],
            },
        );
        proxies.insert(
            "DIRECT".to_string(),
            ProxyItem {
                r#type: "Direct".into(),
                now: None,
                all: vec![],
                history: vec![],
            },
        );
        proxies.insert(
            "REJECT".to_string(),
            ProxyItem {
                r#type: "Reject".into(),
                now: None,
                all: vec![],
                history: vec![],
            },
        );
        proxies.insert(
            "美国主力".to_string(),
            ProxyItem {
                r#type: "Vless".into(),
                now: None,
                all: vec![],
                history: vec![],
            },
        );
        proxies.insert(
            "美国备用".to_string(),
            ProxyItem {
                r#type: "Vless".into(),
                now: None,
                all: vec![],
                history: vec![],
            },
        );

        let groups = group_snapshots(&proxies, Mode::Global);
        // GLOBAL 置顶，内嵌的 PROXY 组不作为节点，内置 DIRECT/REJECT 保留。
        assert_eq!(groups[0].name, "GLOBAL");
        let global_nodes: Vec<&str> = groups[0]
            .nodes
            .iter()
            .map(|node| node.name.as_str())
            .collect();
        assert_eq!(global_nodes, vec!["DIRECT", "REJECT", "美国主力"]);

        // 订阅策略组不受影响，普通节点照常展示。
        let proxy_group = groups.iter().find(|group| group.name == "PROXY").unwrap();
        let proxy_nodes: Vec<&str> = proxy_group
            .nodes
            .iter()
            .map(|node| node.name.as_str())
            .collect();
        assert_eq!(proxy_nodes, vec!["美国主力", "美国备用"]);
        assert!(proxy_group.selectable);
    }

    #[test]
    fn global_group_only_visible_in_global_mode() {
        let mut proxies = std::collections::HashMap::new();
        proxies.insert(
            "GLOBAL".to_string(),
            ProxyItem {
                r#type: "Selector".into(),
                now: Some("DIRECT".into()),
                all: vec!["DIRECT".into()],
                history: vec![],
            },
        );
        proxies.insert(
            "DIRECT".to_string(),
            ProxyItem {
                r#type: "Direct".into(),
                now: None,
                all: vec![],
                history: vec![],
            },
        );

        assert_eq!(group_snapshots(&proxies, Mode::Global).len(), 1);
        // 规则/直连模式下不展示 GLOBAL 组。
        assert!(group_snapshots(&proxies, Mode::Rule).is_empty());
        assert!(group_snapshots(&proxies, Mode::Direct).is_empty());
    }

    #[test]
    fn last_delay_skips_failed_entries() {
        // 内核把失败延迟记为 0，界面应取最近一次成功值。
        let history = vec![
            DelayHistory { delay: 120 },
            DelayHistory { delay: 0 },
            DelayHistory { delay: 0 },
        ];
        assert_eq!(last_delay(&history), Some(120));
        assert_eq!(last_delay(&[DelayHistory { delay: 0 }]), None);
        assert_eq!(last_delay(&[]), None);
    }

    #[test]
    fn connections_snapshot_parses_probe_response() {
        // 结构来自 1.19.30 内核实机响应（含限速下载建立的活跃连接）。
        let body = r#"{
            "downloadTotal": 2015106,
            "uploadTotal": 3729,
            "memory": 0,
            "connections": [{
                "id": "980c3fab-ed27-434e-a2f2-388cbc6840d4",
                "metadata": {
                    "network": "tcp",
                    "type": "HTTPS",
                    "sourceIP": "127.0.0.1",
                    "destinationIP": "",
                    "sourcePort": "39956",
                    "destinationPort": "443",
                    "host": "speed.cloudflare.com",
                    "dnsMode": "normal",
                    "process": "",
                    "processPath": "",
                    "remoteDestination": "172.66.0.218"
                },
                "upload": 1790,
                "download": 2009967,
                "start": "2026-08-30T15:35:09.826432755+08:00",
                "chains": ["DIRECT"],
                "providerChains": [""],
                "rule": "",
                "rulePayload": ""
            }]
        }"#;
        let snapshot: ConnectionsSnapshot = serde_json::from_str::<ConnectionsResponse>(body)
            .unwrap()
            .into();
        assert_eq!(snapshot.download_total, 2015106);
        assert_eq!(snapshot.upload_total, 3729);
        assert_eq!(snapshot.connections.len(), 1);
        let connection = &snapshot.connections[0];
        assert_eq!(connection.target(), "speed.cloudflare.com:443");
        assert_eq!(connection.chain(), "DIRECT");
        assert_eq!(connection.rule_label(), "—");
    }

    #[test]
    fn connections_snapshot_tolerates_null_and_missing_fields() {
        // 空闲内核返回 connections: null；null 解析为 None，快照转换折叠为空数组。
        let response: ConnectionsResponse =
            serde_json::from_str(r#"{"downloadTotal": 5, "connections": null}"#).unwrap();
        let snapshot: ConnectionsSnapshot = response.into();
        assert!(snapshot.connections.is_empty());
        assert_eq!(snapshot.download_total, 5);

        let minimal: ConnectionsResponse = serde_json::from_str("{}").unwrap();
        assert_eq!(minimal.download_total, None);

        let snapshot: ConnectionsSnapshot = minimal.into();
        assert_eq!(snapshot.download_total, 0);
    }

    #[test]
    fn connection_target_falls_back_to_destination_ip() {
        let metadata = ConnectionMetadata {
            network: "tcp".into(),
            host: String::new(),
            destination_ip: "1.2.3.4".into(),
            destination_port: "80".into(),
            process: "curl".into(),
        };
        let connection = ConnectionItem {
            id: "id".into(),
            upload: 1,
            download: 2,
            start: String::new(),
            chains: vec!["美国 01".into()],
            rule: "Match".into(),
            rule_payload: String::new(),
            metadata,
        };
        assert_eq!(connection.target(), "1.2.3.4:80");
        assert_eq!(connection.chain(), "美国 01");
        assert_eq!(connection.rule_label(), "Match");
        // 规则带内容时合并展示。
        let mut with_payload = connection.clone();
        with_payload.rule_payload = "google".into();
        assert_eq!(with_payload.rule_label(), "Match · google");
    }
}
