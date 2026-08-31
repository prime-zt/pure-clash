//! Mihomo Geo 数据的随包安装、完整性记录与显式在线更新。
//!
//! 三份基础数据作为安装资源随应用发布，首次启动复制到 Mihomo 用户数据目录。
//! 配置校验只执行离线完整性检查，不再隐式联网；用户可在设置页显式更新到
//! MetaCubeX 官方 `release` 分支的同一提交快照。

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::platform::{AppPaths, file::atomic_write};

const MANIFEST_FILE: &str = "manifest.json";
const COMPLETION_FILE: &str = ".pure-clash-geodata.json";
const STATE_SCHEMA_VERSION: u32 = 1;
const MIN_ONLINE_GEODATA_BYTES: u64 = 1024;
const MAX_GEODATA_BYTES: u64 = 64 * 1024 * 1024;
const OFFICIAL_REPOSITORY: &str = "https://github.com/MetaCubeX/meta-rules-dat";
const RELEASE_BRANCH: &str = "release";
const RELEASE_BRANCH_API: &str =
    "https://api.github.com/repos/MetaCubeX/meta-rules-dat/branches/release";
const RAW_BASE_URL: &str = "https://raw.githubusercontent.com/MetaCubeX/meta-rules-dat";
const EXPECTED_FILES: &[&str] = &["Country.mmdb", "GeoIP.dat", "GeoSite.dat"];

/// 串行化启动校验、订阅校验与设置页更新，避免三文件事务互相穿插。
static GEODATA_LOCK: Mutex<()> = Mutex::new(());

/// 随包 Geo 数据清单；是构建、打包、初始化和在线更新的共同数据源。
#[derive(Clone, Debug, Deserialize)]
struct BundledManifest {
    /// 清单结构版本，当前固定为 1。
    schema_version: u32,
    /// 官方源码仓库，必须是 MetaCubeX/meta-rules-dat。
    source_repository: String,
    /// 随包快照来源分支，当前固定为 release。
    source_branch: String,
    /// 随包快照的 40 位 Git commit SHA。
    source_commit: String,
    /// 数据许可证 SPDX 标识。
    license: String,
    /// 上游许可证固定版本链接。
    license_url: String,
    /// 随包 LICENSE 文件的 SHA-256。
    license_sha256: String,
    /// 必须包含 GeoSite.dat、GeoIP.dat 和 Country.mmdb 三项。
    files: Vec<ManifestFile>,
}

/// 清单中的单个 Geo 数据文件。
#[derive(Clone, Debug, Deserialize)]
struct ManifestFile {
    /// Mihomo 数据目录中的目标文件名。
    name: String,
    /// 官方 release 分支中的源文件名。
    upstream_path: String,
    /// 随包文件字节数。
    size: u64,
    /// 随包文件 SHA-256，小写十六进制。
    sha256: String,
}

/// 用户数据目录中的安装完成标记。
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct CompletionState {
    /// 状态结构版本，当前固定为 1。
    schema_version: u32,
    /// 数据来源：`bundled` 或 `official-update`。
    source: String,
    /// 三份数据所属的同一官方 Git commit SHA。
    source_commit: String,
    /// 最近一次完整提交的 UNIX 秒时间戳。
    updated_at: u64,
    /// 文件名到已提交大小和 SHA-256 的映射。
    files: BTreeMap<String, CompletedFile>,
}

/// 已提交文件的完整性信息。
#[derive(Clone, Debug, Deserialize, Serialize)]
struct CompletedFile {
    /// 文件字节数。
    size: u64,
    /// 文件 SHA-256，小写十六进制。
    sha256: String,
}

/// 设置页展示的当前 Geo 数据信息。
#[derive(Clone, Debug)]
pub(crate) struct GeodataInfo {
    /// 当前官方 Git commit SHA。
    pub(crate) revision: String,
    /// 最近安装或更新的 UNIX 秒时间戳。
    pub(crate) updated_at: u64,
}

/// 设置页手动更新结果。
#[derive(Clone, Debug)]
pub(crate) enum UpdateOutcome {
    /// 当前数据已对应官方 release 最新提交。
    UpToDate(GeodataInfo),
    /// 已下载并原子切换到新的官方提交。
    Updated(GeodataInfo),
}

/// GitHub branch API 响应，仅解析更新所需的提交字段。
#[derive(Deserialize)]
struct BranchResponse {
    /// release 分支当前提交。
    commit: BranchCommit,
}

