#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

// 编译期嵌入中文和英文资源，运行时只切换当前 locale，不读取外部翻译文件。
rust_i18n::i18n!("locales", fallback = "zh-CN");

// Windows 发布构建不附带控制台，debug 构建则保留终端以便查看启动错误。
// 日志宏经 #[macro_use] 提供给后续声明的所有模块使用。
#[macro_use]
mod logging;
mod app;
mod assets;
mod config;
mod kernel;
mod mihomo;
mod platform;
mod profile;
mod startup;
mod theme;
mod ui;

use app::{AppShell, PureClash};
use assets::Assets;
use gpui::{App, QuitMode, prelude::*};
use gpui_platform::application;
use startup::StartupMode;

#[cfg(any(target_os = "windows", target_os = "linux"))]
use platform::{SingleInstance, SingleInstanceState};

fn main() {
    // Linux TUN 服务安装器和 systemd 服务都会重新执行当前程序；
    // 必须在单实例锁、配置和 GPUI 初始化前分流，避免 root 进程触碰用户应用状态。
    #[cfg(target_os = "linux")]
    match platform::run_elevated_helper_if_requested() {
        Ok(false) => {}
        Ok(true) => return,
        Err(error) => {
            eprintln!("Linux TUN 提权助手启动失败：{error:#}");
            std::process::exit(1);
        }
    }

    let startup_mode = StartupMode::from_env();

    // 当前用户会话只允许一个 Pure Clash 实例；登录自启的次实例静默退出，
    // 用户主动启动的次实例才通知首实例恢复窗口。
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    let (_single_instance, activation_requests) =
        match SingleInstance::acquire(startup_mode.notify_existing_instance())
            .expect("无法初始化 Pure Clash 单实例控制")
        {
            SingleInstanceState::Primary {
                guard,
                activation_requests,
            } => (guard, activation_requests),
            SingleInstanceState::Secondary => return,
        };

    // 运行日志在单实例判定之后、配置加载之前初始化：启动期的配置、基线与
    // 内核失败都能落盘；次实例不写日志、静默退出，唤起事件由首实例记录。
    if let Ok(paths) = platform::AppPaths::from_current_exe() {
        logging::init(&paths.log_dir);
    }

    // 平台模块统一解析配置和资源路径，避免依赖可能被快捷方式改变的工作目录。
    let loaded_config = config::load_or_create().unwrap_or_else(|error| {
        log_error!("app", "初始化 Pure Clash 配置失败：{error:#}");
        panic!("无法初始化 Pure Clash 配置：{error:#}");
    });
    rust_i18n::set_locale(loaded_config.config.language.code());

    let application = application()
        .with_assets(Assets::new())
        // 窗口关闭只释放窗口资源，应用由托盘“退出”显式结束。
        .with_quit_mode(QuitMode::Explicit);

    // macOS 关闭全部窗口后可从 Dock 重新打开；其他平台由托盘或第二实例触发。
    #[cfg(target_os = "macos")]
    application.on_reopen(AppShell::open_global);

    application.run(move |cx: &mut App| {
        // 输入组件的动作键（退格、粘贴等）绑定到 TextInput 上下文。
        ui::bind_input_keys(cx);

        // 业务实体与进程同生命周期；窗口关闭和重建都复用它，避免重复启动内核及轮询。
        let runtime = cx.new(|cx| PureClash::new(loaded_config, startup_mode, cx));
        let shell = AppShell::install(runtime, cx);
        shell.update(cx, |shell, cx| {
            shell.start(startup_mode.show_initial_window(), cx)
        });

        #[cfg(any(target_os = "windows", target_os = "linux"))]
        install_single_instance_listener(activation_requests, cx);
    });
}

/// 监听后续进程的启动请求；当前处于零窗口状态时会重新创建主窗口。
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn install_single_instance_listener(
    activation_requests: async_channel::Receiver<()>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        while activation_requests.recv().await.is_ok() {
            log_info!("app", "收到第二实例唤起请求，恢复主窗口");
            // AppShell 由 GPUI 全局状态持有，零窗口期间也能接收第二实例的唤起请求。
            cx.update(AppShell::open_global);
        }
    })
    .detach();
}
