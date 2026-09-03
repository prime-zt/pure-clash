//! 配置文件（订阅）管理：下载、结构校验、内核校验与落盘。
//!
//! 管线约定：任何新内容（下载或导入）必须依次通过
//! 1. YAML 结构预检与本地基线合并（[`merge_runtime`]）；
//! 2. 随包内核 `-t` 终审（[`validate_kernel_config`]）；
//!
//! 全部通过后才写入 `profiles/<id>.yaml`；激活时再把合并产物写入
//! `runtime.yaml` 并按需重启内核。

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use uuid::Uuid;

use crate::{
    config::ProfileMeta,
    mihomo::{
        config::{ensure_baseline, merge_runtime, validate_kernel_config, write_runtime},
        controller::{Controller, MAX_DOWNLOAD_BYTES},
    },
    platform::AppPaths,
};

/// 单个配置文件的磁盘路径。
pub(crate) fn profile_yaml_path(paths: &AppPaths, id: &str) -> PathBuf {
    paths.profiles_dir.join(format!("{id}.yaml"))
}

/// 读取已保存的配置内容。
pub(crate) fn read_profile(paths: &AppPaths, id: &str) -> Result<String> {
    let path = profile_yaml_path(paths, id);
    fs::read_to_string(&path).with_context(|| format!("无法读取配置：{}", path.display()))
}

/// 删除配置文件；文件不存在时视为成功。
pub(crate) fn delete_profile_file(paths: &AppPaths, id: &str) -> Result<()> {
    let path = profile_yaml_path(paths, id);
    if path.exists() {
        fs::remove_file(&path).with_context(|| format!("无法删除配置：{}", path.display()))?;
    }
    Ok(())
}

/// 当前 UNIX 时间戳（秒）。
pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

/// 校验并保存订阅/导入内容，返回合并产物供后续激活使用。
///
/// 内容只有通过结构预检与内核 `-t` 校验才会落盘，失败时不产生任何残留。
pub(crate) fn validate_and_store(
    paths: &AppPaths,
    version: &str,
    id: &str,
    content: &str,
) -> Result<String> {
    let baseline = ensure_baseline(paths)?;
    let runtime = merge_runtime(content, &baseline)?;
    validate_kernel_config(paths, version, &runtime)?;
    let profile_file = profile_yaml_path(paths, id);
    crate::platform::file::atomic_write(&profile_file, content.as_bytes())
        .with_context(|| format!("无法写入配置：{}", profile_file.display()))?;
    Ok(runtime)
}

/// 下载订阅内容；失败信息已脱敏为站点主机，不含完整 URL。
pub(crate) fn download_subscription(url: &str) -> Result<String> {
    Controller::download_subscription(url)
}

/// 读取用户选择的本地 Mihomo YAML；沿用订阅 10MB 上限并要求 UTF-8 文本。
pub(crate) fn read_local_config(path: &Path) -> Result<String> {
    let metadata =
        fs::metadata(path).with_context(|| format!("无法读取本地配置：{}", path.display()))?;
    if !metadata.is_file() {
        bail!("选择的路径不是文件");
    }
    if metadata.len() > MAX_DOWNLOAD_BYTES {
        bail!("本地配置超过 10MB 体积上限");
    }
    let bytes = fs::read(path).with_context(|| format!("无法读取本地配置：{}", path.display()))?;
    if bytes.len() as u64 > MAX_DOWNLOAD_BYTES {
        bail!("本地配置超过 10MB 体积上限");
    }
    String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("本地配置不是有效的 UTF-8 文本"))
}

/// 从本地文件名派生默认显示名；路径本身不会写入配置元数据。
pub(crate) fn default_name_from_path(path: &Path) -> Option<String> {
    let name = path.file_stem()?.to_string_lossy().trim().to_owned();
    (!name.is_empty() && !name.starts_with('.')).then_some(name)
}

/// 从订阅 URL 派生默认显示名：取主机名部分。
pub(crate) fn default_name_from_url(url: &str) -> Option<String> {
    let rest = url.split("://").nth(1)?;
    let host = rest.split(['/', '?', '#']).next()?;
    if host.is_empty() {
        None
    } else {
        Some(host.to_owned())
    }
}

