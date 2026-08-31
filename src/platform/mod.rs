use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use directories::ProjectDirs;
use gpui::{Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
#[cfg(target_os = "linux")]
use gpui::{Pixels, WindowBackgroundAppearance, WindowDecorations};

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
compile_error!("Pure Clash 当前只规划 Windows、Linux 和 macOS 平台");

#[cfg(target_os = "windows")]
pub(crate) mod windows;

/// Linux 进程守护：父进程死亡信号保证异常退出时回收 Mihomo，等价 Windows Job Object。
#[cfg(target_os = "linux")]
pub(crate) mod linux;

/// 平台无关托盘：各平台提供同名 `SystemTray`，事件统一经 `TrayAction` 通道转发。
#[cfg(any(target_os = "windows", target_os = "linux"))]
pub(crate) mod tray;

pub(crate) mod file;
/// 内核子进程的平台守护与终止策略；各平台提供同名 `KernelProcessGuard`。
pub(crate) mod process_guard;

pub(crate) use process_guard::KernelProcessGuard;

#[cfg(any(target_os = "windows", target_os = "linux"))]
pub(crate) use tray::{SystemTray, TrayAction};

#[cfg(target_os = "linux")]
pub(crate) use linux::show_main_window;
#[cfg(target_os = "windows")]
pub(crate) use windows::show_main_window;

#[cfg(target_os = "linux")]
pub(crate) use linux::{SingleInstance, SingleInstanceState};
/// 单实例锁：Windows 用命名 Mutex/Event，Linux 用抽象命名空间 socket。
#[cfg(target_os = "windows")]
pub(crate) use windows::{SingleInstance, SingleInstanceState};

#[cfg(target_os = "linux")]
pub(crate) use linux::{
    ElevatedProcess, capture_system_proxy, launch_elevated, restore_system_proxy,
    run_elevated_helper_if_requested, set_system_proxy,
};
#[cfg(target_os = "windows")]
pub(crate) use windows::{
    ElevatedProcess, capture_system_proxy, launch_elevated, restore_system_proxy, set_system_proxy,
};

/// Linux 对齐 Clash Verge Rev 使用 gVisor；Windows 保持已验证的 mixed 栈，
/// 避免 Linux 修复改变现有 Wintun 行为。
pub(crate) fn tun_stack() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "gvisor"
    }
    #[cfg(not(target_os = "linux"))]
    {
        "mixed"
    }
}

/// Linux 使用与 Clash Verge Rev 一致的双栈 fake-IP；Windows 维持已有的
/// fake-IP v4 行为，避免 Linux 修复改变 Wintun 配置。
pub(crate) fn tun_dns_ipv6() -> bool {
    cfg!(target_os = "linux")
}

/// Linux 沿用锁定 Mihomo 的默认双栈地址；Windows 保留现有显式 v6 地址，
/// 确保 Wintun 的 IPv6 默认路由行为不因 Linux 兼容修复而变化。
pub(crate) fn tun_inet6_address() -> Option<&'static str> {
    if cfg!(target_os = "linux") {
        None
    } else {
        Some("fdfe:dcba:9876::1/126")
    }
}

/// macOS 提权启动占位：正式支持前 `launch_elevated` 直接失败，调用方按
/// "提权不可用"回退。
#[cfg(target_os = "macos")]
mod elevation_stub {
    use anyhow::Result;
    use std::path::Path;

    #[derive(Debug)]
    pub(crate) struct ElevatedProcess;

    impl ElevatedProcess {
        pub(crate) fn handle(&self) -> isize {
            0
        }

        pub(crate) fn is_running(&self) -> bool {
            false
        }

        pub(crate) fn terminate(&self) -> Result<()> {
            Ok(())
        }
    }