/// GitHub branch API 中的提交摘要。
#[derive(Deserialize)]
struct BranchCommit {
    /// 40 位 Git commit SHA。
    sha: String,
}

struct PreparedFile {
    name: String,
    bytes: Vec<u8>,
    sha256: String,
}

/// 首次启动安装随包 Geo 数据；已有完整在线更新时不回退覆盖。
pub(crate) fn ensure_bundled(paths: &AppPaths) -> Result<GeodataInfo> {
    let _guard = GEODATA_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    ensure_bundled_locked(paths)
}

fn ensure_bundled_locked(paths: &AppPaths) -> Result<GeodataInfo> {
    fs::create_dir_all(&paths.mihomo_data_dir).with_context(|| {
        format!(
            "无法创建 Mihomo 数据目录：{}",
            paths.mihomo_data_dir.display()
        )
    })?;
    let manifest = read_bundled_manifest(paths)?;
    let state = read_completion_state(&paths.mihomo_data_dir);
    if state_is_complete(&paths.mihomo_data_dir, &state, &manifest.files) {
        return Ok(info_from_state(&state));
    }

    let resource_root = bundled_resource_root(paths)?;
    let mut prepared = Vec::with_capacity(manifest.files.len());
    for file in &manifest.files {
        let file_path = resource_root.join(&file.name);
        let bytes = fs::read(&file_path)
            .with_context(|| format!("无法读取随包 Geo 数据：{}", file_path.display()))?;
        validate_payload(&file.name, &bytes, Some(file.size), Some(&file.sha256))?;
        prepared.push(PreparedFile {
            name: file.name.clone(),
            sha256: file.sha256.clone(),
            bytes,
        });
    }
    let state = state_for_files("bundled", &manifest.source_commit, &prepared);
    commit_files(paths, &prepared, &state)?;
    Ok(info_from_state(&state))
}

/// 显式更新到 MetaCubeX 官方 release 分支的同一提交快照。
pub(crate) fn update_from_official(paths: &AppPaths) -> Result<UpdateOutcome> {
    let _guard = GEODATA_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let current = ensure_bundled_locked(paths)?;
    let manifest = read_bundled_manifest(paths)?;
    let latest = fetch_release_commit()?;
    if latest == current.revision {
        return Ok(UpdateOutcome::UpToDate(current));
    }

    let mut prepared = Vec::with_capacity(manifest.files.len());
    for file in &manifest.files {
        let bytes = download_at_commit(&latest, file)?;
        let sha256 = sha256_bytes(&bytes);
        prepared.push(PreparedFile {
            name: file.name.clone(),
            bytes,
            sha256,
        });
    }
    let state = state_for_files("official-update", &latest, &prepared);
    commit_files(paths, &prepared, &state)?;
    Ok(UpdateOutcome::Updated(info_from_state(&state)))
}

/// 每次执行内核配置校验前离线复核整套 Geo 数据。
///
/// 不解析 YAML 猜测实际依赖，避免少见规则写法漏检后触发 Mihomo 自行下载。
pub(super) fn prepare_for_config(paths: &AppPaths, _config_file: &Path) -> Result<()> {
    ensure_bundled(paths)?;
    Ok(())
}

fn fetch_release_commit() -> Result<String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build();
    let response = agent
        .get(RELEASE_BRANCH_API)
        .set("User-Agent", "pure-clash/geodata-updater")
        .call()
        .map_err(|error| anyhow!("{error}"))
        .context("无法查询官方 Geo 数据版本")?;
    let branch: BranchResponse = response
        .into_json()
        .context("官方 Geo 数据版本响应格式无效")?;
    validate_commit(&branch.commit.sha)?;
    Ok(branch.commit.sha.to_ascii_lowercase())
}

fn download_at_commit(commit: &str, file: &ManifestFile) -> Result<Vec<u8>> {
    validate_commit(commit)?;
    if !safe_single_name(&file.upstream_path) {
        bail!("Geo 数据清单包含不安全的上游路径");
    }
    let url = format!("{RAW_BASE_URL}/{commit}/{}", file.upstream_path);
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .build();
    let response = agent
        .get(&url)
        .set("User-Agent", "pure-clash/geodata-updater")
        .call()
        .map_err(|error| anyhow!("{error}"))
        .with_context(|| format!("无法下载官方 Geo 数据 {}", file.name))?;
    if response
        .header("Content-Length")
        .and_then(|length| length.parse::<u64>().ok())
        .is_some_and(|length| length == 0 || length > MAX_GEODATA_BYTES)
    {
        bail!("官方 Geo 数据 {} 响应大小无效", file.name);
    }

    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_GEODATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("下载官方 Geo 数据 {} 中断", file.name))?;
    validate_payload(&file.name, &bytes, None, None)?;
    Ok(bytes)
}