/// 校验 URL 基本形状；只接受 http/https。
pub(crate) fn validate_subscription_url(url: &str) -> Result<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        bail!("订阅地址必须以 http:// 或 https:// 开头");
    }
    if url.len() <= 8 {
        bail!("订阅地址不完整");
    }
    Ok(())
}

/// 生成新的随机配置标识。
pub(crate) fn new_profile_id() -> String {
    Uuid::new_v4().simple().to_string()
}

/// 把当前激活态同步到 runtime.yaml；`active_id` 为 None 时回退到内置默认配置。
pub(crate) fn sync_runtime_file(
    paths: &AppPaths,
    baseline: Option<&crate::mihomo::config::LocalBaseline>,
    version: &str,
    active_id: Option<&str>,
) -> Result<()> {
    let runtime = prepare_runtime(paths, baseline, version, active_id)?;
    write_runtime(paths, &runtime)
}

/// 在内存中生成当前激活配置的 runtime，并用目标版本内核完成终审。
///
/// 成功返回完整 YAML，但不修改磁盘；调用方可在其他状态准备就绪后原子提交。
pub(crate) fn prepare_runtime(
    paths: &AppPaths,
    baseline: Option<&crate::mihomo::config::LocalBaseline>,
    version: &str,
    active_id: Option<&str>,
) -> Result<String> {
    let Some(baseline) = baseline else {
        bail!("本地基线不可用，无法生成运行时配置");
    };
    let runtime = match active_id {
        Some(id) => {
            let content = read_profile(paths, id)?;
            merge_runtime(&content, baseline)?
        }
        None => {
            // 无激活配置时回退到内置默认配置，保证内核始终有可用配置。
            let default_path = &paths.default_mihomo_config_file;
            let content = fs::read_to_string(&default_path)
                .with_context(|| format!("无法读取默认配置：{}", default_path.display()))?;
            merge_runtime(&content, baseline)?
        }
    };
    validate_kernel_config(paths, version, &runtime)?;
    Ok(runtime)
}

/// 创建订阅类型配置的元数据。
pub(crate) fn subscription_meta(name: String, url: String) -> ProfileMeta {
    let now = now_secs();
    ProfileMeta {
        id: new_profile_id(),
        name,
        url: Some(url),
        added_at: now,
        updated_at: now,
        update_interval_minutes: 0,
        last_auto_attempt_at: 0,
    }
}

/// 创建本地导入类型配置的元数据；`url` 为空表示不提供远程更新操作。
pub(crate) fn local_meta(name: String) -> ProfileMeta {
    let now = now_secs();
    ProfileMeta {
        id: new_profile_id(),
        name,
        url: None,
        added_at: now,
        updated_at: now,
        update_interval_minutes: 0,
        last_auto_attempt_at: 0,
    }
}

/// 自动更新间隔下限（分钟）；更短的间隔对订阅服务器不友好。
/// debug 构建放宽到 1 分钟便于真机快速验证调度链路。
pub(crate) const MIN_UPDATE_INTERVAL_MINUTES: u64 = if cfg!(debug_assertions) { 1 } else { 10 };
/// 自动更新间隔上限（分钟），30 天。
pub(crate) const MAX_UPDATE_INTERVAL_MINUTES: u64 = 43_200;

/// 解析用户输入的自动更新间隔：`0` 表示关闭，其余必须落在
/// [`MIN_UPDATE_INTERVAL_MINUTES`]..=[`MAX_UPDATE_INTERVAL_MINUTES`]。
pub(crate) fn parse_update_interval(input: &str) -> Result<u64> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(0);
    }
    let minutes: u64 = trimmed
        .parse()
        .map_err(|_| anyhow::anyhow!("间隔必须是非负整数"))?;
    if minutes == 0 {
        return Ok(0);
    }
    if !(MIN_UPDATE_INTERVAL_MINUTES..=MAX_UPDATE_INTERVAL_MINUTES).contains(&minutes) {
        bail!(
            "间隔需在 {MIN_UPDATE_INTERVAL_MINUTES}..={MAX_UPDATE_INTERVAL_MINUTES} 分钟内，0 表示关闭"
        );
    }
    Ok(minutes)
}