    pub(crate) fn launch_elevated(
        _exe: &Path,
        _data_dir: &Path,
        _config_file: &Path,
    ) -> Result<ElevatedProcess> {
        anyhow::bail!("当前平台暂不支持提权启动内核")
    }
}
#[cfg(target_os = "macos")]
pub(crate) use elevation_stub::{ElevatedProcess, launch_elevated};
#[cfg(target_os = "macos")]
pub(crate) use system_proxy_stub::{capture_system_proxy, restore_system_proxy, set_system_proxy};

/// macOS 系统代理入口：正式实现接入前统一报"暂不支持"。
#[cfg(target_os = "macos")]
mod system_proxy_stub {
    use super::SystemProxySnapshot;

    pub(crate) fn capture_system_proxy() -> anyhow::Result<SystemProxySnapshot> {
        anyhow::bail!("当前平台暂不支持系统代理")
    }

    pub(crate) fn set_system_proxy(_server: &str) -> anyhow::Result<()> {
        anyhow::bail!("当前平台暂不支持系统代理")
    }

    pub(crate) fn restore_system_proxy(_snapshot: &SystemProxySnapshot) -> anyhow::Result<()> {
        anyhow::bail!("当前平台暂不支持系统代理")
    }
}
/// 系统代理托管前的既有设置快照：关闭时恢复，异常退出后下次启动自愈。
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct SystemProxySnapshot {
    /// 是否已捕获用户既有设置；state 文件存在即代表客户端正在托管。
    pub(crate) managed: bool,
    /// 托管前系统代理是否已开启。
    pub(crate) prev_enabled: bool,
    /// 托管前的代理服务器地址。
    pub(crate) prev_server: String,
    /// Linux 桌面会话中被修改的 GSettings 原始值；Windows 旧状态文件缺少
    /// 此字段时按 `None` 兼容读取。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) linux: Option<LinuxSystemProxySnapshot>,
}

