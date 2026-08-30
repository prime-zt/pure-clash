use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, IntoElement, Render, SharedString, Styled,
    Window, div, prelude::*, px, rgb,
};
use rust_i18n::t;

use crate::{
    assets::{
        ICON_EYE, ICON_EYE_OFF, ICON_FILE_CODE, ICON_GIT_BRANCH, ICON_INFO, ICON_LAYOUT,
        ICON_MESSAGE, ICON_SETTINGS,
    },
    config::{AppConfig, Language, LoadedConfig, ProfileMeta},
    kernel::is_available,
    mihomo::{
        MihomoProcess,
        config::NodeEndpoint,
        config::ensure_baseline,
        config::merge_runtime,
        config::read_node_endpoints,
        config::read_proxy_group_order,
        config::save_baseline,
        config::validate_kernel_config,
        config::write_runtime,
        controller::{
            ConnectionItem, ConnectionsSnapshot, Controller, GroupSnapshot, Mode, NodeSnapshot,
        },
    },
    platform::{
        AppPaths, SystemProxySnapshot, SystemTray, capture_system_proxy, restore_system_proxy,
        set_system_proxy,
    },
    profile,
    theme::{FontWeightExt, Palette},
    ui::TextInput,
};

/// Linux 关闭按钮在应用内隐藏窗口；Windows 由原生 WM_CLOSE 拦截处理。
#[cfg(target_os = "linux")]
use crate::platform::hide_main_window;

impl PureClash {
    /// 应用当前版本；与 Cargo 包版本保持一致，供更新检查比较。
    pub(crate) const CURRENT_VERSION: &'static str = env!("CARGO_PKG_VERSION");
}

/// 超过该节点数的代理分组默认折叠，避免大订阅进入代理页时一次性布局全部节点。
const PROXY_AUTO_COLLAPSE_NODES: usize = 30;

/// 未手动操作时允许自动展开的节点总量上界；超出后其余分组默认折叠。
const PROXY_AUTO_EXPAND_NODE_BUDGET: usize = 120;

/// 代理节点卡片一行展示的列数。
const PROXY_NODE_COLUMNS: usize = 3;

/// 展开分组单页渲染的节点数上限；大分组通过“显示更多”分页浏览，
/// 让单次布局量有硬上界，同时滚动只保留页面一层。
const PROXY_NODE_PAGE_SIZE: usize = 30;

mod about;
mod connections;
mod frame;
mod header;
mod overview;
mod profiles;
mod proxies;
mod settings;
mod sidebar;

#[cfg(target_os = "linux")]
use frame::linux_client_side_decorations;
use frame::render_titlebar;
use header::render_page;
use proxies::group_auto_expanded;
use sidebar::render_sidebar;

/// 应用侧边栏中的基础页面，覆盖日常代理管理的最小闭环。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Overview,
    Proxies,
    Connections,
    Profiles,
    Settings,
    About,
}

impl Page {
    fn all() -> [Self; 5] {
        [
            Self::Overview,
            Self::Proxies,
            Self::Connections,
            Self::Profiles,
            Self::Settings,
        ]
    }