/// 订阅是否已到自动更新期：以「最近成功更新」与「最近尝试」的较晚者为基准
/// 加间隔。墙钟比较让睡眠/休眠唤醒后立即补跑，时钟回拨只顺延不风暴。
/// `now` 为当前 UNIX 秒。
pub(crate) fn subscription_due(meta: &ProfileMeta, now: u64) -> bool {
    let Some(_url) = meta.url.as_ref() else {
        return false;
    };
    if meta.update_interval_minutes == 0 {
        return false;
    }
    let base = meta.updated_at.max(meta.last_auto_attempt_at);
    // base 为 0（从未更新过且从未尝试）时立即视为到期，启动后首跳即补跑。
    now >= base.saturating_add(meta.update_interval_minutes * 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn due_meta(interval_minutes: u64, updated_at: u64, last_attempt: u64) -> ProfileMeta {
        ProfileMeta {
            id: "test".to_owned(),
            name: "测试订阅".to_owned(),
            url: Some("https://example.com/sub".to_owned()),
            added_at: 0,
            updated_at,
            update_interval_minutes: interval_minutes,
            last_auto_attempt_at: last_attempt,
        }
    }

    #[test]
    fn parse_update_interval_accepts_bounds_and_rejects_others() {
        assert_eq!(parse_update_interval("").unwrap(), 0);
        assert_eq!(parse_update_interval("0").unwrap(), 0);
        assert_eq!(
            parse_update_interval(&MIN_UPDATE_INTERVAL_MINUTES.to_string()).unwrap(),
            MIN_UPDATE_INTERVAL_MINUTES
        );
        assert_eq!(
            parse_update_interval(&MAX_UPDATE_INTERVAL_MINUTES.to_string()).unwrap(),
            MAX_UPDATE_INTERVAL_MINUTES
        );
        assert_eq!(parse_update_interval(" 60 ").unwrap(), 60);
        assert!(parse_update_interval("abc").is_err());
        assert!(parse_update_interval("-1").is_err());
        assert!(parse_update_interval("999999999").is_err());
    }

    #[test]
    fn subscription_due_uses_latest_base() {
        // 间隔以分钟计：60 分钟 = 3600 秒。
        // 正常到期：updated_at(9000) + 3600 = 12600。
        assert!(subscription_due(&due_meta(60, 9_000, 0), 12_600));
        assert!(!subscription_due(&due_meta(60, 9_000, 0), 12_599));
        // 失败顺延：尝试时间(9999)比成功更新更晚，到期 = 9999 + 3600 = 13599。
        assert!(!subscription_due(&due_meta(60, 9_000, 9_999), 12_600));
        assert!(subscription_due(&due_meta(60, 9_000, 9_999), 13_599));
        // 关闭自动更新、本地导入（无 URL）永不到期。
        assert!(!subscription_due(&due_meta(0, 0, 0), 1_000_000));
        let mut local = due_meta(60, 0, 0);
        local.url = None;
        assert!(!subscription_due(&local, 1_000_000));
        // 从未更新也从未尝试（base 为 0）：10 分钟间隔在 600 秒时到期，
        // 启动后首跳即补跑错过的更新。
        assert!(subscription_due(&due_meta(10, 0, 0), 600));
        assert!(!subscription_due(&due_meta(10, 0, 0), 599));
    }

    #[test]
    fn url_validation_accepts_only_http_forms() {
        assert!(validate_subscription_url("https://example.com/sub").is_ok());
        assert!(validate_subscription_url("http://example.com/sub").is_ok());
        assert!(validate_subscription_url("ftp://example.com").is_err());
        assert!(validate_subscription_url("https://").is_err());
        assert!(validate_subscription_url("example.com").is_err());
    }

    #[test]
    fn default_name_extracts_host() {
        assert_eq!(
            default_name_from_url("https://p.example.com/clash/token"),
            Some("p.example.com".to_owned())
        );
        assert_eq!(default_name_from_url("not-a-url"), None);
        assert_eq!(default_name_from_url("https:///path"), None);
    }

    #[test]
    fn local_profile_uses_filename_and_has_no_subscription_url() {
        assert_eq!(
            default_name_from_path(Path::new("C:/configs/home.yaml")),
            Some("home".to_owned())
        );
        assert_eq!(default_name_from_path(Path::new(".yaml")), None);
        let meta = local_meta("本地配置".to_owned());
        assert_eq!(meta.name, "本地配置");
        assert!(meta.url.is_none());
    }

    #[test]
    fn local_config_reader_rejects_non_utf8_and_oversized_files() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时间应晚于 UNIX_EPOCH")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "pure-clash-local-profile-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("应创建本地配置测试目录");

        let valid = root.join("valid.yaml");
        fs::write(&valid, "rules:\n- MATCH,DIRECT\n").expect("应写入 UTF-8 配置");
        assert!(read_local_config(&valid).is_ok());

        let invalid = root.join("invalid.yaml");
        fs::write(&invalid, [0xff, 0xfe]).expect("应写入无效 UTF-8 配置");
        assert!(read_local_config(&invalid).is_err());

        let oversized = root.join("oversized.yaml");
        fs::File::create(&oversized)
            .and_then(|file| file.set_len(MAX_DOWNLOAD_BYTES + 1))
            .expect("应创建超限稀疏文件");
        assert!(read_local_config(&oversized).is_err());

        fs::remove_dir_all(root).expect("应清理本地配置测试目录");
    }

    #[test]
    fn profile_ids_are_random_and_filename_safe() {
        let first = new_profile_id();
        let second = new_profile_id();
        assert_ne!(first, second);
        assert!(first.bytes().all(|byte| byte.is_ascii_alphanumeric()));
    }

    /// 回归测试：激活配置后 runtime.yaml 必须包含订阅节点。
    /// 此前“添加订阅”路径未写 runtime.yaml，导致内核仍加载默认配置。
    #[test]
    fn activation_writes_subscription_nodes_into_runtime() {
        use crate::mihomo::config::LocalBaseline;
        use crate::mihomo::config::{merge_runtime, write_runtime};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时间应晚于 UNIX_EPOCH")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "pure-clash-activate-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("应创建测试目录");
        let mut paths = AppPaths::portable(&root);
        paths.kernel_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("kernel");
        fs::create_dir_all(&paths.mihomo_config_dir).expect("应创建配置目录");
        fs::create_dir_all(&paths.profiles_dir).expect("应创建 profiles 目录");

        let baseline = LocalBaseline {
            mixed_port: 17890,
            controller_addr: "127.0.0.1:19098".to_owned(),
            secret: "activate-secret".to_owned(),
            tun_enable: false,
            find_process_always: false,
        };
        let profile_yaml = "\
proxies:
- name: 美国 Vless主力
  type: vless
  server: 1.2.3.4
  port: 443
  uuid: 00000000-0000-0000-0000-000000000000
proxy-groups:
- name: PROXY
  type: select
  proxies:
  - 美国 Vless主力
rules:
- MATCH,PROXY
";
        let id = new_profile_id();
        fs::write(profile_yaml_path(&paths, &id), profile_yaml).expect("应写入配置文件");

        // 模拟激活链路：读取配置 → 合并基线 → 落地 runtime.yaml。
        let runtime = merge_runtime(&read_profile(&paths, &id).expect("应读取配置"), &baseline)
            .expect("应合并");
        write_runtime(&paths, &runtime).expect("应写入运行时配置");

        let runtime_on_disk =
            fs::read_to_string(&paths.runtime_mihomo_config_file).expect("应存在 runtime.yaml");
        assert!(
            runtime_on_disk.contains("美国 Vless主力"),
            "runtime 应包含订阅节点"
        );
        assert!(
            runtime_on_disk.contains("mixed-port: 17890"),
            "本地端口应覆盖"
        );

        let _ = fs::remove_dir_all(root);
    }

    /// 端到端验证：真实订阅下载 → 校验 → 激活 → 内核启动 → controller
    /// 模式与节点操作。依赖网络与随包内核，使用 `cargo test -- --ignored` 运行。
    #[test]
    #[ignore = "需要网络与随包内核，验证真实订阅端到端链路"]
    fn real_subscription_end_to_end() {
        use crate::mihomo::MihomoProcess;
        use crate::mihomo::controller::Mode;

        const SUBSCRIPTION_URL: &str = "https://p.ztion.cc/clash/Ztion-Net";

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时间应晚于 UNIX_EPOCH")
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("pure-clash-e2e-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&root).expect("应创建测试目录");
        let mut paths = AppPaths::portable(&root);
        paths.kernel_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("kernel");
        fs::create_dir_all(&paths.config_dir).expect("应创建配置目录");
        fs::create_dir_all(&paths.mihomo_config_dir).expect("应创建 Mihomo 配置目录");
        fs::create_dir_all(&paths.mihomo_data_dir).expect("应创建数据目录");
        fs::create_dir_all(&paths.profiles_dir).expect("应创建 profiles 目录");

        // 预写本地基线，避免默认 mixed-port 与本机已运行实例冲突。
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("应分配代理端口");
        let proxy_port = listener.local_addr().expect("应读取端口").port();
        drop(listener);
        fs::write(
            &paths.local_mihomo_config_file,
            format!(
                "mixed-port: {proxy_port}\nexternal-controller: 127.0.0.1:19097\nsecret: e2e-secret\n"
            ),
        )
        .expect("应写入基线");

        let version = env!("PURE_CLASH_DEFAULT_MIHOMO_VERSION");

        // 1. 下载真实订阅并走完整校验管线。
        let content = download_subscription(SUBSCRIPTION_URL).expect("应下载订阅");
        let id = new_profile_id();
        let runtime = validate_and_store(&paths, version, &id, &content).expect("订阅应通过校验");
        assert!(runtime.contains(&format!("mixed-port: {proxy_port}")));
        assert!(!runtime.contains("enable: true\ntun") && !runtime.contains("tun: true"));

        // 2. 合并产物写入 runtime.yaml 并启动内核。
        write_runtime(&paths, &runtime).expect("应写入运行时配置");
        let mut process = MihomoProcess::start(
            &paths,
            version,
            &paths.runtime_mihomo_config_file,
            false,
            true,
        )
        .expect("应启动内核");

        // 3. controller 就绪后验证配置生效。
        let baseline = ensure_baseline(&paths).expect("应读取基线");
        let controller = Controller::new(&baseline);
        let mut ready = false;
        for _ in 0..50 {
            if controller.version().is_ok() {
                ready = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(ready, "controller 应就绪");

        let config = controller.configs().expect("应读取配置");
        assert_eq!(config.mode, Mode::Rule);

        let proxies = controller.proxies(Mode::Rule).expect("应读取代理组");
        let proxy_group = proxies
            .groups
            .iter()
            .find(|group| group.name == "PROXY")
            .expect("订阅的 PROXY 组应存在");
        assert!(proxy_group.selectable, "PROXY 应是 Selector 组");
        assert!(
            proxy_group
                .nodes
                .iter()
                .any(|node| node.name.contains("美国")),
            "应包含订阅中的美国节点"
        );

        // 4. 切换运行模式与节点，验证真实生效。
        controller.patch_mode("global").expect("应切换全局模式");
        let config = controller.configs().expect("应读取配置");
        assert_eq!(config.mode, Mode::Global);

        let target = proxy_group
            .nodes
            .iter()
            .find(|node| node.name.contains("美国"))
            .expect("应有美国节点");
        controller
            .select_proxy("PROXY", &target.name)
            .expect("应切换节点");
        let proxies = controller.proxies(Mode::Global).expect("应读取代理组");
        let group = proxies
            .groups
            .iter()
            .find(|group| group.name == "PROXY")
            .expect("PROXY 应存在");
        assert_eq!(group.now, target.name);

        // 5. mixed-port 连通测试：通过本地代理端口发起真实请求。
        let proxy =
            ureq::Proxy::new(&format!("http://127.0.0.1:{proxy_port}")).expect("应构造代理");
        let agent = ureq::AgentBuilder::new()
            .proxy(proxy)
            .timeout(std::time::Duration::from_secs(15))
            .build();
        let response = agent
            .get("http://cp.cloudflare.com/generate_204")
            .call()
            .expect("应通过 mixed-port 收到响应");
        assert_eq!(response.status(), 204, "generate_204 应返回 204");

        process.stop().expect("应停止内核");
        let _ = fs::remove_dir_all(root);
    }
}
