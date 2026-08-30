//! Linux TUN 常驻服务、一次性安装器与受限 IPC。
//!
//! 设计参考 Clash Verge Rev：首次授权把服务与锁定版本 Mihomo 安装到 root
//! 所有的固定位置；之后 GUI 只向按 UID 授权的 Unix socket 提交运行时 bundle。
//! 服务把配置与资源物化到受保护目录，并始终以 root 启动 Mihomo。这里不降权、
//! 不设置 ambient capabilities，也不替换系统网络工具。

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::{CString, OsStr, OsString},
    fs::{self, OpenOptions},
    io::{Read, Write},
    net::{IpAddr, SocketAddr},
    os::unix::{
        ffi::OsStrExt,
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        io::AsRawFd,
        net::{UnixListener, UnixStream},
        process::CommandExt,
    },
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use uuid::Uuid;

use crate::platform::{kernel_binary_name, process_guard::terminate_unix};

const INSTALL_ARG: &str = "--linux-tun-service-install";
const SERVICE_ARG: &str = "--linux-tun-service";
// 协议 3 是 root runtime bundle 架构；旧的 capabilities/helper 服务必须重装。
const SERVICE_PROTOCOL: u32 = 3;
const SERVICE_NAME: &str = "pure-clash-service.service";
const SERVICE_BINARY: &str = "/usr/libexec/pure-clash-service";
const SERVICE_ROOT: &str = "/usr/lib/pure-clash";
const SERVICE_STATE_ROOT: &str = "/var/lib/pure-clash-service";
const SERVICE_RUNTIME_ROOT: &str = "/run/pure-clash-service";
const SERVICE_UNIT: &str = "/etc/systemd/system/pure-clash-service.service";
const SERVICE_SOCKET: &str = "/run/pure-clash-service/service.sock";
const RUNTIME_CONFIG: &str = "config.yaml";
const RUNTIME_MANIFEST: &str = ".pure-clash-runtime.json";
const MAX_CONFIG_BYTES: usize = 12 * 1024 * 1024;
const MAX_FRAME_SIZE: usize = 32 * 1024 * 1024;
const MAX_ASSETS: usize = 128;
const MAX_ASSET_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TOTAL_ASSET_BYTES: u64 = 192 * 1024 * 1024;
const GEO_ASSETS: &[&str] = &[
    "Country.mmdb",
    "GeoIP.dat",
    "geoip.dat",
    "GeoSite.dat",
    "geosite.dat",
    "GeoIP.metadb",
    "geoip.metadb",
    "ASN.mmdb",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RuntimeAsset {
    source: PathBuf,
    destination: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RemoteProvider {
    destination: PathBuf,
    url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RuntimeBundle {
    yaml: String,
    assets: Vec<RuntimeAsset>,
    remote_providers: Vec<RemoteProvider>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
enum ServiceRequest {
    Ping,
    Start { bundle: RuntimeBundle },
    Status { pid: u32 },
    Stop { pid: u32 },
}

#[derive(Debug, Serialize, Deserialize)]
struct ServiceResponse {
    protocol: u32,
    kernel_version: String,
    ok: bool,
    running: bool,
    pid: Option<u32>,
    error: Option<String>,
}

impl ServiceResponse {
    fn success(pid: Option<u32>, running: bool) -> Self {
        Self {
            protocol: SERVICE_PROTOCOL,
            kernel_version: env!("PURE_CLASH_DEFAULT_MIHOMO_VERSION").to_owned(),
            ok: true,
            running,
            pid,
            error: None,
        }
    }

    fn failure(error: &anyhow::Error) -> Self {
        Self {
            protocol: SERVICE_PROTOCOL,
            kernel_version: env!("PURE_CLASH_DEFAULT_MIHOMO_VERSION").to_owned(),
            ok: false,
            running: false,
            pid: None,
            error: Some(error_chain(error, 480)),
        }
    }
}

#[cfg(test)]
pub(super) fn is_internal_mode(arg: Option<&OsStr>) -> bool {
    matches!(arg.and_then(OsStr::to_str), Some(INSTALL_ARG | SERVICE_ARG))
}

/// 正常应用启动返回 `Ok(false)`；安装器或 systemd 服务模式完成后返回。
pub(super) fn run_internal_mode_if_requested() -> Result<bool> {
    let mut args = env::args_os();
    let _program = args.next();
    let Some(mode) = args.next() else {
        return Ok(false);
    };
    match mode.to_str() {
        Some(INSTALL_ARG) => {
            let parent_pid = parse_u32(args.next(), "父进程 PID")?;
            let kernel = required_path(args.next(), "Mihomo 路径")?;
            reject_extra_args(args)?;
            install_service(parent_pid, &kernel)?;
            Ok(true)
        }
        Some(SERVICE_ARG) => {
            let authorized_uid = parse_uid(args.next(), "授权 UID")?;
            reject_extra_args(args)?;
            run_service(authorized_uid)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

pub(super) fn start_core(source_kernel: &Path, data_dir: &Path, config_file: &Path) -> Result<u32> {
    if Path::new("/sys/class/net/Meta").exists() {
        bail!("检测到其他 Mihomo TUN 设备 Meta；请先关闭其他代理客户端的 TUN 模式");
    }
    let bundle = collect_runtime_bundle(data_dir, config_file)?;
    ensure_service(source_kernel)?;
    let response = request(&ServiceRequest::Start { bundle })?;
    if !response.ok {
        bail!(
            "Linux TUN 服务启动内核失败：{}",
            response.error.as_deref().unwrap_or("服务未返回详情")
        );
    }
    response.pid.context("Linux TUN 服务未返回内核 PID")
}

pub(super) fn is_core_running(pid: u32) -> bool {
    request(&ServiceRequest::Status { pid })
        .is_ok_and(|response| response.ok && response.running && response.pid == Some(pid))
}

pub(super) fn stop_core(pid: u32) -> Result<()> {
    let response = request(&ServiceRequest::Stop { pid })?;
    if response.ok {
        Ok(())
    } else {
        bail!(
            "Linux TUN 服务停止内核失败：{}",
            response.error.as_deref().unwrap_or("服务未返回详情")
        )
    }
}

fn collect_runtime_bundle(data_dir: &Path, config_file: &Path) -> Result<RuntimeBundle> {
    let yaml = fs::read_to_string(config_file)
        .with_context(|| format!("无法读取 Linux TUN 运行时配置：{}", config_file.display()))?;
    if yaml.is_empty() || yaml.len() > MAX_CONFIG_BYTES {
        bail!("Linux TUN 运行时配置为空或超过 12MB 上限");
    }
    let mut config: Value =
        serde_yaml::from_str(&yaml).context("Linux TUN 运行时配置不是有效 YAML")?;
    let config_root = canonical_directory(
        config_file
            .parent()
            .context("Linux TUN 配置路径缺少父目录")?,
    )?;
    let data_root = canonical_directory(data_dir)?;
    let mut assets = Vec::new();
    let mut remote_providers = Vec::new();
    let mut destinations = BTreeSet::new();

    collect_provider_assets(
        &mut config,
        "proxy-providers",
        &config_root,
        &mut destinations,
        &mut assets,
        &mut remote_providers,
    )?;
    collect_provider_assets(
        &mut config,
        "rule-providers",
        &config_root,
        &mut destinations,
        &mut assets,
        &mut remote_providers,
    )?;
    for name in GEO_ASSETS {
        let source = data_root.join(name);
        if source.is_file() && destinations.insert((*name).to_owned()) {
            assets.push(RuntimeAsset {
                source: canonical_regular_file(&source, "Mihomo 数据资源")?,
                destination: PathBuf::from(name),
            });
        }
    }
    if assets.len() > MAX_ASSETS {
        bail!("Linux TUN 运行时资源数量超过 {MAX_ASSETS} 个上限");
    }

    Ok(RuntimeBundle {
        yaml: serde_yaml::to_string(&config).context("无法序列化 Linux TUN 运行时配置")?,
        assets,
        remote_providers,
    })
}

fn collect_provider_assets(
    config: &mut Value,
    section: &str,
    config_root: &Path,
    destinations: &mut BTreeSet<String>,
    assets: &mut Vec<RuntimeAsset>,
    remote_providers: &mut Vec<RemoteProvider>,
) -> Result<()> {
    let Some(providers) = config
        .as_mapping_mut()
        .and_then(|mapping| mapping.get_mut(Value::from(section)))
        .and_then(Value::as_mapping_mut)
    else {
        return Ok(());
    };

    for provider in providers.values_mut() {
        let Some(provider) = provider.as_mapping_mut() else {
            continue;
        };
        let Some(raw_path) = provider
            .get(Value::from("path"))
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            continue;
        };
        let is_remote = provider.get(Value::from("type")).and_then(Value::as_str) == Some("http");
        let destination = provider_destination(config_root, &raw_path)?;
        let key = destination_key(&destination)?;
        if !destinations.insert(key.clone()) {
            bail!("Linux TUN provider 运行时路径重复：{key}");
        }

        if is_remote {
            let url = provider
                .get(Value::from("url"))
                .and_then(Value::as_str)
                .filter(|url| !url.is_empty())
                .context("远程 provider 缺少 URL")?;
            remote_providers.push(RemoteProvider {
                destination: destination.clone(),
                url: url.to_owned(),
            });
        } else {
            assets.push(RuntimeAsset {
                source: local_provider_source(config_root, &raw_path)?,
                destination: destination.clone(),
            });
        }
        provider.insert(
            Value::from("path"),
            Value::from(destination.to_string_lossy().into_owned()),
        );
    }
    Ok(())
}

fn local_provider_source(config_root: &Path, raw_path: &str) -> Result<PathBuf> {
    let requested = Path::new(raw_path);
    let source = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        config_root.join(requested)
    };
    let canonical = canonical_regular_file(&source, "本地 provider")?;
    canonical
        .strip_prefix(config_root)
        .context("本地 provider 位于配置目录之外")?;
    Ok(canonical)
}

fn provider_destination(config_root: &Path, raw_path: &str) -> Result<PathBuf> {
    let requested = Path::new(raw_path);
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("provider 路径不能包含上级目录");
    }
    let relative = if requested.is_absolute() {
        canonicalize_with_missing_tail(requested)?
            .strip_prefix(config_root)
            .context("provider 绝对路径位于配置目录之外")?
            .to_path_buf()
    } else {
        requested.to_path_buf()
    };
    normalized_destination(&relative)
}

fn canonicalize_with_missing_tail(path: &Path) -> Result<PathBuf> {
    let mut ancestor = path;
    let mut tail = Vec::new();
    while !ancestor.exists() {
        tail.push(
            ancestor
                .file_name()
                .context("provider 路径没有可解析的父目录")?
                .to_owned(),
        );
        ancestor = ancestor
            .parent()
            .context("provider 路径没有可解析的父目录")?;
    }
    let mut result = fs::canonicalize(ancestor)?;
    for component in tail.iter().rev() {
        result.push(component);
    }
    Ok(result)
}

fn normalized_destination(path: &Path) -> Result<PathBuf> {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(component) => result.push(component),
            Component::CurDir => {}
            _ => bail!("运行时资源路径必须是非穿越相对路径"),
        }
    }
    if result.as_os_str().is_empty() {
        bail!("运行时资源路径不能为空");
    }
    Ok(result)
}

fn ensure_service(source_kernel: &Path) -> Result<()> {
    if service_is_current() {
        return Ok(());
    }

    let helper = env::current_exe().context("无法确定 Linux TUN 服务安装器路径")?;
    let output = Command::new("pkexec")
        .arg(&helper)
        .arg(INSTALL_ARG)
        .arg(std::process::id().to_string())
        .arg(source_kernel)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("无法启动 pkexec；请确认系统已安装并运行 polkit 认证代理")?;
    if !output.status.success() {
        match output.status.code() {
            Some(126) => bail!("用户取消了 Linux TUN 服务安装授权"),
            Some(127) => bail!("Linux TUN 服务安装授权失败；请确认 polkit 正常运行"),
            Some(code) => bail!(
                "Linux TUN 服务安装失败（退出码 {code}）：{}",
                output_detail(&output)
            ),
            None => bail!("Linux TUN 服务安装器被信号终止"),
        }
    }

    for _ in 0..100 {
        if service_is_current() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    bail!("Linux TUN 服务安装完成但 IPC 未就绪")
}

fn service_is_current() -> bool {
    request(&ServiceRequest::Ping).is_ok_and(|response| {
        response.ok
            && response.protocol == SERVICE_PROTOCOL
            && response.kernel_version == env!("PURE_CLASH_DEFAULT_MIHOMO_VERSION")
    })
}

fn request(request: &ServiceRequest) -> Result<ServiceResponse> {
    let mut stream = UnixStream::connect(SERVICE_SOCKET).context("无法连接 Linux TUN 服务")?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    write_frame(&mut stream, request)?;
    read_frame(&mut stream)
}

fn install_service(parent_pid: u32, source_kernel: &Path) -> Result<()> {
    require_root("Linux TUN 服务安装器")?;
    let caller_uid = pkexec_uid()?;
    if caller_uid == 0 {
        bail!("Linux TUN 服务安装器拒绝 root 会话调用");
    }
    verify_installer_parent(parent_pid, caller_uid)?;

    let source_service = validate_file(&env::current_exe()?, caller_uid, true, true)?;
    let source_kernel = validate_file(source_kernel, caller_uid, true, true)?;
    validate_kernel_path(&source_kernel)?;
    let account = lookup_account(caller_uid)?;

    let _ = Command::new("systemctl")
        .args(["stop", SERVICE_NAME])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    ensure_root_directory(Path::new("/usr/libexec"), 0o755)?;
    ensure_root_directory(Path::new(SERVICE_ROOT), 0o755)?;
    ensure_root_directory(Path::new(SERVICE_STATE_ROOT), 0o700)?;
    let installed_kernel = installed_kernel_path();
    ensure_root_directory(
        installed_kernel
            .parent()
            .context("安装内核路径缺少父目录")?,
        0o755,
    )?;
    atomic_install(&source_service, Path::new(SERVICE_BINARY), 0o755)?;
    atomic_install(&source_kernel, &installed_kernel, 0o755)?;

    atomic_write_root(
        Path::new(SERVICE_UNIT),
        service_unit(caller_uid, account.gid).as_bytes(),
        0o644,
    )?;
    run_checked("systemctl", &["daemon-reload"])?;
    run_checked("systemctl", &["enable", "--now", SERVICE_NAME])?;
    Ok(())
}

fn service_unit(uid: libc::uid_t, gid: libc::gid_t) -> String {
    format!(
        "[Unit]\n\
         Description=Pure Clash TUN Service\n\
         After=network-online.target nftables.service iptables.service\n\n\
         [Service]\n\
         Type=simple\n\
         ExecStart={SERVICE_BINARY} {SERVICE_ARG} {uid}\n\
         Group={gid}\n\
         Restart=always\n\
         RestartSec=5\n\
         RuntimeDirectory=pure-clash-service\n\
         RuntimeDirectoryMode=0755\n\
         UMask=0022\n\
         KillMode=control-group\n\
         SyslogIdentifier=pure-clash-service\n\
         PrivateTmp=true\n\
         ProtectSystem=full\n\n\
         [Install]\n\
         WantedBy=multi-user.target\n"
    )
}

struct ManagedCore {
    child: Child,
    owner_pid: u32,
}

impl ManagedCore {
    fn is_running(&mut self) -> bool {
        self.child.try_wait().is_ok_and(|status| status.is_none())
    }

    fn stop(&mut self) -> Result<()> {
        if self.child.try_wait()?.is_none() {
            terminate_unix(&mut self.child)?;
        }
        self.child.wait().context("无法回收 Linux TUN 内核")?;
        Ok(())
    }
}

fn run_service(authorized_uid: libc::uid_t) -> Result<()> {
    require_root("Linux TUN 服务")?;
    let executable = fs::canonicalize(env::current_exe()?)?;
    if executable != fs::canonicalize(SERVICE_BINARY)? {
        bail!("Linux TUN 服务只允许从已安装路径运行");
    }
    validate_root_file(&executable, true)?;
    validate_root_file(&installed_kernel_path(), true)?;
    let account = lookup_account(authorized_uid)?;

    ensure_root_directory(Path::new(SERVICE_RUNTIME_ROOT), 0o755)?;
    ensure_root_directory(Path::new(SERVICE_STATE_ROOT), 0o700)?;
    ensure_root_directory(&user_state_directory(authorized_uid), 0o700)?;
    ensure_root_directory(&runtime_directory(authorized_uid), 0o700)?;
    remove_stale_socket(Path::new(SERVICE_SOCKET))?;
    let listener = UnixListener::bind(SERVICE_SOCKET).context("无法创建 Linux TUN 服务 IPC")?;
    fs::set_permissions(SERVICE_SOCKET, fs::Permissions::from_mode(0o660))?;
    chown_path(Path::new(SERVICE_SOCKET), 0, account.gid)?;
    listener.set_nonblocking(true)?;

    let mut core: Option<ManagedCore> = None;
    loop {
        let stale = core.as_mut().is_some_and(|managed| {
            !managed.is_running() || !Path::new(&format!("/proc/{}", managed.owner_pid)).exists()
        });
        if stale && let Some(mut managed) = core.take() {
            let _ = managed.stop();
        }

        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_read_timeout(Some(Duration::from_secs(30)))?;
                stream.set_write_timeout(Some(Duration::from_secs(30)))?;
                let response = match handle_connection(&mut stream, authorized_uid, &mut core) {
                    Ok(response) => response,
                    Err(error) => {
                        eprintln!("Linux TUN 服务处理请求失败：{error:#}");
                        ServiceResponse::failure(&error)
                    }
                };
                let _ = write_frame(&mut stream, &response);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(error).context("Linux TUN 服务接受 IPC 失败"),
        }
    }
}

fn handle_connection(
    stream: &mut UnixStream,
    authorized_uid: libc::uid_t,
    core: &mut Option<ManagedCore>,
) -> Result<ServiceResponse> {
    let (peer_pid, peer_uid) = peer_credentials(stream)?;
    if peer_uid != authorized_uid {
        bail!("Linux TUN 服务拒绝未授权用户");
    }
    let request: ServiceRequest = read_frame(stream)?;
    match request {
        ServiceRequest::Ping => {
            let pid = core
                .as_mut()
                .and_then(|managed| managed.is_running().then(|| managed.child.id()));
            Ok(ServiceResponse::success(pid, pid.is_some()))
        }
        ServiceRequest::Start { bundle } => {
            let runtime = runtime_directory(authorized_uid);
            materialize_runtime(authorized_uid, &runtime, &bundle)?;
            validate_materialized_config(&runtime)?;
            if let Some(mut running) = core.take() {
                running.stop()?;
            }
            let child = launch_root_core(&runtime)?;
            let pid = child.id();
            *core = Some(ManagedCore {
                child,
                owner_pid: peer_pid,
            });
            Ok(ServiceResponse::success(Some(pid), true))
        }
        ServiceRequest::Status { pid } => {
            let running = core
                .as_mut()
                .is_some_and(|managed| managed.child.id() == pid && managed.is_running());
            Ok(ServiceResponse::success(running.then_some(pid), running))
        }
        ServiceRequest::Stop { pid } => {
            if let Some(mut running) = core.take() {
                if running.child.id() != pid {
                    *core = Some(running);
                    bail!("Linux TUN 服务中的内核 PID 不匹配");
                }
                running.stop()?;
            }
            Ok(ServiceResponse::success(None, false))
        }
    }
}

fn materialize_runtime(
    authorized_uid: libc::uid_t,
    runtime: &Path,
    bundle: &RuntimeBundle,
) -> Result<()> {
    validate_runtime_yaml(&bundle.yaml)?;
    if bundle.yaml.len() > MAX_CONFIG_BYTES || bundle.assets.len() > MAX_ASSETS {
        bail!("Linux TUN runtime bundle 超过安全上限");
    }

    let previous = read_runtime_manifest(runtime);
    let mut copied = BTreeSet::new();
    let mut remote = BTreeMap::new();
    let mut staged = Vec::new();
    let mut total_bytes = 0u64;

    for asset in &bundle.assets {
        let key = destination_key(&asset.destination)?;
        if !copied.insert(key.clone()) || remote.contains_key(&key) {
            bail!("Linux TUN runtime bundle 重复声明资源：{key}");
        }
        let target = runtime_target(runtime, &asset.destination)?;
        let temporary = stage_asset(authorized_uid, &asset.source, &target, &mut total_bytes)?;
        staged.push((temporary, target));
    }
    for provider in &bundle.remote_providers {
        let key = destination_key(&provider.destination)?;
        if provider.url.is_empty()
            || copied.contains(&key)
            || remote.insert(key.clone(), provider.url.clone()).is_some()
        {
            bail!("Linux TUN runtime bundle 的远程 provider 声明无效：{key}");
        }
        runtime_target(runtime, &provider.destination)?;
    }

    let config_temp = stage_root_bytes(runtime, RUNTIME_CONFIG, bundle.yaml.as_bytes(), 0o600)?;
    let manifest = RuntimeManifest {
        copied: copied.clone(),
        remote: remote.clone(),
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    let manifest_temp = stage_root_bytes(runtime, RUNTIME_MANIFEST, &manifest_bytes, 0o600)?;

    let commit_result = (|| {
        for (temporary, target) in &staged {
            fs::rename(temporary, target)
                .with_context(|| format!("无法物化 Linux TUN 资源：{}", target.display()))?;
        }
        for old in previous.copied.difference(&copied) {
            remove_managed_runtime_file(runtime, old)?;
        }
        for (destination, old_url) in &previous.remote {
            if remote.get(destination) != Some(old_url) {
                remove_managed_runtime_file(runtime, destination)?;
            }
        }
        fs::rename(&config_temp, runtime.join(RUNTIME_CONFIG))?;
        fs::rename(&manifest_temp, runtime.join(RUNTIME_MANIFEST))?;
        Ok(())
    })();
    if commit_result.is_err() {
        for (temporary, _) in staged {
            let _ = fs::remove_file(temporary);
        }
        let _ = fs::remove_file(config_temp);
        let _ = fs::remove_file(manifest_temp);
    }
    commit_result
}

#[derive(Default, Serialize, Deserialize)]
struct RuntimeManifest {
    #[serde(default)]
    copied: BTreeSet<String>,
    #[serde(default)]
    remote: BTreeMap<String, String>,
}

fn read_runtime_manifest(runtime: &Path) -> RuntimeManifest {
    fs::read(runtime.join(RUNTIME_MANIFEST))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn validate_runtime_yaml(yaml: &str) -> Result<()> {
    if yaml.is_empty() || yaml.len() > MAX_CONFIG_BYTES {
        bail!("Linux TUN 运行时配置为空或超过 12MB 上限");
    }
    let value: Value = serde_yaml::from_str(yaml).context("Linux TUN 服务收到无效 YAML")?;
    let mapping = value
        .as_mapping()
        .context("Linux TUN 运行时配置顶层必须是映射")?;
    let tun_enabled = mapping
        .get(Value::from("tun"))
        .and_then(|tun| tun.get("enable"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !tun_enabled {
        bail!("Linux TUN 服务拒绝启动未开启 TUN 的 root 内核");
    }
    if mapping
        .get(Value::from("allow-lan"))
        .and_then(Value::as_bool)
        != Some(false)
    {
        bail!("Linux TUN 服务要求 allow-lan=false");
    }
    let bind = mapping
        .get(Value::from("bind-address"))
        .and_then(Value::as_str)
        .context("Linux TUN 配置缺少 bind-address")?;
    if !bind
        .parse::<IpAddr>()
        .is_ok_and(|address| address.is_loopback())
    {
        bail!("Linux TUN 服务只允许回环代理监听地址");
    }
    let controller = mapping
        .get(Value::from("external-controller"))
        .and_then(Value::as_str)
        .context("Linux TUN 配置缺少 external-controller")?
        .parse::<SocketAddr>()
        .context("Linux TUN external-controller 地址无效")?;
    if !controller.ip().is_loopback() {
        bail!("Linux TUN 服务只允许回环 controller");
    }
    for unsupported in ["external-controller-unix", "external-controller-pipe"] {
        if mapping.contains_key(Value::from(unsupported)) {
            bail!("Linux TUN 配置不得声明 {unsupported}");
        }
    }
    Ok(())
}

fn stage_asset(
    authorized_uid: libc::uid_t,
    source: &Path,
    target: &Path,
    total_bytes: &mut u64,
) -> Result<PathBuf> {
    let canonical = canonical_regular_file(source, "Linux TUN runtime 资源")?;
    if canonical != source {
        bail!("Linux TUN runtime 资源路径必须是规范绝对路径");
    }
    let mut input = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&canonical)?;
    let metadata = input.metadata()?;
    if !metadata.is_file() || metadata.uid() != authorized_uid || metadata.mode() & 0o022 != 0 {
        bail!("Linux TUN 服务拒绝所有者或权限不安全的资源");
    }
    if metadata.len() == 0 || metadata.len() > MAX_ASSET_BYTES {
        bail!("Linux TUN runtime 单个资源为空或超过 64MB 上限");
    }
    *total_bytes = total_bytes.saturating_add(metadata.len());
    if *total_bytes > MAX_TOTAL_ASSET_BYTES {
        bail!("Linux TUN runtime 资源总量超过 192MB 上限");
    }

    let parent = target.parent().context("Linux TUN 资源目标缺少父目录")?;
    ensure_root_directory(parent, 0o700)?;
    let temporary = parent.join(format!(
        ".{}.staging-{}",
        target
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("asset"),
        Uuid::new_v4().simple()
    ));
    let result = (|| {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temporary)?;
        let size = std::io::copy(&mut input, &mut output)?;
        if size != metadata.len() {
            bail!("Linux TUN runtime 资源复制期间发生变化");
        }
        output.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map(|()| temporary)
}

fn stage_root_bytes(directory: &Path, name: &str, bytes: &[u8], mode: u32) -> Result<PathBuf> {
    let temporary = directory.join(format!(".{name}.staging-{}", Uuid::new_v4().simple()));
    let result = (|| {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temporary)?;
        output.write_all(bytes)?;
        output.sync_all()?;
        output.set_permissions(fs::Permissions::from_mode(mode))?;
        validate_root_file(&temporary, false)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map(|()| temporary)
}

fn runtime_target(runtime: &Path, destination: &Path) -> Result<PathBuf> {
    destination_key(destination)?;
    let target = runtime.join(destination);
    if let Some(parent) = target.parent() {
        ensure_root_directory(parent, 0o700)?;
    }
    Ok(target)
}

fn destination_key(destination: &Path) -> Result<String> {
    let normalized = normalized_destination(destination)?;
    let key = normalized.to_string_lossy().into_owned();
    let first = normalized
        .components()
        .next()
        .and_then(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        });
    if matches!(first, Some(RUNTIME_CONFIG | RUNTIME_MANIFEST)) || key.contains(".staging-") {
        bail!("Linux TUN runtime 资源目标使用了保留名称");
    }
    Ok(key)
}

fn remove_managed_runtime_file(runtime: &Path, destination: &str) -> Result<()> {
    let target = runtime_target(runtime, Path::new(destination))?;
    match fs::symlink_metadata(&target) {
        Ok(metadata) if metadata.is_file() && metadata.uid() == 0 => fs::remove_file(target)?,
        Ok(_) => bail!("Linux TUN runtime 中的托管资源类型或所有者异常"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn validate_materialized_config(runtime: &Path) -> Result<()> {
    let output = Command::new(installed_kernel_path())
        .arg("-d")
        .arg(runtime)
        .arg("-f")
        .arg(runtime.join(RUNTIME_CONFIG))
        .arg("-t")
        .current_dir(runtime)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("Linux TUN 服务无法执行 Mihomo 配置校验")?;
    if output.status.success() {
        return Ok(());
    }
    bail!("受保护运行时校验失败：{}", output_detail(&output))
}

fn launch_root_core(runtime: &Path) -> Result<Child> {
    let mut child = Command::new(installed_kernel_path())
        .arg("-d")
        .arg(runtime)
        .arg("-f")
        .arg(runtime.join(RUNTIME_CONFIG))
        .current_dir(runtime)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .process_group(0)
        .spawn()
        .context("Linux TUN 服务无法启动 root Mihomo")?;
    thread::sleep(Duration::from_millis(300));
    if let Some(status) = child.try_wait()? {
        bail!("Linux TUN 内核启动后立即退出：{status}；详情见 systemd journal");
    }
    Ok(child)
}

fn verify_installer_parent(parent_pid: u32, caller_uid: libc::uid_t) -> Result<()> {
    if unsafe { libc::getppid() } as u32 != parent_pid {
        bail!("Linux TUN 服务安装器的父进程已变化");
    }
    if process_uid(parent_pid)? != caller_uid {
        bail!("Linux TUN 服务安装器调用者与父进程不匹配");
    }
    let parent_executable = fs::canonicalize(format!("/proc/{parent_pid}/exe"))?;
    let installer_executable = fs::canonicalize(env::current_exe()?)?;
    if parent_executable != installer_executable {
        bail!("Linux TUN 服务安装器只接受 Pure Clash 主进程调用");
    }
    Ok(())
}

fn process_uid(pid: u32) -> Result<libc::uid_t> {
    let status = fs::read_to_string(format!("/proc/{pid}/status"))?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|uids| uids.split_whitespace().next())
        .and_then(|uid| uid.parse().ok())
        .ok_or_else(|| anyhow!("进程 UID 无效"))
}

fn peer_credentials(stream: &UnixStream) -> Result<(u32, libc::uid_t)> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    if result != 0 || credentials.pid <= 0 {
        return Err(std::io::Error::last_os_error()).context("无法验证 Linux TUN IPC 调用者");
    }
    Ok((credentials.pid as u32, credentials.uid))
}

fn validate_file(
    path: &Path,
    owner_uid: libc::uid_t,
    executable: bool,
    allow_root_owner: bool,
) -> Result<PathBuf> {
    let path = canonical_regular_file(path, "Linux TUN 文件")?;
    let metadata = fs::metadata(&path)?;
    let valid_owner = metadata.uid() == owner_uid || (allow_root_owner && metadata.uid() == 0);
    if !valid_owner || metadata.mode() & 0o022 != 0 {
        bail!("Linux TUN 服务拒绝不安全的文件：{}", path.display());
    }
    if executable && metadata.mode() & 0o111 == 0 {
        bail!("文件不可执行：{}", path.display());
    }
    Ok(path)
}

fn validate_root_file(path: &Path, executable: bool) -> Result<PathBuf> {
    validate_file(path, 0, executable, false)
}

fn canonical_regular_file(path: &Path, label: &str) -> Result<PathBuf> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("{label}不可用：{}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("{label}必须是普通文件：{}", path.display());
    }
    fs::canonicalize(path).with_context(|| format!("无法解析{label}：{}", path.display()))
}

fn canonical_directory(path: &Path) -> Result<PathBuf> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("Linux TUN runtime 根路径必须是普通目录");
    }
    fs::canonicalize(path).context("无法解析 Linux TUN runtime 根路径")
}

fn validate_kernel_path(path: &Path) -> Result<()> {
    let version_dir = path
        .parent()
        .and_then(Path::file_name)
        .and_then(OsStr::to_str)
        .ok_or_else(|| anyhow!("Mihomo 路径缺少版本目录"))?;
    let kernel_dir = path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(OsStr::to_str);
    let valid_version = !version_dir.is_empty()
        && version_dir
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'));
    if path.file_name().and_then(OsStr::to_str) != Some(kernel_binary_name())
        || kernel_dir != Some("kernel")
        || !valid_version
    {
        bail!("Linux TUN 服务只允许安装 Pure Clash 随包 Mihomo 内核");
    }
    Ok(())
}

struct UserAccount {
    gid: libc::gid_t,
}

fn lookup_account(uid: libc::uid_t) -> Result<UserAccount> {
    let passwd = unsafe { libc::getpwuid(uid) };
    if passwd.is_null() {
        bail!("无法解析 Linux TUN 授权用户");
    }
    let passwd = unsafe { &*passwd };
    Ok(UserAccount { gid: passwd.pw_gid })
}

fn ensure_root_directory(path: &Path, mode: u32) -> Result<()> {
    if path.exists() {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() || metadata.uid() != 0 {
            bail!("root 运行时目录权限不安全：{}", path.display());
        }
    } else {
        fs::create_dir_all(path)?;
    }
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
        bail!("root 运行时目录权限不安全：{}", path.display());
    }
    Ok(())
}

fn remove_stale_socket(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() && metadata.uid() == 0 => {
            fs::remove_file(path)?;
        }
        Ok(_) => bail!("Linux TUN IPC 路径被不安全文件占用"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn atomic_install(source: &Path, destination: &Path, mode: u32) -> Result<()> {
    let temporary = destination.with_extension(format!("tmp-{}", std::process::id()));
    let _ = fs::remove_file(&temporary);
    fs::copy(source, &temporary)?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))?;
    validate_root_file(&temporary, mode & 0o111 != 0)?;
    fs::rename(temporary, destination)?;
    Ok(())
}

fn atomic_write_root(destination: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let parent = destination.parent().context("root 文件缺少父目录")?;
    ensure_root_directory(parent, 0o755)?;
    let temporary = destination.with_extension(format!("tmp-{}", std::process::id()));
    let _ = fs::remove_file(&temporary);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    file.set_permissions(fs::Permissions::from_mode(mode))?;
    validate_root_file(&temporary, false)?;
    fs::rename(temporary, destination)?;
    Ok(())
}

fn chown_path(path: &Path, uid: libc::uid_t, gid: libc::gid_t) -> Result<()> {
    let path = CString::new(path.as_os_str().as_bytes())?;
    if unsafe { libc::chown(path.as_ptr(), uid, gid) } != 0 {
        return Err(std::io::Error::last_os_error()).context("无法设置 Linux TUN 路径所有者");
    }
    Ok(())
}

fn run_checked(program: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(program).args(args).output()?;
    if output.status.success() {
        Ok(())
    } else {
        bail!("{program} 执行失败：{}", output_detail(&output))
    }
}

fn write_frame<T: Serialize>(stream: &mut UnixStream, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.is_empty() || bytes.len() > MAX_FRAME_SIZE {
        bail!("Linux TUN IPC 消息过大");
    }
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(&bytes)?;
    stream.flush()?;
    Ok(())
}

fn read_frame<T: for<'de> Deserialize<'de>>(stream: &mut UnixStream) -> Result<T> {
    let mut length = [0u8; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > MAX_FRAME_SIZE {
        bail!("Linux TUN IPC 消息长度无效");
    }
    let mut bytes = vec![0u8; length];
    stream.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).context("Linux TUN IPC 消息格式无效")
}

fn user_state_directory(uid: libc::uid_t) -> PathBuf {
    Path::new(SERVICE_STATE_ROOT)
        .join("users")
        .join(uid.to_string())
}

fn runtime_directory(uid: libc::uid_t) -> PathBuf {
    user_state_directory(uid).join("runtime")
}

fn installed_kernel_path() -> PathBuf {
    Path::new(SERVICE_ROOT)
        .join("kernel")
        .join(env!("PURE_CLASH_DEFAULT_MIHOMO_VERSION"))
        .join(kernel_binary_name())
}

fn require_root(label: &str) -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        bail!("{label}未获得 root 权限");
    }
    Ok(())
}

fn pkexec_uid() -> Result<libc::uid_t> {
    env::var("PKEXEC_UID")
        .context("Linux TUN 服务安装器缺少 PKEXEC_UID")?
        .parse()
        .context("PKEXEC_UID 无效")
}

fn parse_u32(value: Option<OsString>, label: &str) -> Result<u32> {
    value
        .ok_or_else(|| anyhow!("Linux TUN 内部模式缺少{label}"))?
        .into_string()
        .map_err(|_| anyhow!("Linux TUN 内部模式的{label}不是 UTF-8"))?
        .parse()
        .with_context(|| format!("Linux TUN 内部模式的{label}无效"))
}

fn parse_uid(value: Option<OsString>, label: &str) -> Result<libc::uid_t> {
    parse_u32(value, label).map(|value| value as libc::uid_t)
}

fn required_path(value: Option<OsString>, label: &str) -> Result<PathBuf> {
    value
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("Linux TUN 内部模式缺少{label}"))
}