fn commit_files(
    paths: &AppPaths,
    prepared: &[PreparedFile],
    state: &CompletionState,
) -> Result<()> {
    fs::create_dir_all(&paths.mihomo_data_dir)?;
    let state_path = paths.mihomo_data_dir.join(COMPLETION_FILE);
    let previous_state = fs::read(&state_path).ok();
    let previous_files = prepared
        .iter()
        .map(|file| {
            let path = paths.mihomo_data_dir.join(&file.name);
            (path.clone(), fs::read(&path).ok())
        })
        .collect::<Vec<_>>();

    let result = (|| {
        for file in prepared {
            atomic_write(&paths.mihomo_data_dir.join(&file.name), &file.bytes)
                .with_context(|| format!("无法提交 Geo 数据 {}", file.name))?;
        }
        write_completion_state(&state_path, state)
    })();
    if let Err(error) = result {
        let rollback = rollback_files(&previous_files, &state_path, previous_state.as_deref());
        return match rollback {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(anyhow!(
                "{error:#}；恢复原 Geo 数据也失败：{rollback_error:#}"
            )),
        };
    }
    Ok(())
}

fn rollback_files(
    previous_files: &[(PathBuf, Option<Vec<u8>>)],
    state_path: &Path,
    previous_state: Option<&[u8]>,
) -> Result<()> {
    for (path, previous) in previous_files {
        match previous {
            Some(bytes) => atomic_write(path, bytes)?,
            None if path.exists() => fs::remove_file(path)?,
            None => {}
        }
    }
    match previous_state {
        Some(bytes) => atomic_write(state_path, bytes)?,
        None if state_path.exists() => fs::remove_file(state_path)?,
        None => {}
    }
    Ok(())
}

fn write_completion_state(path: &Path, state: &CompletionState) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(state).context("无法序列化 Geo 数据状态")?;
    bytes.push(b'\n');
    atomic_write(path, &bytes).context("无法写入 Geo 数据完成标记")
}

fn read_bundled_manifest(paths: &AppPaths) -> Result<BundledManifest> {
    let root = bundled_resource_root(paths)?;
    let path = root.join(MANIFEST_FILE);
    let bytes = fs::read(&path)
        .with_context(|| format!("无法读取随包 Geo 数据清单：{}", path.display()))?;
    let manifest: BundledManifest = serde_json::from_slice(&bytes)
        .with_context(|| format!("随包 Geo 数据清单格式无效：{}", path.display()))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn bundled_resource_root(paths: &AppPaths) -> Result<PathBuf> {
    if paths.geodata_resource_dir.join(MANIFEST_FILE).is_file() {
        return Ok(paths.geodata_resource_dir.clone());
    }
    #[cfg(any(debug_assertions, test))]
    {
        let development = Path::new(env!("CARGO_MANIFEST_DIR")).join("geodata");
        if development.join(MANIFEST_FILE).is_file() {
            return Ok(development);
        }
    }
    bail!(
        "随包 Geo 数据资源不可用：{}",
        paths.geodata_resource_dir.display()
    )
}

fn validate_manifest(manifest: &BundledManifest) -> Result<()> {
    if manifest.schema_version != 1
        || manifest.source_repository != OFFICIAL_REPOSITORY
        || manifest.source_branch != RELEASE_BRANCH
        || manifest.license != "GPL-3.0"
        || manifest.license_url.is_empty()
        || !valid_sha256(&manifest.license_sha256)
    {
        bail!("随包 Geo 数据清单元数据无效");
    }
    validate_commit(&manifest.source_commit)?;
    let mut names = BTreeSet::new();
    for file in &manifest.files {
        if !safe_single_name(&file.name)
            || !safe_single_name(&file.upstream_path)
            || expected_upstream_path(&file.name) != Some(file.upstream_path.as_str())
            || file.size == 0
            || file.size > MAX_GEODATA_BYTES
            || !valid_sha256(&file.sha256)
            || !names.insert(file.name.as_str())
        {
            bail!("随包 Geo 数据清单文件条目无效");
        }
    }
    if names != EXPECTED_FILES.iter().copied().collect() {
        bail!("随包 Geo 数据清单必须且只能包含三份基础数据");
    }
    Ok(())
}

fn expected_upstream_path(name: &str) -> Option<&'static str> {
    match name {
        "GeoSite.dat" => Some("geosite.dat"),
        "GeoIP.dat" => Some("geoip.dat"),
        "Country.mmdb" => Some("country.mmdb"),
        _ => None,
    }
}

