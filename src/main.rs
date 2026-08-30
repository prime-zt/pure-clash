#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

// 编译期嵌入中文和英文资源，运行时只切换当前 locale，不读取外部翻译文件。
rust_i18n::i18n!("locales", fallback = "zh-CN");

// Windows 发布构建不附带控制台，debug 构建则保留终端以便查看启动错误。
mod app;
mod assets;
mod config;
mod kernel;
mod mihomo;
mod platform;
mod profile;
mod theme;
mod ui;

use app::PureClash;
use assets::Assets;
use gpui::{App, Application, Bounds, prelude::*, px, size};

#[cfg(any(target_os = "windows", target_os = "linux"))]
use platform::{SingleInstance, SingleInstanceState};
#[cfg(any(target_os = "windows", target_os = "linux"))]
use platform::{SystemTray, TrayAction, hide_main_window, show_main_window};

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

    // 当前用户会话只允许一个 Pure Clash 实例；后续启动只通知首实例恢复窗口。
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    let (_single_instance, activation_requests) =
        match SingleInstance::acquire().expect("无法初始化 Pure Clash 单实例控制") {
            SingleInstanceState::Primary {
                guard,
                activation_requests,
            } => (guard, activation_requests),
            SingleInstanceState::Secondary => return,
        };

    // 平台模块统一解析配置和资源路径，避免依赖可能被快捷方式改变的工作目录。
    let loaded_config = config::load_or_create().expect("无法初始化 Pure Clash 配置");
    rust_i18n::set_locale(loaded_config.config.language.code());

    Application::new()
        .with_assets(Assets::new())
        .run(move |cx: &mut App| {
            // 输入组件的动作键（退格、粘贴等）绑定到 TextInput 上下文。
            ui::bind_input_keys(cx);

            let bounds = Bounds::centered(None, size(px(1080.0), px(700.0)), cx);
            let main_window = cx
                .open_window(platform::main_window_options(bounds), move |window, cx| {
                    window.set_window_title("Pure Clash");
                    install_close_to_tray(window, cx);
                    cx.new(|cx| PureClash::new(loaded_config, cx))
                })
                .expect("无法创建 Pure Clash 主窗口");

            #[cfg(any(target_os = "windows", target_os = "linux"))]
            install_single_instance_listener(main_window, activation_requests, cx);
            install_system_tray(main_window, cx);
            cx.activate(true);
        });
}

/// 监听后续进程的启动请求，并始终恢复和激活当前主窗口。
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn install_single_instance_listener(
    main_window: gpui::WindowHandle<PureClash>,
    activation_requests: async_channel::Receiver<()>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        while activation_requests.recv().await.is_ok() {
            // 主窗口可能已隐藏到托盘，必须先显示再置前。
            if main_window
                .update(cx, |_, window, _| show_main_window(window))
                .is_err()
            {
                break;
            }
        }
    })
    .detach();
}

/// 拦截窗口关闭：Windows 隐藏到托盘，Linux 最小化到概览；两者都保留托盘退出路径，
/// 阻止 GPUI 销毁窗口，避免内核和托盘意外残留或丢失。
fn install_close_to_tray(window: &mut gpui::Window, cx: &mut App) {
    window.on_window_should_close(cx, |window, _app| {
        hide_main_window(window);
        false
    });
}

/// 安装平台系统托盘，并把托盘回调转交到 GPUI 主线程恢复主窗口。
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn install_system_tray(main_window: gpui::WindowHandle<PureClash>, cx: &mut App) {
    let (system_tray, actions) = match SystemTray::new() {
        Ok(result) => result,
        Err(error) => {
            // 托盘失败不阻止代理客户端主界面启动，错误保留在 debug 控制台中诊断。
            eprintln!("初始化系统托盘失败：{error:#}");
            return;
        }
    };

    if let Err(error) = main_window.update(cx, |app, _, _| app.attach_system_tray(system_tray)) {
        eprintln!("挂载系统托盘失败：{error:#}");
        return;
    }

    cx.spawn(async move |cx| {
        while let Ok(action) = actions.recv().await {
            match action {
                TrayAction::OpenMainWindow => {
                    // 主窗口可能已隐藏到托盘，必须先显示再置前。
                    if main_window
                        .update(cx, |_, window, _| show_main_window(window))
                        .is_err()
                    {
                        break;
                    }
                }
                TrayAction::Quit => {
                    // 真实退出：先恢复系统代理并回收内核，再结束 GPUI 主循环。
                    // 窗口通常已隐藏，这里不依赖窗口可见性。
                    let _ = main_window.update(cx, |app, _, cx| {
                        app.shutdown();
                        cx.quit();
                    });
                    break;
                }
            }
        }
    })
    .detach();
}