    fn label(self) -> SharedString {
        match self {
            Self::Overview => tr("page.overview"),
            Self::Proxies => tr("page.proxies"),
            Self::Connections => tr("page.connections"),
            Self::Profiles => tr("page.profiles"),
            Self::Settings => tr("page.settings"),
            Self::About => tr("page.about"),
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Overview => ICON_LAYOUT,
            Self::Proxies => ICON_GIT_BRANCH,
            Self::Connections => ICON_MESSAGE,
            Self::Profiles => ICON_FILE_CODE,
            Self::Settings => ICON_SETTINGS,
            Self::About => ICON_INFO,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProxyMode {
    Rule,
    Global,
    Direct,
}

impl ProxyMode {
    fn label(self) -> SharedString {
        match self {
            Self::Rule => tr("mode.rule"),
            Self::Global => tr("mode.global"),
            Self::Direct => tr("mode.direct"),
        }
    }

    fn detail(self) -> SharedString {
        match self {
            Self::Rule => tr("mode.rule_detail"),
            Self::Global => tr("mode.global_detail"),
            Self::Direct => tr("mode.direct_detail"),
        }
    }

    fn from_controller(mode: Mode) -> Self {
        match mode {
            Mode::Rule => Self::Rule,
            Mode::Global => Self::Global,
            Mode::Direct => Self::Direct,
        }
    }

    fn to_controller(self) -> Mode {
        match self {
            Self::Rule => Mode::Rule,
            Self::Global => Mode::Global,
            Self::Direct => Mode::Direct,
        }
    }
}

/// 内核运行状态；Starting 表示进程已拉起，正在等待 controller 就绪。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CoreState {
    Stopped,
    Starting,
    Running,
}

/// 连接页单次渲染的最大行数；超出部分提示条数，避免超大订阅拖慢布局。
const CONNECTIONS_RENDER_LIMIT: usize = 200;
/// 实时连接与流量的轮询间隔；与内核 dashboard 的默认推送节奏一致。
const CONNECTIONS_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

pub(crate) struct PureClash {
    page: Page,
    /// 启动时读取并在应用内持续维护的主配置对象。
    config: AppConfig,
    /// 当前平台的程序、配置、数据和内核目录集合。
    paths: AppPaths,
    /// 仅表示配置所选版本的内核文件存在，不代表 Mihomo 进程已经运行。
    kernel_available: bool,
    /// 当前由应用持有的真实 Mihomo 子进程，释放时会自动停止。
    mihomo_process: Option<MihomoProcess>,
    core_state: CoreState,
    /// 最近一次内核启停错误，直接展示在运行状态卡片中。
    mihomo_error: Option<String>,
    system_proxy: bool,
    tun_enabled: bool,
    /// 运行模式；与 controller 同步，未运行时保持上次已知值。
    mode: ProxyMode,
    /// controller 返回的真实策略组快照；内核未运行时为空。
    groups: Vec<GroupSnapshot>,
    /// controller 轮询到的活跃连接（按建立时间序）；内核停止时清空。
    connections: Vec<ConnectionItem>,
    /// 实时上传/下载速度与会话累计字节数；由相邻连接快照差分得出。
    traffic_down_speed: u64,
    traffic_up_speed: u64,
    download_total: u64,
    upload_total: u64,
    /// 延迟测试进行中的节点名集合；测速按钮据此显示忙态。
    delay_testing: std::collections::HashSet<String>,
    /// 手动测速覆盖的节点延迟（毫秒）；优先于 /proxies 自带的历史值。
    node_delays: std::collections::HashMap<String, u64>,
    /// 手动测速失败的节点（超时/拒绝）；下次成功测速或停内核时清除。
    node_delay_failures: std::collections::HashSet<String>,
    /// 订阅/导入配置的元数据；内容按 id 存放在 profiles 目录。
    profiles: Vec<ProfileMeta>,
    /// 当前激活的配置 id；None 表示使用内置默认配置。
    active_profile: Option<String>,
    /// 本地基线；controller 地址与 secret 的唯一来源。
    baseline: Option<crate::mihomo::config::LocalBaseline>,
    /// 配置页内联表单。
    profile_form_open: bool,
    profile_form_name: Entity<TextInput>,
    profile_form_url: Entity<TextInput>,
    /// 配置页后台任务忙态提示；非空时禁用相关操作。
    profile_busy: Option<String>,
    profile_error: Option<String>,
    /// 系统代理 / TUN 的操作失败提示，展示在设置页与概览页。
    integration_error: Option<String>,
    /// 代理页数据拉取中。
    proxies_loading: bool,
    /// 分组手动折叠状态（组名 → 是否展开）；未记录的组按节点数自动决定。
    group_expanded: std::collections::HashMap<String, bool>,
    /// 分组已翻页渲染的节点数（组名 → 数量）；未记录的组只渲染首页。
    group_page: std::collections::HashMap<String, usize>,
    /// 节点接入信息（协议/服务器/端口），来自 runtime.yaml，按节点名索引。
    node_endpoints: std::collections::HashMap<String, NodeEndpoint>,
    /// 概览页服务器地址是否明文展示；默认打码，眼睛图标切换。
    server_visible: bool,
    /// 关于页检查更新是否进行中。
    update_checking: bool,
    /// 检查发现新版本时置位；界面用它强调更新提示。
    update_available: bool,
    /// 关于页检查更新的结果文案；None 表示尚未检查。
    update_status: Option<SharedString>,
    /// 平台托盘句柄必须与应用状态同生命周期持有，释放后系统会移除图标。
    system_tray: Option<SystemTray>,
}

impl PureClash {
    pub(crate) fn new(loaded_config: LoadedConfig, cx: &mut Context<Self>) -> Self {
        let LoadedConfig { mut config, paths } = loaded_config;
        // 启动时以 AppConfig 指定的版本确定当前内核路径和可用状态。
        let kernel_available = is_available(&paths, &config.mihomo_version);

        // 本地基线提供 controller 地址与 secret；失败时仅禁用在线功能。
        let baseline = match ensure_baseline(&paths) {
            Ok(baseline) => Some(baseline),
            Err(error) => {
                eprintln!("初始化本地基线失败：{error:#}");
                None
            }
        };

        // 校验持久化的激活配置：内容缺失时回退到内置默认配置。
        if let Some(active_id) = config.active_profile.clone()
            && !profile::profile_yaml_path(&paths, &active_id).is_file()
        {
            config.active_profile = None;
        }
        let active_profile = config.active_profile.clone();
        if let Err(error) = profile::sync_runtime_file(
            &paths,
            baseline.as_ref(),
            &config.mihomo_version,
            active_profile.as_deref(),
        ) {
            eprintln!("同步运行时配置失败：{error:#}");
        }

        // 上次异常退出可能遗留系统代理托管，启动时按记录恢复用户原有设置。
        if let Some(snapshot) = load_system_proxy_state(&paths) {
            match restore_system_proxy(&snapshot) {
                Ok(()) => {
                    clear_system_proxy_state(&paths);
                    eprintln!("检测到上次托管的系统代理，已恢复用户原有设置");
                }
                Err(error) => eprintln!("恢复系统代理设置失败：{error:#}"),
            }
        }

        let profile_form_name = cx.new(|cx| TextInput::new(t!("profiles.name_placeholder"), cx));
        let profile_form_url = cx.new(|cx| TextInput::new(t!("profiles.url_placeholder"), cx));
        let profiles = config.profiles.clone();

        let mut app = Self {
            page: Page::Overview,
            config,
            paths,
            kernel_available,
            mihomo_process: None,
            core_state: CoreState::Stopped,
            mihomo_error: None,
            system_proxy: false,
            // TUN 是基线里的持久开关，启动时如实恢复；系统代理始终默认关闭。
            tun_enabled: baseline
                .as_ref()
                .is_some_and(|baseline| baseline.tun_enable),
            mode: ProxyMode::Rule,
            groups: Vec::new(),
            connections: Vec::new(),
            traffic_down_speed: 0,
            traffic_up_speed: 0,
            download_total: 0,
            upload_total: 0,
            delay_testing: std::collections::HashSet::new(),
            node_delays: std::collections::HashMap::new(),
            node_delay_failures: std::collections::HashSet::new(),
            profiles,
            active_profile,
            baseline,
            profile_form_open: false,
            profile_form_name,
            profile_form_url,
            profile_busy: None,
            profile_error: None,
            integration_error: None,
            proxies_loading: false,
            group_expanded: std::collections::HashMap::new(),
            group_page: std::collections::HashMap::new(),
            node_endpoints: std::collections::HashMap::new(),
            server_visible: false,
            update_checking: false,
            update_available: false,
            update_status: None,
            system_tray: None,
        };
        // 开机即按上次记录的激活配置启动内核；失败时只在界面提示，不阻断打开。
        app.start_core(cx);
        app.spawn_connection_poll(cx);
        app
    }

    /// 内核是否处于运行状态（controller 已就绪）。
    fn mihomo_running(&self) -> bool {
        matches!(self.core_state, CoreState::Running)
    }

    /// 内核是否允许接受启停操作。
    fn core_operable(&self) -> bool {
        self.core_state != CoreState::Starting
    }

    /// 主窗口创建后挂载平台托盘，并立即同步当前运行状态。
    pub(crate) fn attach_system_tray(&mut self, system_tray: SystemTray) {
        self.system_tray = Some(system_tray);
        self.refresh_tray_texts();
    }

    /// 用当前语言同步托盘提示和右键菜单，并展示内核、系统代理与 TUN 三项状态。
    fn refresh_tray_texts(&self) {
        let core = match self.core_state {
            CoreState::Running => t!("tray.running"),
            CoreState::Starting => t!("app.core_starting"),
            CoreState::Stopped => t!("tray.stopped"),
        };
        let system_proxy = if self.system_proxy {
            t!("tray.on")
        } else {
            t!("tray.off")
        };
        let tun = if self.tun_enabled {
            t!("tray.on")
        } else {
            t!("tray.off")
        };
        let tooltip = t!(
            "tray.tooltip",
            core = core,
            system_proxy = system_proxy,
            tun = tun
        );

        if let Some(system_tray) = &self.system_tray {
            if let Err(error) = system_tray.set_tooltip(&tooltip) {
                eprintln!("更新托盘状态失败：{error:#}");
            }
            system_tray.set_menu_texts(&t!("tray.menu_open"), &t!("tray.menu_quit"));
        }
    }

    fn palette(&self) -> Palette {
        if self.config.theme.is_dark() {
            Palette::dark()
        } else {
            Palette::light()
        }
    }

    fn select_page(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        // 进入关于页时自动检查一次更新；本会话已有结果或正在检查则跳过，
        // 手动按钮随时可以重新检查。
        if page == Page::About && self.update_status.is_none() && !self.update_checking {
            self.check_for_updates(cx);
        }
        cx.notify();
    }

    /// 关于页检查更新：查询仓库最新发布版本并与当前版本比较。
    ///
    /// 进入关于页自动触发一次，也可通过按钮手动重查；不做自动安装，
    /// 发现新版本时仅在关于页提示并引导用户前往仓库。
    fn check_for_updates(&mut self, cx: &mut Context<Self>) {
        if self.update_checking {
            return;
        }
        self.update_checking = true;
        self.update_available = false;
        self.update_status = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::app::about::latest_release_version() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.update_checking = false;
                this.update_status = Some(match result {
                    Ok(Some(latest)) => {
                        if crate::app::about::is_newer_version(&latest, Self::CURRENT_VERSION) {
                            this.update_available = true;
                            SharedString::from(
                                t!("about.new_version", version = latest).into_owned(),
                            )
                        } else {
                            tr("about.up_to_date")
                        }
                    }
                    Ok(None) => tr("about.no_release"),
                    Err(error) => SharedString::from(
                        t!(
                            "about.check_failed",
                            error = concise_error(&format!("{error:#}"), 160)
                        )
                        .into_owned(),
                    ),
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// 标题栏和设置页共用同一主题状态，确保两个入口始终同步。
    fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        let previous = self.config.theme;
        self.config.theme = previous.toggled();

        if let Err(error) = self.config.save(&self.paths.config_file) {
            // 写入失败时恢复旧值，避免界面状态与磁盘配置不一致。
            self.config.theme = previous;
            eprintln!("保存主题配置失败：{error:#}");
            return;
        }

        cx.notify();
    }

    /// 语言切换立即更新全局 locale 和主配置，保存失败时完整回滚。
    fn toggle_language(&mut self, cx: &mut Context<Self>) {
        self.set_language(self.config.language.toggled(), cx);
    }

    fn set_language(&mut self, language: Language, cx: &mut Context<Self>) {
        if language == self.config.language {
            return;
        }

        let previous = self.config.language;
        self.config.language = language;
        rust_i18n::set_locale(language.code());

        if let Err(error) = self.config.save(&self.paths.config_file) {
            self.config.language = previous;
            rust_i18n::set_locale(previous.code());
            eprintln!("保存语言配置失败：{error:#}");
            return;
        }

        self.refresh_tray_texts();
        // 输入框占位文案在实体创建时固定，语言切换后按新 locale 重建。
        self.profile_form_name = cx.new(|cx| TextInput::new(t!("profiles.name_placeholder"), cx));
        self.profile_form_url = cx.new(|cx| TextInput::new(t!("profiles.url_placeholder"), cx));
        cx.notify();
    }

    fn toggle_core(&mut self, cx: &mut Context<Self>) {
        if self.mihomo_running() {
            // 系统代理指向本机内核端口，停内核前先恢复用户原有代理设置。
            self.disable_system_proxy();
            self.stop_core();
            // TUN 由内核承载，内核停止即失效；同步回退避免界面与下次启动
            // 仍显示/启用已停止的 TUN。
            self.revert_tun(cx);
        } else {
            self.start_core(cx);
        }
        self.refresh_tray_texts();
        cx.notify();
    }

    /// 启动内核：主线程完成 `-t` 校验与进程拉起，后台等待 controller 就绪。
    fn start_core(&mut self, cx: &mut Context<Self>) {
        if !self.core_operable() || self.mihomo_process.is_some() {
            return;
        }
        match MihomoProcess::start(
            &self.paths,
            &self.config.mihomo_version,
            &self.paths.runtime_mihomo_config_file,
            // TUN 需要系统网络权限；UAC/polkit 弹窗即用户的显式授权。
            self.tun_enabled,
        ) {
            Ok(process) => {
                self.mihomo_process = Some(process);
                self.core_state = CoreState::Starting;
                self.mihomo_error = None;
                self.spawn_readiness_probe(cx);
            }
            Err(error) => {
                self.mihomo_process = None;
                self.core_state = CoreState::Stopped;
                // 提权启动被拒绝（用户取消 UAC/polkit）或校验失败时，回退关闭 TUN
                // 并以普通权限重新拉起内核，保证用户始终有可用代理。
                if self.tun_enabled {
                    let fallback_message = tun_reverted_error(&error);
                    self.revert_tun(cx);
                    self.integration_error = Some(fallback_message);
                    self.start_core(cx);
                    self.refresh_tray_texts();
                    cx.notify();
                    return;
                }
                let detail = concise_error(&format!("{error:#}"), 240);
                // 原始校验错误只展示给当前用户，不写入日志，避免泄露配置中的敏感内容。
                eprintln!("启动 Mihomo 失败，详情已显示在应用界面");
                self.mihomo_error = Some(t!("app.core_start_failed", error = detail).into_owned());
            }
        }
    }

    /// 内核就绪后核对 TUN 是否真实生效。
    ///
    /// 内核在缺少系统网络权限或平台 TUN 能力时会静默降级继续运行，界面不能
    /// 假装 TUN 已开启：未生效时自动回退关闭并重启内核，明确提示原因。
    fn verify_tun_effective(&mut self, cx: &mut Context<Self>) {
        if !self.tun_enabled {
            return;
        }
        let Some(controller) = self.controller() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let background = cx.background_executor().clone();
            let timer = background.clone();
            let check = background.spawn(async move {
                // controller 监听可能早于 TUN 初始化完成，短暂轮询内核的真实状态。
                for _ in 0..10 {
                    if let Ok(config) = controller.configs()
                        && config.tun_enabled
                    {
                        return Ok(());
                    }
                    timer.timer(std::time::Duration::from_millis(300)).await;
                }
                anyhow::bail!("Mihomo controller 未确认 TUN 已生效")
            });
            let result = check.await;
            let _ = this.update(cx, |this, cx| {
                let Err(error) = result else {
                    return;
                };
                if !this.tun_enabled {
                    return;
                }
                // sync_runtime 会写入关闭 TUN 的配置并重启内核恢复代理。
                this.revert_tun(cx);
                // 挂在 integration_error 上：内核重启成功的就绪探针会清掉 mihomo_error。
                this.integration_error = Some(tun_reverted_error(&error));
                this.refresh_tray_texts();
                cx.notify();
            });
        })
        .detach();
    }

    /// 后台轮询 controller `/version`；就绪后拉取运行模式与代理组。
    fn spawn_readiness_probe(&mut self, cx: &mut Context<Self>) {
        let Some(controller) = self.controller() else {
            self.core_state = CoreState::Running;
            return;
        };
        cx.spawn(async move |this, cx| {
            let background = cx.background_executor().clone();
            let probe = {
                let timer = background.clone();
                background.spawn(async move {
                    for _ in 0..50 {
                        if controller.version().is_ok() {
                            return Ok(());
                        }
                        timer.timer(std::time::Duration::from_millis(100)).await;
                    }
                    Err(anyhow::anyhow!("等待 Mihomo controller 超时"))
                })
            };
            let result = probe.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(()) => {
                    this.core_state = CoreState::Running;
                    this.mihomo_error = None;
                    this.refresh_tray_texts();
                    this.fetch_runtime_state(cx);
                    this.verify_tun_effective(cx);
                }
                Err(error) => {
                    // 内核未能就绪：先恢复代理设置再回收进程，避免流量断在死端口上。
                    this.disable_system_proxy();
                    this.stop_core();
                    // TUN 开启时最常见的失败原因是缺少系统授权或平台 TUN 能力，
                    // 自动回退到无 TUN 的配置并重新拉起内核，保证用户有可用代理。
                    if this.tun_enabled {
                        // sync_runtime 会写入关闭 TUN 的配置并重启内核恢复代理。
                        this.revert_tun(cx);
                        this.start_core(cx);
                    }
                    this.integration_error = Some(
                        t!(
                            "app.core_start_failed",
                            error = concise_error(&format!("{error:#}"), 160)
                        )
                        .into_owned(),
                    );
                    this.refresh_tray_texts();
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// controller 客户端；基线缺失时在线功能不可用。
    fn controller(&self) -> Option<Controller> {
        self.baseline.as_ref().map(Controller::new)
    }

    /// 后台拉取运行模式与代理组；内核未运行时忽略。
    fn fetch_runtime_state(&mut self, cx: &mut Context<Self>) {
        let Some(controller) = self.controller() else {
            return;
        };
        let runtime_config = self.paths.runtime_mihomo_config_file.clone();
        self.proxies_loading = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let snapshot = cx.background_executor().spawn(async move {
                let config = controller.configs()?;
                let mut proxies = controller.proxies(config.mode)?;
                // controller 返回无序映射，分组按订阅定义顺序展示。
                order_groups(
                    &mut proxies.groups,
                    &read_proxy_group_order(&runtime_config),
                );
                // 节点接入信息（协议/服务器/端口）来自 runtime.yaml，供概览页展示。
                let endpoints = read_node_endpoints(&runtime_config);
                Ok::<_, anyhow::Error>((config, proxies, endpoints))
            });
            let result = snapshot.await;
            let _ = this.update(cx, |this, cx| {
                this.proxies_loading = false;
                match result {
                    Ok((config, proxies, endpoints)) => {
                        this.mode = ProxyMode::from_controller(config.mode);
                        this.groups = proxies.groups;
                        this.node_endpoints = endpoints;
                    }
                    Err(error) => {
                        eprintln!("拉取 controller 状态失败：{error:#}");
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 实时连接与流量轮询：内核运行期间每秒拉取一次 /connections 快照。
    ///
    /// 任务随应用常驻，内核停止时只跳过拉取，无需在启停路径上管理生命周期；
    /// 应用实体释放后循环自动退出。网速由相邻快照的累计字节数差分得出。
    fn spawn_connection_poll(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(CONNECTIONS_POLL_INTERVAL)
                    .await;
                let controller = this
                    .update(cx, |this, _| {
                        this.mihomo_running().then(|| this.controller()).flatten()
                    })
                    .ok()
                    .flatten();
                let Some(controller) = controller else {
                    continue;
                };
                let snapshot = cx
                    .background_executor()
                    .spawn(async move { controller.connections() })
                    .await;
                let applied = this.update(cx, |this, cx| {
                    // 快照返回时内核可能刚被停止；保持清空状态即可。
                    if this.mihomo_running() {
                        match snapshot {
                            Ok(data) => this.apply_connections(data),
                            // controller 短暂不可达（重启窗口期）静默清零，等下一拍恢复。
                            Err(_) => this.clear_live_traffic(),
                        }
                    }
                    cx.notify();
                });
                if applied.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// 应用连接快照并差分出实时网速；内核重启后累计值变小，差分自然归零。
    fn apply_connections(&mut self, snapshot: ConnectionsSnapshot) {
        self.traffic_down_speed = snapshot.download_total.saturating_sub(self.download_total);
        self.traffic_up_speed = snapshot.upload_total.saturating_sub(self.upload_total);
        self.download_total = snapshot.download_total;
        self.upload_total = snapshot.upload_total;
        self.connections = snapshot.connections;
    }

    /// controller 不可达时清空实时数据；下次成功快照会重新建立基准。
    fn clear_live_traffic(&mut self) {
        self.traffic_down_speed = 0;
        self.traffic_up_speed = 0;
        self.connections.clear();
    }

    /// 关闭单条连接：先乐观移除，失败时提示；下一拍轮询会带回真实状态。
    fn close_connection(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(controller) = self.controller() else {
            return;
        };
        self.connections.retain(|connection| connection.id != id);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { controller.close_connection(&id) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.integration_error = Some(concise_error(&format!("{error:#}"), 160));
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 关闭全部连接。
    fn close_all_connections(&mut self, cx: &mut Context<Self>) {
        let Some(controller) = self.controller() else {
            return;
        };
        self.connections.clear();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { controller.close_all_connections() })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.integration_error = Some(concise_error(&format!("{error:#}"), 160));
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 节点当前展示的延迟：手动测速结果优先，其次取 /proxies 自带的历史值。
    /// 返回 `Some(None)` 表示最近一次手动测速失败（超时/拒绝）。
    fn node_delay(&self, node: &NodeSnapshot) -> Option<Option<u64>> {
        if self.delay_testing.contains(&node.name) {
            return None;
        }
        if self.node_delay_failures.contains(&node.name) {
            return Some(None);
        }
        Some(Some(
            self.node_delays.get(&node.name).copied().or(node.delay)?,
        ))
    }

    /// 对策略组内全部节点测速：结果覆盖历史缓存，未返回的节点按失败处理。
    fn test_group_delay(&mut self, name: String, cx: &mut Context<Self>) {
        let group_key = format!("group:{name}");
        if !self.mihomo_running() || self.delay_testing.contains(&group_key) {
            return;
        }
        let Some(group) = self.groups.iter().find(|group| group.name == name) else {
            return;
        };
        let nodes: Vec<String> = group.nodes.iter().map(|node| node.name.clone()).collect();
        if nodes.is_empty() {
            return;
        }
        let Some(controller) = self.controller() else {
            return;
        };
        self.delay_testing.insert(group_key.clone());
        for node in &nodes {
            self.delay_testing.insert(node.clone());
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { controller.group_delay(&name) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.delay_testing.remove(&group_key);
                for node in &nodes {
                    this.delay_testing.remove(node);
                }
                match result {
                    Ok(delays) => {
                        for node in &nodes {
                            match delays.get(node) {
                                Some(delay) => {
                                    this.node_delays.insert(node.clone(), *delay);
                                    this.node_delay_failures.remove(node);
                                }
                                // 本轮未返回即失败；失败状态让节点显示超时而非旧值。
                                None => {
                                    this.node_delays.remove(node);
                                    this.node_delay_failures.insert(node.clone());
                                }
                            }
                        }
                    }
                    Err(error) => {
                        this.integration_error = Some(concise_error(&format!("{error:#}"), 160));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 对单个节点测速。
    fn test_node_delay(&mut self, name: String, cx: &mut Context<Self>) {
        if !self.mihomo_running() || self.delay_testing.contains(&name) {
            return;
        }
        let Some(controller) = self.controller() else {
            return;
        };
        self.delay_testing.insert(name.clone());
        let node_name = name.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { controller.proxy_delay(&name) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.delay_testing.remove(&node_name);
                match result {
                    Ok(delay) => {
                        this.node_delays.insert(node_name.clone(), delay);
                        this.node_delay_failures.remove(&node_name);
                    }
                    Err(_) => {
                        this.node_delays.remove(&node_name);
                        this.node_delay_failures.insert(node_name);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 分组当前是否展开：用户手动切换优先，未手动操作过的组走自动折叠规则。
    fn group_expanded(&self, group: &GroupSnapshot) -> bool {
        self.group_expanded
            .get(&group.name)
            .copied()
            .unwrap_or_else(|| group_auto_expanded(&self.groups, &group.name))
    }

    /// 点击分组标题切换折叠；手动状态跨刷新保留，切换配置后按组名继续生效。
    fn toggle_group_expanded(&mut self, name: String, cx: &mut Context<Self>) {
        if let Some(group) = self.groups.iter().find(|group| group.name == name) {
            let expanded = !self.group_expanded(group);
            self.group_expanded.insert(name, expanded);
            cx.notify();
        }
    }

    /// 分组当前渲染的节点数：默认首页，点过“显示更多”后按页累加。
    fn group_rendered_count(&self, group: &GroupSnapshot) -> usize {
        self.group_page
            .get(&group.name)
            .copied()
            .map_or(PROXY_NODE_PAGE_SIZE, |count| count.min(group.nodes.len()))
    }

    /// “显示更多”翻页：在当前渲染数上加一页，封顶到节点总数。
    fn show_more_nodes(&mut self, name: String, cx: &mut Context<Self>) {
        if let Some(group) = self.groups.iter().find(|group| group.name == name) {
            let next =
                (self.group_rendered_count(group) + PROXY_NODE_PAGE_SIZE).min(group.nodes.len());
            self.group_page.insert(name, next);
            cx.notify();
        }
    }

    /// 重启内核并重新拉取状态；配置切换后的真实生效路径。
    fn restart_core(&mut self, cx: &mut Context<Self>) {
        self.stop_core();
        self.start_core(cx);
        self.refresh_tray_texts();
        cx.notify();
    }

    /// 回收 Mihomo 子进程；托盘退出和窗口内停止按钮共用同一条路径。
    ///
    /// 注意：配置切换（restart_core）也走这里，系统代理保持托管不中断，
    /// 仅在真实停机路径（手动停止/退出/内核失联）由调用方恢复代理设置。
    pub(crate) fn stop_core(&mut self) {
        let stop_result = self.mihomo_process.take().map(|mut process| process.stop());

        self.core_state = CoreState::Stopped;
        if let Some(Err(error)) = stop_result {
            let detail = concise_error(&format!("{error:#}"), 240);
            eprintln!("停止 Mihomo 失败，详情已显示在应用界面");
            self.mihomo_error = Some(t!("app.core_stop_failed", error = detail).into_owned());
        } else {
            self.mihomo_error = None;
        }

        // 内核停止后在线数据不再有效；TUN 的回退由真实停机路径（toggle_core/
        // 启动失败回退）显式调用 revert_tun，配置切换重启不在此处处理。
        self.groups.clear();
        self.connections.clear();
        self.traffic_down_speed = 0;
        self.traffic_up_speed = 0;
        self.download_total = 0;
        self.upload_total = 0;
        self.delay_testing.clear();
        self.node_delays.clear();
        self.node_delay_failures.clear();
    }

    /// 退出前的完整清理：恢复系统代理并回收内核；托盘退出专用。
    pub(crate) fn shutdown(&mut self) {
        self.disable_system_proxy();
        self.stop_core();
    }

    /// 开关系统代理：启用要求内核运行中；关闭恢复托管前的用户设置。
    fn toggle_system_proxy(&mut self, cx: &mut Context<Self>) {
        if !self.system_proxy {
            if !self.mihomo_running() {
                self.integration_error = Some(t!("app.system_proxy_requires_core").into_owned());
                self.refresh_tray_texts();
                cx.notify();
                return;
            }
            let Some(baseline) = self.baseline.clone() else {
                self.integration_error = Some(t!("app.baseline_missing").into_owned());
                self.refresh_tray_texts();
                cx.notify();
                return;
            };
            let addr = format!("127.0.0.1:{}", baseline.mixed_port);
            match enable_system_proxy(&self.paths, &addr) {
                Ok(()) => {
                    self.integration_error = None;
                    self.system_proxy = true;
                }
                Err(error) => {
                    self.integration_error = Some(system_proxy_error(&error));
                }
            }
        } else {
            self.disable_system_proxy();
        }
        self.refresh_tray_texts();
        cx.notify();
    }

    /// 关闭系统代理并恢复托管前的用户设置；停内核与退出应用共用。
    fn disable_system_proxy(&mut self) {
        if !self.system_proxy {
            return;
        }
        let snapshot = load_system_proxy_state(&self.paths).unwrap_or_default();
        match restore_system_proxy(&snapshot) {
            Ok(()) => {
                clear_system_proxy_state(&self.paths);
                self.system_proxy = false;
                self.integration_error = None;
            }
            Err(error) => {
                self.integration_error = Some(system_proxy_error(&error));
            }
        }
    }

    /// 回退 TUN：状态、基线与运行时配置同步关闭并持久化。
    ///
    /// 供真实停机路径调用（手动停止内核、启动失败、TUN 未生效回退）；
    /// 调用时内核应已停止或尚未拉起，sync_runtime 因此只重写 runtime.yaml，
    /// 不会重启内核。配置切换重启与托盘退出不走这里，TUN 基线保持不变。
    fn revert_tun(&mut self, cx: &mut Context<Self>) {
        self.tun_enabled = false;
        if let Some(mut baseline) = self.baseline.clone() {
            baseline.tun_enable = false;
            if save_baseline(&self.paths, &baseline).is_ok() {
                self.baseline = Some(baseline);
            }
        }
        self.sync_runtime(cx);
    }

    /// 开关 TUN：写入本地基线并重新合并校验；内核运行中自动重启生效。
    ///
    /// TUN 由内核承载，内核未运行时不允许开启（与系统代理约束一致）；
    /// 关闭随时允许，只写基线、下次启动生效。
    fn toggle_tun(&mut self, cx: &mut Context<Self>) {
        let enabled = !self.tun_enabled;
        if enabled && !self.mihomo_running() {
            self.integration_error = Some(t!("app.tun_requires_core").into_owned());
            self.refresh_tray_texts();
            cx.notify();
            return;
        }
        let Some(mut baseline) = self.baseline.clone() else {
            self.integration_error = Some(t!("app.baseline_missing").into_owned());
            self.refresh_tray_texts();
            cx.notify();
            return;
        };
        baseline.tun_enable = enabled;
        if let Err(error) = save_baseline(&self.paths, &baseline) {
            self.integration_error = Some(concise_error(&format!("{error:#}"), 160));
            self.refresh_tray_texts();
            cx.notify();
            return;
        }
        self.baseline = Some(baseline);
        self.tun_enabled = enabled;
        self.integration_error = None;
        // 重新合并 runtime.yaml 后再由 sync_runtime 重启内核；
        // 仅 restart_core 会带着旧配置拉起，TUN 开关不会生效。
        self.sync_runtime(cx);
        self.refresh_tray_texts();
        cx.notify();
    }

    /// 切换运行模式：先经 controller 生效，成功后才更新本地状态。
    fn set_mode(&mut self, mode: ProxyMode, cx: &mut Context<Self>) {
        if !self.mihomo_running() || mode == self.mode {
            return;
        }
        let Some(controller) = self.controller() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let target = mode.to_controller();
            let result = cx
                .background_executor()
                .spawn(async move { controller.patch_mode(target.as_str()) })
                .await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(()) => {
                    this.mode = mode;
                    // GLOBAL 组的可见性随模式变化，重新拉取分组。
                    this.fetch_runtime_state(cx);
                }
                Err(error) => {
                    eprintln!("切换运行模式失败：{error:#}");
                }
            });
        })
        .detach();
    }

    /// 在策略组中选择节点：乐观更新界面，后台提交 controller，失败回拉。
    fn select_node(&mut self, group_index: usize, node_index: usize, cx: &mut Context<Self>) {
        let Some(group) = self.groups.get(group_index) else {
            return;
        };
        let Some(node) = group.nodes.get(node_index) else {
            return;
        };
        if !group.selectable || group.now == node.name || !self.mihomo_running() {
            return;
        }
        let group_name = group.name.clone();
        let node_name = node.name.clone();
        let Some(controller) = self.controller() else {
            return;
        };

        if let Some(group) = self.groups.get_mut(group_index) {
            group.now = node_name.clone();
        }
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { controller.select_proxy(&group_name, &node_name) })
                .await;
            if result.is_err() {
                let _ = this.update(cx, |this, cx| this.fetch_runtime_state(cx));
            }
        })
        .detach();
    }

    /// 保存 profiles 列表到主配置；失败时回滚。
    fn save_profiles(&mut self) {
        let previous = self.config.profiles.clone();
        let previous_active = self.config.active_profile.clone();
        self.config.profiles = self.profiles.clone();
        self.config.active_profile = self.active_profile.clone();
        if let Err(error) = self.config.save(&self.paths.config_file) {
            self.config.profiles = previous;
            self.config.active_profile = previous_active;
            eprintln!("保存配置列表失败：{error:#}");
        }
    }

    /// 打开或关闭添加配置的内联表单。
    fn toggle_profile_form(&mut self, cx: &mut Context<Self>) {
        self.profile_form_open = !self.profile_form_open;
        if !self.profile_form_open {
            self.profile_error = None;
        }
        cx.notify();
    }

    /// 确认添加订阅：后台下载并校验，通过后保存并自动激活。
    fn add_subscription(&mut self, cx: &mut Context<Self>) {
        if self.profile_busy.is_some() {
            return;
        }
        let url = self.profile_form_url.read(cx).content().trim().to_owned();
        if let Err(error) = profile::validate_subscription_url(&url) {
            self.profile_error = Some(error.to_string());
            cx.notify();
            return;
        }
        let name = {
            let input = self.profile_form_name.read(cx).content().trim().to_owned();
            if input.is_empty() {
                profile::default_name_from_url(&url)
                    .unwrap_or_else(|| t!("profiles.default_name").into_owned())
            } else {
                input
            }
        };

        let version = self.config.mihomo_version.clone();
        let paths = self.paths.clone();
        self.profile_busy = Some(t!("profiles.busy_downloading").into_owned());
        self.profile_error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let url_inner = url.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let content = profile::download_subscription(&url_inner)?;
                    let id = profile::new_profile_id();
                    let runtime = profile::validate_and_store(&paths, &version, &id, &content)?;
                    Ok::<_, anyhow::Error>((id, runtime))
                })
                .await;

            let _ = this.update(cx, |this, cx| match result {
                Ok((id, runtime)) => {
                    this.profile_busy = None;
                    this.profile_form_open = false;
                    let mut meta = profile::subscription_meta(name, url);
                    // 文件已按后台任务生成的 id 落盘，元数据必须使用同一 id。
                    meta.id = id;
                    let index = this.profiles.len();
                    this.profiles.push(meta);
                    this.save_profiles();
                    this.activate_profile_by_id(index, runtime, cx);
                }
                Err(error) => {
                    this.profile_busy = None;
                    this.profile_error = Some(concise_error(&format!("{error:#}"), 200));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// 重新下载订阅内容并更新时间戳；激活中的配置同时刷新内核。
    fn update_profile(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.profile_busy.is_some() {
            return;
        }
        let Some(meta) = self.profiles.get(index) else {
            return;
        };
        let Some(url) = meta.url.clone() else {
            return;
        };
        let version = self.config.mihomo_version.clone();
        let paths = self.paths.clone();
        let id = meta.id.clone();
        self.profile_busy = Some(t!("profiles.busy_updating").into_owned());
        self.profile_error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let id_inner = id.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let content = profile::download_subscription(&url)?;
                    let runtime =
                        profile::validate_and_store(&paths, &version, &id_inner, &content)?;
                    Ok::<_, anyhow::Error>(runtime)
                })
                .await;

            let _ = this.update(cx, |this, cx| match result {
                Ok(runtime) => {
                    this.profile_busy = None;
                    if let Some(meta) = this.profiles.get_mut(index) {
                        meta.updated_at = profile::now_secs();
                    }
                    this.save_profiles();
                    if this.active_profile.as_deref() == Some(id.as_str()) {
                        this.apply_runtime(&runtime, cx);
                    }
                    cx.notify();
                }
                Err(error) => {
                    this.profile_busy = None;
                    this.profile_error = Some(concise_error(&format!("{error:#}"), 200));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// 删除配置；激活中的配置会同时回退到内置默认配置。
    fn delete_profile(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.profile_busy.is_some() {
            return;
        }
        let Some(meta) = self.profiles.get(index) else {
            return;
        };
        let was_active = self.active_profile.as_deref() == Some(meta.id.as_str());
        let id = meta.id.clone();
        if let Err(error) = profile::delete_profile_file(&self.paths, &id) {
            self.profile_error = Some(concise_error(&format!("{error:#}"), 200));
            cx.notify();
            return;
        }
        self.profiles.remove(index);
        if was_active {
            self.active_profile = None;
            self.sync_runtime(cx);
        } else {
            cx.notify();
        }
        self.save_profiles();
    }

    /// 点击行激活配置：校验合并产物后写入 runtime.yaml，运行中则重启内核。
    fn activate_profile_clicked(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.profile_busy.is_some() {
            return;
        }
        let Some(meta) = self.profiles.get(index) else {
            return;
        };
        if self.active_profile.as_deref() == Some(meta.id.as_str()) {
            return;
        }
        let id = meta.id.clone();
        let version = self.config.mihomo_version.clone();
        let paths = self.paths.clone();
        self.profile_busy = Some(t!("profiles.busy_activating").into_owned());
        self.profile_error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let id_inner = id.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let content = profile::read_profile(&paths, &id_inner)?;
                    let runtime = merge_runtime(&content, &ensure_baseline(&paths)?)?;
                    validate_kernel_config(&paths, &version, &runtime)?;
                    Ok::<_, anyhow::Error>((content, runtime))
                })
                .await;

            let _ = this.update(cx, |this, cx| match result {
                Ok((_, runtime)) => {
                    this.profile_busy = None;
                    this.active_profile = Some(id);
                    this.save_profiles();
                    this.apply_runtime(&runtime, cx);
                }
                Err(error) => {
                    this.profile_busy = None;
                    this.profile_error = Some(concise_error(&format!("{error:#}"), 200));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// 切回内置默认配置（仅 DIRECT）：与订阅激活同一条校验链路，成功后记录。
    fn activate_default_profile(&mut self, cx: &mut Context<Self>) {
        if self.profile_busy.is_some() || self.active_profile.is_none() {
            return;
        }
        let version = self.config.mihomo_version.clone();
        let paths = self.paths.clone();
        self.profile_busy = Some(t!("profiles.busy_activating").into_owned());
        self.profile_error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let default_path = &paths.default_mihomo_config_file;
                    let content = std::fs::read_to_string(default_path).map_err(|error| {
                        anyhow::anyhow!("无法读取默认配置 {}: {error}", default_path.display())
                    })?;
                    let runtime = merge_runtime(&content, &ensure_baseline(&paths)?)?;
                    validate_kernel_config(&paths, &version, &runtime)?;
                    Ok::<_, anyhow::Error>(runtime)
                })
                .await;

            let _ = this.update(cx, |this, cx| match result {
                Ok(runtime) => {
                    this.profile_busy = None;
                    // active_profile 记为空即代表默认配置，下次启动同样生效。
                    this.active_profile = None;
                    this.save_profiles();
                    this.apply_runtime(&runtime, cx);
                }
                Err(error) => {
                    this.profile_busy = None;
                    this.profile_error = Some(concise_error(&format!("{error:#}"), 200));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// 激活/更新配置后的落地动作：运行中则重启内核让配置真实生效。
    fn apply_runtime(&mut self, runtime_yaml: &str, cx: &mut Context<Self>) {
        // 先落地 runtime.yaml，未运行时下次启动直接使用新配置。
        if let Err(error) = write_runtime(&self.paths, runtime_yaml) {
            let detail = concise_error(&format!("{error:#}"), 160);
            self.profile_error = Some(detail);
        }
        if self.mihomo_process.is_some() {
            self.restart_core(cx);
        } else {
            cx.notify();
        }
    }

    /// 添加订阅后按索引激活：补齐激活态并重启内核。
    fn activate_profile_by_id(
        &mut self,
        index: usize,
        runtime_yaml: String,
        cx: &mut Context<Self>,
    ) {
        let id = self.profiles.get(index).map(|meta| meta.id.clone());
        if let Some(id) = id {
            self.active_profile = Some(id);
            self.save_profiles();
        }
        self.apply_runtime(&runtime_yaml, cx);
    }

    /// 把当前激活态同步到 runtime.yaml；None 回退到内置默认配置。
    fn sync_runtime(&mut self, cx: &mut Context<Self>) {
        let version = self.config.mihomo_version.clone();
        if let Err(error) = profile::sync_runtime_file(
            &self.paths,
            self.baseline.as_ref(),
            &version,
            self.active_profile.as_deref(),
        ) {
            eprintln!("同步运行时配置失败：{error:#}");
        }
        if self.mihomo_process.is_some() {
            self.restart_core(cx);
        }
        cx.notify();
    }
}

impl Render for PureClash {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette();
        let content = div()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .text_color(palette.text)
            .child(render_titlebar(self, palette, window, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(render_sidebar(self, palette, window, cx))
                    .child(render_page(self, palette, window, cx)),
            );

        #[cfg(target_os = "linux")]
        return linux_client_side_decorations(content, palette, window);

        #[cfg(not(target_os = "linux"))]
        content.into_any_element()
    }
}
fn system_proxy_state_file(paths: &AppPaths) -> std::path::PathBuf {
    paths.data_dir.join("system-proxy.json")
}

fn save_system_proxy_state(paths: &AppPaths, snapshot: &SystemProxySnapshot) -> anyhow::Result<()> {
    std::fs::create_dir_all(&paths.data_dir)?;
    let bytes = serde_json::to_vec_pretty(snapshot)?;
    let file = system_proxy_state_file(paths);
    let temporary = paths.data_dir.join("system-proxy.json.tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(temporary, file)?;
    Ok(())
}

fn load_system_proxy_state(paths: &AppPaths) -> Option<SystemProxySnapshot> {
    let content = std::fs::read_to_string(system_proxy_state_file(paths)).ok()?;
    serde_json::from_str(&content).ok()
}

fn clear_system_proxy_state(paths: &AppPaths) {
    let _ = std::fs::remove_file(system_proxy_state_file(paths));
}

/// 启用系统代理，捕获并返回托管前的用户设置快照。
fn enable_system_proxy(paths: &AppPaths, server: &str) -> anyhow::Result<()> {
    let snapshot = capture_system_proxy()?;
    // 状态必须先于系统设置落盘；应用在 set_system_proxy 后崩溃时，下次启动
    // 仍能恢复用户原配置。
    save_system_proxy_state(paths, &snapshot)?;
    if let Err(apply_error) = set_system_proxy(server) {
        match restore_system_proxy(&snapshot) {
            Ok(()) => clear_system_proxy_state(paths),
            Err(restore_error) => {
                return Err(anyhow::anyhow!(
                    "{apply_error:#}；自动恢复原系统代理也失败：{restore_error:#}"
                ));
            }
        }
        return Err(apply_error);
    }
    Ok(())
}

/// 系统代理操作失败的界面文案。
fn system_proxy_error(error: &anyhow::Error) -> String {
    t!(
        "app.system_proxy_failed",
        error = concise_error(&format!("{error:#}"), 160)
    )
    .into_owned()
}

/// TUN 自动回退时保留可操作的失败原因，但限制长度，避免平台命令输出撑坏横幅。
fn tun_reverted_error(error: &anyhow::Error) -> String {
    t!(
        "app.tun_reverted",
        error = concise_error(&format!("{error:#}"), 180)
    )
    .into_owned()
}

/// 系统代理 / TUN 操作失败的提示横幅。
fn order_groups(groups: &mut [GroupSnapshot], order: &[String]) {
    groups.sort_by_key(|group| {
        if group.name == "GLOBAL" {
            (0usize, 0usize)
        } else {
            let position = order
                .iter()
                .position(|name| name == &group.name)
                .unwrap_or(usize::MAX);
            (1usize, position)
        }
    });
}

/// 未手动操作过分组的默认展开规则：单组超过 [`PROXY_AUTO_COLLAPSE_NODES`]
/// 直接折叠；其余组按显示顺序消耗 [`PROXY_AUTO_EXPAND_NODE_BUDGET`] 节点预算，
/// 超出后默认折叠，保证代理页初始渲染量有上界，大订阅不再卡死界面。
fn section_heading(
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    icon_path: &'static str,
    palette: Palette,
) -> AnyElement {
    let title = title.into();
    let detail = detail.into();
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .size_8()
                .rounded_sm()
                .flex()
                .items_center()
                .justify_center()
                .bg(palette.accent_soft)
                .child(icon(icon_path, palette.accent, 16.0)),
        )
        .child(
            div()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .text_color(palette.text)
                        .child(title),
                )
                .child(
                    div()
                        .mt_1()
                        .text_xs()
                        .text_color(palette.muted)
                        .child(detail),
                ),
        )
        .into_any_element()
}

fn info_line(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    positive: bool,
    palette: Palette,
) -> AnyElement {
    let label = label.into();
    let value = value.into();
    div()
        .min_h(px(34.0))
        .flex()
        .items_center()
        .justify_between()
        .border_b_1()
        .border_color(palette.border)
        .child(div().text_xs().text_color(palette.muted).child(label))
        .child(
            div()
                .text_xs()
                .text_color(if positive {
                    palette.success
                } else {
                    palette.text
                })
                .child(value),
        )
        .into_any_element()
}

fn icon(path: &'static str, color: gpui::Rgba, size: f32) -> AnyElement {
    gpui::svg()
        .path(path)
        .size(px(size))
        .flex_none()
        .text_color(color)
        .into_any_element()
}

fn tr(key: &'static str) -> SharedString {
    SharedString::from(t!(key).into_owned())
}

fn concise_error(error: &str, max_chars: usize) -> String {
    let mut concise: String = error.chars().take(max_chars).collect();
    if error.chars().count() > max_chars {
        concise.push('…');
    }
    concise
}

/// 字节数人性化展示；超过 1KB 保留一位小数。
fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// 实时速度展示：字节数 + `/s` 后缀。
fn format_speed(bytes_per_second: u64) -> String {
    format!("{}/s", format_bytes(bytes_per_second))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_formatting_stays_readable_across_scales() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(2516582), "2.4 MB");
        assert_eq!(format_speed(8_912_896), "8.5 MB/s");
    }

    #[test]
    fn locales_cover_core_navigation_and_pages() {
        assert_eq!(t!("page.overview", locale = "zh-CN"), "概览");
        assert_eq!(t!("page.overview", locale = "en-US"), "Overview");
        assert_eq!(
            t!("connections.detail", locale = "zh-CN"),
            "实时数据来自 Mihomo controller"
        );
        assert_eq!(
            t!("connections.detail", locale = "en-US"),
            "Live data from the Mihomo controller"
        );
        assert_eq!(
            t!("app.tun_reverted", locale = "zh-CN", error = "DNS 未接管"),
            "TUN 启用失败，已自动关闭并重启内核：DNS 未接管"
        );
    }

    #[test]
    fn tray_tooltips_cover_locales_and_runtime_states() {
        let chinese = t!(
            "tray.tooltip",
            locale = "zh-CN",
            core = "运行中",
            system_proxy = "开启",
            tun = "关闭"
        );
        let english = t!(
            "tray.tooltip",
            locale = "en-US",
            core = "Stopped",
            system_proxy = "Off",
            tun = "On"
        );

        assert_eq!(
            chinese,
            "Pure Clash\n内核：运行中\n系统代理：开启\nTUN：关闭"
        );
        assert_eq!(
            english,
            "Pure Clash\nCore: Stopped\nSystem proxy: Off\nTUN: On"
        );
        // Windows NOTIFYICONDATAW 的 szTip 只容纳 128 个 UTF-16 单元，需预留结尾 NUL。
        assert!(chinese.encode_utf16().count() <= 127);
        assert!(english.encode_utf16().count() <= 127);

        assert_eq!(t!("tray.menu_open", locale = "zh-CN"), "打开 Pure Clash");
        assert_eq!(t!("tray.menu_quit", locale = "zh-CN"), "退出");
        assert_eq!(t!("tray.menu_open", locale = "en-US"), "Open Pure Clash");
        assert_eq!(t!("tray.menu_quit", locale = "en-US"), "Quit");
    }

    #[test]
    fn concise_error_preserves_utf8_boundaries() {
        assert_eq!(concise_error("启动失败", 2), "启动…");
        assert_eq!(concise_error("short", 10), "short");
    }
}
