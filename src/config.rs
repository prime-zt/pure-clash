use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::platform::AppPaths;

/// 默认配置只提供一个内置 `DIRECT` 节点，确保首次启动无需订阅即可运行。
const DEFAULT_MIHOMO_CONFIG: &str = include_str!("../config/mihomo/default.yaml");

fn default_mihomo_version() -> String {
    // 版本由构建脚本从随包内核 manifest 注入，避免在运行时代码中重复维护。
    env!("PURE_CLASH_DEFAULT_MIHOMO_VERSION").to_owned()
}

/// 应用支持的界面语言；JSON 使用稳定的 BCP-47 风格标识保存。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Language {
    /// 简体中文，也是首次启动时的默认语言。
    #[default]
    #[serde(rename = "zh-CN")]
    Chinese,
    /// 美式英文。
    #[serde(rename = "en-US")]
    English,
}

impl Language {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Chinese => "zh-CN",
            Self::English => "en-US",
        }
    }

    pub(crate) fn toggled(self) -> Self {
        match self {
            Self::Chinese => Self::English,
            Self::English => Self::Chinese,
        }
    }
}

/// 应用支持的界面主题，JSON 中使用小写字符串保存。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Theme {
    /// 深色主题。
    Dark,
    /// 浅色主题，也是首次启动时的默认值。
    #[default]
    Light,
}

impl Theme {
    pub(crate) fn is_dark(self) -> bool {
        self == Self::Dark
    }

    pub(crate) fn toggled(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }
}

/// 订阅/导入配置的元数据；YAML 内容按 id 存放在 config/profiles/ 目录。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProfileMeta {
    /// 随机生成的安全文件名；不使用用户输入，避免路径穿越。
    pub(crate) id: String,
    /// 界面显示名称。
    pub(crate) name: String,
    /// 订阅地址；本地导入的配置为 None。
    pub(crate) url: Option<String>,
    /// 添加时间（UNIX 秒）。
    pub(crate) added_at: u64,
    /// 最近一次内容更新时间（UNIX 秒）。
    pub(crate) updated_at: u64,
    /// 自动更新间隔（分钟）；0 表示不自动更新（默认，兼容旧配置）。
    #[serde(default)]
    pub(crate) update_interval_minutes: u64,
    /// 最近一次自动更新尝试（UNIX 秒，无论成败）；失败也记录以顺延到期，
    /// 避免失败订阅每分钟到期被反复重试。
    #[serde(default)]
    pub(crate) last_auto_attempt_at: u64,
}

/// Pure Clash 主配置；新增字段必须提供默认值以兼容旧版配置文件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct AppConfig {
    /// 启动时使用的 Mihomo 内核版本，默认取随包内核 manifest 的版本。
    #[serde(default = "default_mihomo_version")]
    pub(crate) mihomo_version: String,
    /// 上次启动版本所携带的内核版本，用于区分“跟随随包版本”和未来的手动选择。
    /// 旧配置缺少该字段时为 None，首次读取后会补写当前随包版本。
    #[serde(default)]
    pub(crate) bundled_mihomo_version: Option<String>,
    /// 界面主题，可选值为 `dark` 或 `light`，默认为 `dark`。
    pub(crate) theme: Theme,
    /// 界面语言，可选值为 `zh-CN` 或 `en-US`，默认为 `zh-CN`。
    pub(crate) language: Language,
    /// 强制为每条连接匹配发起进程（注入 `find-process-mode: always`）；
    /// 关闭时不注入，使用内核默认行为。开关以此文件为唯一事实来源。
    pub(crate) find_process_always: bool,
    /// 订阅与导入的配置列表，按添加顺序展示。
    pub(crate) profiles: Vec<ProfileMeta>,
    /// 当前激活的配置 id；None 表示使用内置默认配置。
    pub(crate) active_profile: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            mihomo_version: default_mihomo_version(),
            bundled_mihomo_version: Some(default_mihomo_version()),
            theme: Theme::default(),
            language: Language::default(),
            find_process_always: false,
            profiles: Vec::new(),
            active_profile: None,
        }
    }
}