fn state_for_files(source: &str, commit: &str, files: &[PreparedFile]) -> CompletionState {
    CompletionState {
        schema_version: STATE_SCHEMA_VERSION,
        source: source.to_owned(),
        source_commit: commit.to_owned(),
        updated_at: now_secs(),
        files: files
            .iter()
            .map(|file| {
                (
                    file.name.clone(),
                    CompletedFile {
                        size: file.bytes.len() as u64,
                        sha256: file.sha256.clone(),
                    },
                )
            })
            .collect(),
    }
}

fn read_completion_state(data_dir: &Path) -> CompletionState {
    fs::read(data_dir.join(COMPLETION_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn state_is_complete(
    data_dir: &Path,
    state: &CompletionState,
    manifest_files: &[ManifestFile],
) -> bool {
    state.schema_version == STATE_SCHEMA_VERSION
        && matches!(state.source.as_str(), "bundled" | "official-update")
        && validate_commit(&state.source_commit).is_ok()
        && state.updated_at > 0
        && state.files.len() == manifest_files.len()
        && manifest_files
            .iter()
            .all(|file| completed_file_is_valid(data_dir, file, state))
}

fn completed_file_is_valid(
    data_dir: &Path,
    manifest_file: &ManifestFile,
    state: &CompletionState,
) -> bool {
    let Some(completed) = state.files.get(&manifest_file.name) else {
        return false;
    };
    if completed.size == 0 || !valid_sha256(&completed.sha256) {
        return false;
    }
    let path = data_dir.join(&manifest_file.name);
    if !fs::metadata(&path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() == completed.size)
    {
        return false;
    }

    // 完成标记不能单独证明文件未损坏；启动和配置校验时复核真实内容哈希。
    sha256_file(&path).is_ok_and(|actual| actual.eq_ignore_ascii_case(&completed.sha256))
}

fn validate_payload(
    name: &str,
    bytes: &[u8],
    expected_size: Option<u64>,
    expected_sha256: Option<&str>,
) -> Result<()> {
    let size = bytes.len() as u64;
    if size == 0 || size > MAX_GEODATA_BYTES || expected_size.is_some_and(|value| value != size) {
        bail!("Geo 数据 {name} 大小无效");
    }
    if expected_size.is_none() {
        let prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
        if size < MIN_ONLINE_GEODATA_BYTES
            || prefix.contains("<!doctype html")
            || prefix.contains("<html")
            || prefix.starts_with("version https://git-lfs.github.com/spec/")
        {
            bail!("Geo 数据 {name} 响应不是有效的数据库文件");
        }
    }
    let actual = sha256_bytes(bytes);
    if expected_sha256.is_some_and(|expected| !actual.eq_ignore_ascii_case(expected)) {
        bail!("Geo 数据 {name} SHA-256 不匹配");
    }
    Ok(())
}

fn info_from_state(state: &CompletionState) -> GeodataInfo {
    GeodataInfo {
        revision: state.source_commit.clone(),
        updated_at: state.updated_at,
    }
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut input = fs::File::open(path)
        .with_context(|| format!("无法打开 Geo 数据进行校验：{}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = input
            .read(&mut buffer)
            .with_context(|| format!("无法读取 Geo 数据进行校验：{}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_commit(value: &str) -> Result<()> {
    if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("Geo 数据版本不是有效的 Git commit SHA");
    }
    Ok(())
}

fn safe_single_name(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn test_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时间应晚于 UNIX_EPOCH")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "pure-clash-geodata-{name}-{}-{nonce}",
            std::process::id()
        ))
    }

    fn write_test_bundle(root: &Path, commit: &str, payloads: &[(&str, &str, &[u8])]) {
        let resource_dir = root.join("geodata");
        fs::create_dir_all(&resource_dir).unwrap();
        let files = payloads
            .iter()
            .map(|(name, upstream_path, bytes)| {
                fs::write(resource_dir.join(name), bytes).unwrap();
                serde_json::json!({
                    "name": name,
                    "upstream_path": upstream_path,
                    "size": bytes.len(),
                    "sha256": sha256_bytes(bytes),
                })
            })
            .collect::<Vec<_>>();
        let manifest = serde_json::json!({
            "schema_version": 1,
            "source_repository": OFFICIAL_REPOSITORY,
            "source_branch": RELEASE_BRANCH,
            "source_commit": commit,
            "license": "GPL-3.0",
            "license_url": "https://example.invalid/LICENSE",
            "license_sha256": "0".repeat(64),
            "files": files,
        });
        fs::write(
            resource_dir.join(MANIFEST_FILE),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn bundled_manifest_is_complete_and_pinned() {
        let manifest: BundledManifest =
            serde_json::from_str(include_str!("../../geodata/manifest.json")).unwrap();
        validate_manifest(&manifest).unwrap();
        assert_eq!(
            manifest.source_commit,
            env!("PURE_CLASH_BUNDLED_GEODATA_REVISION")
        );
        assert_eq!(manifest.files.len(), 3);
    }

    #[test]
    fn manifest_pins_expected_upstream_paths() {
        assert_eq!(expected_upstream_path("GeoSite.dat"), Some("geosite.dat"));
        assert_eq!(expected_upstream_path("GeoIP.dat"), Some("geoip.dat"));
        assert_eq!(expected_upstream_path("Country.mmdb"), Some("country.mmdb"));
        assert_eq!(expected_upstream_path("unknown.dat"), None);
    }

    #[test]
    fn rejects_online_placeholder_payloads() {
        assert!(validate_payload("GeoSite.dat", b"<html>error</html>", None, None).is_err());
        assert!(
            validate_payload(
                "GeoIP.dat",
                b"version https://git-lfs.github.com/spec/v1\n",
                None,
                None
            )
            .is_err()
        );
    }

    #[test]
    fn parses_official_branch_response_and_rejects_invalid_sha() {
        let response: BranchResponse = serde_json::from_str(
            r#"{"commit":{"sha":"d2a52e8ab09378b8818c8ad6e39198e5788ccfbb"}}"#,
        )
        .unwrap();
        assert!(validate_commit(&response.commit.sha).is_ok());
        assert!(validate_commit("release").is_err());
    }

    #[test]
    fn bundled_restore_preserves_complete_updates_and_repairs_corruption() {
        let root = test_dir("restore");
        let bundled_commit = "1".repeat(40);
        let online_commit = "2".repeat(40);
        let bundled = [
            ("GeoSite.dat", "geosite.dat", b"bundled-site".as_slice()),
            ("GeoIP.dat", "geoip.dat", b"bundled-ip".as_slice()),
            (
                "Country.mmdb",
                "country.mmdb",
                b"bundled-country".as_slice(),
            ),
        ];
        write_test_bundle(&root, &bundled_commit, &bundled);
        let paths = AppPaths::portable(&root);

        let first = ensure_bundled(&paths).expect("应从随包资源完成离线安装");
        assert_eq!(first.revision, bundled_commit);

        let online = [
            PreparedFile {
                name: "GeoSite.dat".to_owned(),
                bytes: b"official-site".to_vec(),
                sha256: sha256_bytes(b"official-site"),
            },
            PreparedFile {
                name: "GeoIP.dat".to_owned(),
                bytes: b"official-ip".to_vec(),
                sha256: sha256_bytes(b"official-ip"),
            },
            PreparedFile {
                name: "Country.mmdb".to_owned(),
                bytes: b"official-country".to_vec(),
                sha256: sha256_bytes(b"official-country"),
            },
        ];
        let online_state = state_for_files("official-update", &online_commit, &online);
        commit_files(&paths, &online, &online_state).expect("应提交模拟在线更新");

        let preserved = ensure_bundled(&paths).expect("应保留完整的在线更新");
        assert_eq!(preserved.revision, online_commit);
        assert_eq!(
            fs::read(paths.mihomo_data_dir.join("GeoSite.dat")).unwrap(),
            b"official-site"
        );

        // 保持长度不变地篡改文件，确认完整性判断会计算真实哈希而非只看大小。
        fs::write(paths.mihomo_data_dir.join("GeoSite.dat"), b"tampered-site").unwrap();
        let repaired = ensure_bundled(&paths).expect("损坏时应离线恢复整套随包快照");
        assert_eq!(repaired.revision, bundled_commit);
        assert_eq!(
            fs::read(paths.mihomo_data_dir.join("GeoSite.dat")).unwrap(),
            b"bundled-site"
        );

        fs::remove_dir_all(root).expect("应清理 Geo 数据测试目录");
    }
}