/// Linux GNOME/Cinnamon 系统代理的完整快照。
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct LinuxSystemProxySnapshot {
    /// `org.gnome.system.proxy` 或兼容的 Cinnamon schema 前缀。
    pub(crate) schema_prefix: String,
    /// 所有会被 Pure Clash 修改的键及其原始 GVariant 文本。
    pub(crate) settings: Vec<LinuxProxySetting>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct LinuxProxySetting {
    pub(crate) schema: String,
    pub(crate) key: String,
    pub(crate) value: String,
}

/// 与 Zed 客户端装饰保持一致的 Linux 窗口阴影边距和圆角尺寸。
#[cfg(target_os = "linux")]
pub(crate) const CLIENT_SIDE_DECORATION_SIZE: Pixels = px(10.0);

/// 应用运行时使用的目录集合，集中隔离各平台的目录约定。
#[derive(Clone, Debug)]
pub(crate) struct AppPaths {
    /// 程序可执行文件所在目录。
    pub(crate) program_dir: PathBuf,
    /// 应用配置目录。
    pub(crate) config_dir: PathBuf,
    /// 应用数据目录。
    pub(crate) data_dir: PathBuf,
    /// 主配置文件路径。
    pub(crate) config_file: PathBuf,
    /// Mihomo 配置文件目录。
    pub(crate) mihomo_config_dir: PathBuf,
    /// 首次启动生成的默认 Mihomo 配置文件。
    pub(crate) default_mihomo_config_file: PathBuf,
    /// Mihomo 的 geodata、缓存和 provider 数据目录。
    pub(crate) mihomo_data_dir: PathBuf,
    /// 客户端本地基线配置；保存端口、controller 与随机 secret 等本机字段。
    pub(crate) local_mihomo_config_file: PathBuf,
    /// 基线与激活配置合并后的运行时配置；内核 `-f` 实际加载的文件。
    pub(crate) runtime_mihomo_config_file: PathBuf,
    /// 订阅与导入的原始配置文件目录，按 profile 标识存放。
    pub(crate) profiles_dir: PathBuf,
    /// 随包内核根目录。
    pub(crate) kernel_dir: PathBuf,
    /// 随包 Geo 数据资源目录；运行时复制到 `mihomo_data_dir`，在线更新不修改此目录。
    pub(crate) geodata_resource_dir: PathBuf,
}

impl AppPaths {
    /// 构造 Windows 当前使用的便携式目录布局，也供开发测试复用。
    pub(crate) fn portable(program_dir: &Path) -> Self {
        let config_dir = program_dir.join("config");
        let data_dir = program_dir.join("data");
        let mihomo_config_dir = config_dir.join("mihomo");
        Self {
            program_dir: program_dir.to_path_buf(),
            config_file: config_dir.join("app.json"),
            default_mihomo_config_file: mihomo_config_dir.join("default.yaml"),
            local_mihomo_config_file: mihomo_config_dir.join("local.yaml"),
            runtime_mihomo_config_file: mihomo_config_dir.join("runtime.yaml"),
            profiles_dir: config_dir.join("profiles"),
            mihomo_config_dir,
            mihomo_data_dir: data_dir.join("mihomo"),
            config_dir,
            data_dir,
            kernel_dir: program_dir.join("kernel"),
            geodata_resource_dir: program_dir.join("geodata"),
        }
    }

    /// 根据目标平台解析配置、数据和内核目录。
    pub(crate) fn from_current_exe() -> Result<Self> {
        let executable = std::env::current_exe().context("无法确定 Pure Clash 可执行文件路径")?;
        let program_dir = executable.parent().context("可执行文件路径缺少父目录")?;

        #[cfg(target_os = "windows")]
        {
            // Windows 当前采用 per-user 安装，安装目录可写，保持程序同级目录约定。
            Ok(Self::portable(program_dir))
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            // macOS 应用包和 Linux 系统安装目录通常不可写，因此使用标准用户目录。
            let project_dirs = ProjectDirs::from("", "", "pure-clash")
                .context("无法确定当前平台的 Pure Clash 用户目录")?;
            let config_dir = project_dirs.config_dir().to_path_buf();
            let data_dir = project_dirs.data_local_dir().to_path_buf();
            let mihomo_config_dir = config_dir.join("mihomo");
            Ok(Self {
                program_dir: program_dir.to_path_buf(),
                config_file: config_dir.join("app.json"),
                default_mihomo_config_file: mihomo_config_dir.join("default.yaml"),
                local_mihomo_config_file: mihomo_config_dir.join("local.yaml"),
                runtime_mihomo_config_file: mihomo_config_dir.join("runtime.yaml"),
                profiles_dir: config_dir.join("profiles"),
                mihomo_config_dir,
                mihomo_data_dir: data_dir.join("mihomo"),
                config_dir,
                data_dir,
                kernel_dir: platform_kernel_dir(program_dir),
                geodata_resource_dir: platform_geodata_dir(program_dir),
            })
        }
    }

    /// 设置页使用本机路径分隔符展示主配置位置。
    pub(crate) fn config_display(&self) -> String {
        display_path(&self.program_dir, &self.config_file)
    }

    /// 设置页使用本机路径分隔符展示数据目录位置。
    pub(crate) fn data_display(&self) -> String {
        display_path(&self.program_dir, &self.data_dir)
    }

    /// 设置页展示内置默认配置的存放路径（首次初始化的来源文件）。
    #[allow(dead_code)]
    pub(crate) fn default_mihomo_config_display(&self) -> String {
        display_path(&self.program_dir, &self.default_mihomo_config_file)
    }

    /// 设置页展示内核实际加载的运行时配置路径。
    pub(crate) fn runtime_mihomo_config_display(&self) -> String {
        display_path(&self.program_dir, &self.runtime_mihomo_config_file)
    }

    /// 设置页展示 Mihomo 的运行数据目录。
    pub(crate) fn mihomo_data_display(&self) -> String {
        display_path(&self.program_dir, &self.mihomo_data_dir)
    }
}

fn display_path(program_dir: &Path, path: &Path) -> String {
    path.strip_prefix(program_dir)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(target_os = "macos")]
fn platform_kernel_dir(program_dir: &Path) -> PathBuf {
    // `.app/Contents/MacOS` 的同级 `Resources` 用于放置签名后的随包资源。
    program_dir
        .parent()
        .unwrap_or(program_dir)
        .join("Resources")
        .join("kernel")
}

#[cfg(target_os = "macos")]
fn platform_geodata_dir(program_dir: &Path) -> PathBuf {
    // Geo 数据与内核一样属于签名后的只读资源，首次启动再复制到用户数据目录。
    program_dir
        .parent()
        .unwrap_or(program_dir)
        .join("Resources")
        .join("geodata")
}

#[cfg(target_os = "linux")]
fn platform_kernel_dir(program_dir: &Path) -> PathBuf {
    // Linux 按便携式/AppImage 布局准备；发行包可在此处扩展 `/usr/lib` 查找。
    program_dir.join("kernel")
}

#[cfg(target_os = "linux")]
fn platform_geodata_dir(program_dir: &Path) -> PathBuf {
    program_dir.join("geodata")
}

/// 返回当前随包 manifest 指定的 Mihomo 可执行文件名。
pub(crate) fn kernel_binary_name() -> &'static str {
    env!("PURE_CLASH_DEFAULT_MIHOMO_BINARY")
}