impl AppConfig {
    /// 将当前配置立即写回 `config/app.json`。
    pub(crate) fn save(&self, path: &Path) -> Result<()> {
        let mut json = serde_json::to_string_pretty(self).context("无法序列化应用配置")?;
        json.push('\n');
        crate::platform::file::atomic_write(path, json.as_bytes())
            .with_context(|| format!("无法写入应用配置：{}", path.display()))
    }
}

/// 启动时完成目录初始化后交给应用实体维护的配置状态。
pub(crate) struct LoadedConfig {
    /// 从磁盘读取或按默认值创建的主配置对象。
    pub(crate) config: AppConfig,
    /// 当前平台的程序、配置、数据和内核目录集合。
    pub(crate) paths: AppPaths,
    /// 启动时完成离线安装并校验后的 Geo 数据版本信息。
    pub(crate) geodata_info: crate::mihomo::geodata::GeodataInfo,
}

/// 按当前平台目录策略初始化配置、数据目录和主配置文件。
pub(crate) fn load_or_create() -> Result<LoadedConfig> {
    load_or_create_with_paths(AppPaths::from_current_exe()?)
}

#[cfg(test)]
fn load_or_create_in(app_dir: &Path) -> Result<LoadedConfig> {
    load_or_create_with_paths(AppPaths::portable(app_dir))
}