fn reject_extra_args(mut args: impl Iterator<Item = OsString>) -> Result<()> {
    if args.next().is_some() {
        bail!("Linux TUN 内部模式收到多余参数");
    }
    Ok(())
}

fn output_detail(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if !stderr.trim().is_empty() {
        stderr.trim()
    } else if !stdout.trim().is_empty() {
        stdout.trim()
    } else {
        "命令未返回详情"
    };
    detail
        .chars()
        .rev()
        .take(480)
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

fn error_chain(error: &anyhow::Error, max_chars: usize) -> String {
    format!("{error:#}").chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_modes_are_explicit_and_have_no_child_helper() {
        assert!(is_internal_mode(Some(OsStr::new(INSTALL_ARG))));
        assert!(is_internal_mode(Some(OsStr::new(SERVICE_ARG))));
        assert!(!is_internal_mode(Some(OsStr::new(
            "--linux-tun-service-child"
        ))));
        assert!(!is_internal_mode(Some(OsStr::new("dev"))));
    }

    #[test]
    fn service_paths_use_protected_runtime_and_fixed_kernel() {
        assert_eq!(
            Path::new(SERVICE_SOCKET),
            Path::new("/run/pure-clash-service/service.sock")
        );
        assert_eq!(
            runtime_directory(1000),
            Path::new("/var/lib/pure-clash-service/users/1000/runtime")
        );
        assert!(installed_kernel_path().starts_with(SERVICE_ROOT));
        assert_eq!(
            installed_kernel_path().file_name().and_then(OsStr::to_str),
            Some(kernel_binary_name())
        );
    }

    #[test]
    fn service_unit_matches_root_service_model() {
        let unit = service_unit(1000, 1001);
        assert!(unit.contains("--linux-tun-service 1000"));
        assert!(unit.contains("Group=1001"));
        assert!(unit.contains("After=network-online.target nftables.service iptables.service"));
        assert!(unit.contains("Restart=always"));
        assert!(unit.contains("RuntimeDirectory=pure-clash-service"));
        assert!(!unit.contains("User="));
        assert!(!unit.contains("AmbientCapabilities"));
    }

    #[test]
    fn runtime_yaml_requires_tun_and_loopback_control() {
        let valid = "allow-lan: false\nbind-address: 127.0.0.1\nexternal-controller: 127.0.0.1:9097\ntun:\n  enable: true\n";
        assert!(validate_runtime_yaml(valid).is_ok());
        assert!(validate_runtime_yaml(&valid.replace("enable: true", "enable: false")).is_err());
        assert!(validate_runtime_yaml(&valid.replace("127.0.0.1:9097", "0.0.0.0:9097")).is_err());
    }

    #[test]
    fn destination_rejects_traversal_and_reserved_files() {
        assert_eq!(
            destination_key(Path::new("providers/a.yaml")).unwrap(),
            "providers/a.yaml"
        );
        assert!(destination_key(Path::new("../etc/passwd")).is_err());
        assert!(destination_key(Path::new(RUNTIME_CONFIG)).is_err());
        assert!(destination_key(Path::new(".asset.staging-1")).is_err());
    }

    #[test]
    fn protocol_start_contains_bundle_instead_of_client_kernel_path() {
        let request = ServiceRequest::Start {
            bundle: RuntimeBundle {
                yaml: "tun: { enable: true }".to_owned(),
                assets: vec![],
                remote_providers: vec![],
            },
        };
        let encoded = serde_json::to_string(&request).unwrap();
        assert!(encoded.contains("\"bundle\""));
        assert!(!encoded.contains("config_file"));
        assert!(!encoded.contains("kernel"));
    }
}