/// 构造主窗口选项；Windows 和 Linux 使用平台适配的自绘标题栏，macOS 保留原生装饰。
pub(crate) fn main_window_options(bounds: Bounds<gpui::Pixels>) -> WindowOptions {
    let titlebar = if cfg!(target_os = "windows") {
        None
    } else {
        Some(TitlebarOptions {
            title: Some("Pure Clash".into()),
            ..Default::default()
        })
    };

    let options = WindowOptions {
        titlebar,
        app_id: Some("pure-clash".to_owned()),
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(1080.0), px(700.0))),
        ..Default::default()
    };

    #[cfg(target_os = "linux")]
    let options = WindowOptions {
        // GNOME Wayland 不提供服务端标题栏；显式使用客户端装饰后由应用统一提供
        // 拖动区域与窗口按钮。X11 不支持客户端装饰时，GPUI 会回退到服务端装饰。
        window_decorations: Some(WindowDecorations::Client),
        window_background: WindowBackgroundAppearance::Transparent,
        ..options
    };

    options
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_layout_uses_path_joins() {
        let root = PathBuf::from("program-root");
        let paths = AppPaths::portable(&root);
        assert_eq!(paths.config_file, root.join("config").join("app.json"));
        assert_eq!(
            paths.default_mihomo_config_file,
            root.join("config").join("mihomo").join("default.yaml")
        );
        assert_eq!(
            paths.runtime_mihomo_config_file,
            root.join("config").join("mihomo").join("runtime.yaml")
        );
        assert_eq!(paths.data_dir, root.join("data"));
        assert_eq!(paths.mihomo_data_dir, root.join("data").join("mihomo"));
        assert_eq!(
            paths.local_mihomo_config_file,
            root.join("config").join("mihomo").join("local.yaml")
        );
        assert_eq!(
            paths.runtime_mihomo_config_file,
            root.join("config").join("mihomo").join("runtime.yaml")
        );
        assert_eq!(paths.profiles_dir, root.join("config").join("profiles"));
        assert_eq!(paths.geodata_resource_dir, root.join("geodata"));
        assert_eq!(paths.kernel_dir, root.join("kernel"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_uses_client_decorations_for_application_window_controls() {
        let options = main_window_options(Bounds::default());
        assert_eq!(options.window_decorations, Some(WindowDecorations::Client));
        assert_eq!(
            options.window_background,
            WindowBackgroundAppearance::Transparent
        );
        assert_eq!(options.window_min_size, Some(size(px(1080.0), px(700.0))));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_keeps_borderless_custom_titlebar_options() {
        let options = main_window_options(Bounds::default());
        assert!(options.titlebar.is_none());
        assert!(options.window_decorations.is_none());
    }
}