fn load_or_create_with_paths(paths: AppPaths) -> Result<LoadedConfig> {
    let config_path = &paths.config_file;

    fs::create_dir_all(&paths.config_dir)
        .with_context(|| format!("无法创建配置目录：{}", paths.config_dir.display()))?;
    fs::create_dir_all(&paths.data_dir)
        .with_context(|| format!("无法创建数据目录：{}", paths.data_dir.display()))?;
    fs::create_dir_all(&paths.mihomo_config_dir).with_context(|| {
        format!(
            "无法创建 Mihomo 配置目录：{}",
            paths.mihomo_config_dir.display()
        )
    })?;
    fs::create_dir_all(&paths.mihomo_data_dir).with_context(|| {
        format!(
            "无法创建 Mihomo 数据目录：{}",
            paths.mihomo_data_dir.display()
        )
    })?;
    fs::create_dir_all(&paths.profiles_dir)
        .with_context(|| format!("无法创建配置订阅目录：{}", paths.profiles_dir.display()))?;

    // Geo 规则库随安装包分发，首次启动只从本地只读资源复制到 Mihomo 数据目录。
    // 初始化失败时直接中止，避免后续订阅校验退化为依赖用户网络的隐式下载。
    let geodata_info =
        crate::mihomo::geodata::ensure_bundled(&paths).context("无法初始化随包 Geo 数据")?;

    if !paths.default_mihomo_config_file.exists() {
        // 默认文件只在缺失时生成，后续启动不得覆盖用户已经编辑的 YAML。
        crate::platform::file::atomic_write(
            &paths.default_mihomo_config_file,
            DEFAULT_MIHOMO_CONFIG.as_bytes(),
        )
        .with_context(|| {
            format!(
                "无法创建默认 Mihomo 配置：{}",
                paths.default_mihomo_config_file.display()
            )
        })?;
    }

    let mut config = if config_path.exists() {
        let content = fs::read_to_string(config_path)
            .with_context(|| format!("无法读取应用配置：{}", config_path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("应用配置格式无效：{}", config_path.display()))?
    } else {
        let config = AppConfig::default();
        config.save(config_path)?;
        config
    };

    // 随包内核升级时，仍跟随旧随包版本或已经失效的选择自动迁移到当前版本。
    // marker 引入前项目没有内核选择界面，因此旧 schema 可安全视为跟随随包版本。
    let bundled_version = default_mihomo_version();
    let follows_bundled = config
        .bundled_mihomo_version
        .as_deref()
        .is_none_or(|previous| config.mihomo_version == previous);
    let selected_available = crate::kernel::is_available(&paths, &config.mihomo_version);
    let mut config_changed = false;
    if (follows_bundled || !selected_available) && config.mihomo_version != bundled_version {
        config.mihomo_version.clone_from(&bundled_version);
        config_changed = true;
    }
    if config.bundled_mihomo_version.as_deref() != Some(&bundled_version) {
        config.bundled_mihomo_version = Some(bundled_version);
        config_changed = true;
    }
    if config_changed {
        config.save(config_path)?;
    }

    Ok(LoadedConfig {
        config,
        paths,
        geodata_info,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn test_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时间应晚于 UNIX_EPOCH")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "pure-clash-config-{name}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn creates_directories_and_default_config() {
        let root = test_dir("create");
        let loaded = load_or_create_in(&root).expect("应完成首次配置初始化");

        assert_eq!(loaded.config.theme, Theme::Light);
        assert_eq!(loaded.config.language, Language::Chinese);
        assert_eq!(
            loaded.config.mihomo_version,
            env!("PURE_CLASH_DEFAULT_MIHOMO_VERSION")
        );
        assert!(root.join("config/app.json").is_file());
        assert!(root.join("data").is_dir());
        assert!(root.join("config/mihomo/default.yaml").is_file());
        assert!(root.join("data/mihomo").is_dir());
        assert_eq!(
            fs::read_to_string(root.join("config/mihomo/default.yaml"))
                .expect("应读取默认 Mihomo 配置"),
            DEFAULT_MIHOMO_CONFIG
        );

        let persisted: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(root.join("config/app.json")).expect("应读取默认配置文件"),
        )
        .expect("默认配置文件应是有效 JSON");
        assert_eq!(
            persisted["mihomo_version"],
            env!("PURE_CLASH_DEFAULT_MIHOMO_VERSION")
        );

        fs::remove_dir_all(root).expect("应清理测试目录");
    }

    #[test]
    fn loads_theme_and_defaults_missing_fields() {
        let root = test_dir("load");
        fs::create_dir_all(root.join("config")).expect("应创建测试配置目录");
        fs::write(root.join("config/app.json"), r#"{"theme":"light"}"#).expect("应写入测试配置");

        let loaded = load_or_create_in(&root).expect("应读取已有配置");
        assert_eq!(loaded.config.theme, Theme::Light);
        assert_eq!(loaded.config.language, Language::Chinese);
        assert_eq!(
            loaded.config.mihomo_version,
            env!("PURE_CLASH_DEFAULT_MIHOMO_VERSION")
        );

        fs::write(root.join("config/app.json"), r#"{"language":"en-US"}"#).expect("应写入英文配置");
        let english = load_or_create_in(&root).expect("应读取英文语言配置");
        assert_eq!(english.config.language, Language::English);

        fs::write(root.join("config/app.json"), "{}\n").expect("应写入缺省字段配置");
        let defaulted = load_or_create_in(&root).expect("缺少字段时应使用默认值");
        assert_eq!(defaulted.config.theme, Theme::Light);
        assert_eq!(
            defaulted.config.mihomo_version,
            env!("PURE_CLASH_DEFAULT_MIHOMO_VERSION")
        );

        fs::remove_dir_all(root).expect("应清理测试目录");
    }

    #[test]
    fn saves_config_changes_immediately() {
        let root = test_dir("save");
        let mut loaded = load_or_create_in(&root).expect("应完成配置初始化");
        let custom_kernel = root
            .join("kernel/test-version")
            .join(crate::platform::kernel_binary_name());
        fs::create_dir_all(custom_kernel.parent().unwrap()).expect("应创建自定义内核目录");
        fs::write(&custom_kernel, b"custom-kernel").expect("应创建自定义内核文件");
        loaded.config.theme = Theme::Light;
        loaded.config.language = Language::English;
        loaded.config.mihomo_version = "test-version".to_owned();
        loaded
            .config
            .save(&loaded.paths.config_file)
            .expect("应立即保存配置变更");

        let reloaded = load_or_create_in(&root).expect("应重新读取已保存配置");
        assert_eq!(reloaded.config.theme, Theme::Light);
        assert_eq!(reloaded.config.language, Language::English);
        assert_eq!(reloaded.config.mihomo_version, "test-version");

        fs::remove_dir_all(root).expect("应清理测试目录");
    }

    #[test]
    fn migrates_legacy_bundled_kernel_version_but_preserves_installed_custom_version() {
        let root = test_dir("kernel-migration");
        fs::create_dir_all(root.join("config")).expect("应创建测试配置目录");
        fs::write(
            root.join("config/app.json"),
            r#"{"mihomo_version":"1.18.0"}"#,
        )
        .expect("应写入旧 schema 配置");

        let migrated = load_or_create_in(&root).expect("应迁移旧随包内核版本");
        let bundled = env!("PURE_CLASH_DEFAULT_MIHOMO_VERSION");
        assert_eq!(migrated.config.mihomo_version, bundled);
        assert_eq!(
            migrated.config.bundled_mihomo_version.as_deref(),
            Some(bundled)
        );

        let mut following = migrated.config.clone();
        following.mihomo_version = "previous-bundled".to_owned();
        following.bundled_mihomo_version = Some("previous-bundled".to_owned());
        following.save(&root.join("config/app.json")).unwrap();
        let upgraded = load_or_create_in(&root).expect("应迁移仍跟随上次随包版本的配置");
        assert_eq!(upgraded.config.mihomo_version, bundled);

        let custom_version = "custom-version";
        let custom_kernel = root
            .join("kernel")
            .join(custom_version)
            .join(crate::platform::kernel_binary_name());
        fs::create_dir_all(custom_kernel.parent().unwrap()).expect("应创建自定义内核目录");
        fs::write(custom_kernel, b"custom-kernel").expect("应创建自定义内核文件");
        let mut custom = upgraded.config;
        custom.mihomo_version = custom_version.to_owned();
        custom.save(&root.join("config/app.json")).unwrap();

        let reloaded = load_or_create_in(&root).expect("应保留仍可用的自定义内核");
        assert_eq!(reloaded.config.mihomo_version, custom_version);
        fs::remove_dir_all(root).expect("应清理测试目录");
    }

    #[test]
    fn preserves_existing_default_mihomo_config() {
        let root = test_dir("preserve-mihomo");
        let config_dir = root.join("config/mihomo");
        fs::create_dir_all(&config_dir).expect("应创建 Mihomo 测试配置目录");
        let custom = "mode: direct\n";
        fs::write(config_dir.join("default.yaml"), custom).expect("应写入用户配置");

        load_or_create_in(&root).expect("应完成配置初始化");
        assert_eq!(
            fs::read_to_string(config_dir.join("default.yaml")).expect("应读取用户配置"),
            custom
        );

        fs::remove_dir_all(root).expect("应清理测试目录");
    }

    #[test]
    fn app_paths_use_platform_neutral_relative_layout() {
        let root = PathBuf::from(r"C:\Pure Clash");
        let paths = AppPaths::portable(&root);
        assert_eq!(paths.config_file, root.join("config/app.json"));
        assert_eq!(paths.data_dir, root.join("data"));
        assert_eq!(
            paths.default_mihomo_config_file,
            root.join("config/mihomo/default.yaml")
        );
        assert_eq!(paths.mihomo_data_dir, root.join("data/mihomo"));
        assert_eq!(paths.kernel_dir, root.join("kernel"));
        assert!(paths.config_display().contains("config"));
        assert!(paths.data_display().contains("data"));
    }
}
