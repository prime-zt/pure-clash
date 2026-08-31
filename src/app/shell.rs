//! 应用级生命周期外壳：持有长期业务实体、托盘和可重建的主窗口句柄。
//!
//! GPUI 事件循环使用 `QuitMode::Explicit` 后，最后一个窗口关闭不会结束进程。
//! 本模块负责在零窗口状态下继续持有业务状态，并统一处理托盘、第二实例和
//! macOS Dock 的重新打开请求。

use gpui::{
    App, Context, Entity, Global, Subscription, WindowHandle, WindowId, prelude::*, px, size,
};

#[cfg(any(target_os = "windows", target_os = "linux"))]
use crate::platform::{SystemTray, TrayAction, show_main_window};

use super::PureClash;

/// 存入 GPUI 全局状态的应用外壳句柄，保证零窗口期间外壳和业务实体仍然存活。
struct AppShellGlobal(Entity<AppShell>);

impl Global for AppShellGlobal {}

/// 与进程同生命周期的应用外壳；窗口句柄为空时代表当前处于零窗口状态。
pub(crate) struct AppShell {
    runtime: Entity<PureClash>,
    main_window: Option<WindowHandle<PureClash>>,
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    system_tray: Option<SystemTray>,
    /// 观察业务状态刷新托盘；必须持有订阅，否则离开构造函数后监听会失效。
    _runtime_observer: Subscription,
    /// 窗口销毁后清空句柄；按 WindowId 匹配，避免旧窗口事件影响新窗口。
    _window_closed_observer: Subscription,
}

impl AppShell {
    /// 创建并注册应用外壳。业务实体只初始化一次，窗口重建不会重复启动内核或轮询。
    pub(crate) fn install(runtime: Entity<PureClash>, cx: &mut App) -> Entity<Self> {
        let shell = cx.new(|cx| {
            let runtime_observer = cx.observe(&runtime, |this: &mut AppShell, _, cx| {
                this.refresh_tray_texts(cx);
            });
            let shell = cx.weak_entity();
            let window_closed_observer = cx.on_window_closed(move |cx, window_id| {
                let _ = shell.update(cx, |this, _| this.on_window_closed(window_id));
            });

            Self {
                runtime,
                main_window: None,
                #[cfg(any(target_os = "windows", target_os = "linux"))]
                system_tray: None,
                _runtime_observer: runtime_observer,
                _window_closed_observer: window_closed_observer,
            }
        });
        cx.set_global(AppShellGlobal(shell.clone()));
        shell
    }

    /// 从 GPUI 全局状态处理 macOS Dock reopen 等没有直接持有外壳句柄的入口。
    pub(crate) fn open_global(cx: &mut App) {
        let shell = cx
            .try_global::<AppShellGlobal>()
            .map(|global| global.0.clone());
        if let Some(shell) = shell {
            let _ = shell.update(cx, |this, cx| this.open_or_focus_main_window(cx));
        }
    }

    /// 初始化主窗口和平台托盘；托盘失败不阻止窗口及代理核心启动。
    pub(crate) fn start(&mut self, cx: &mut Context<Self>) {
        self.open_or_focus_main_window(cx);

        #[cfg(any(target_os = "windows", target_os = "linux"))]
        self.install_system_tray(cx);
    }

    /// 已有窗口时恢复并激活；窗口已销毁时把同一个长期业务实体挂到新窗口。
    pub(crate) fn open_or_focus_main_window(&mut self, cx: &mut App) {
        if let Some(window_handle) = self.main_window
            && window_handle
                .update(cx, |_, window, _| {
                    #[cfg(any(target_os = "windows", target_os = "linux"))]
                    show_main_window(window);

                    #[cfg(target_os = "macos")]
                    window.activate_window();
                })
                .is_ok()
        {
            return;
        }

        self.main_window = None;
        let bounds = gpui::Bounds::centered(None, size(px(1080.0), px(700.0)), cx);
        let runtime = self.runtime.clone();
        match cx.open_window(
            crate::platform::main_window_options(bounds),
            move |window, _| {
                window.set_window_title("Pure Clash");
                runtime
            },
        ) {
            Ok(window_handle) => {
                self.main_window = Some(window_handle);
                cx.activate(true);
            }
            Err(error) => eprintln!("无法创建 Pure Clash 主窗口：{error:#}"),
        }
    }

    /// 关闭事件只清除当前窗口句柄；业务实体、托盘和 GPUI 事件循环继续存活。
    fn on_window_closed(&mut self, window_id: WindowId) {
        if self
            .main_window
            .is_some_and(|handle| handle.window_id() == window_id)
        {
            self.main_window = None;
        }
    }

    /// 托盘“退出”的唯一真实退出路径：先恢复系统状态并回收内核，再结束 GPUI。
    fn request_quit(&mut self, cx: &mut App) {
        let _ = self.runtime.update(cx, |runtime, _| runtime.shutdown());
        cx.quit();
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    fn install_system_tray(&mut self, cx: &mut Context<Self>) {
        let (system_tray, actions) = match SystemTray::new() {
            Ok(result) => result,
            Err(error) => {
                eprintln!("初始化系统托盘失败：{error:#}");
                return;
            }
        };
        self.system_tray = Some(system_tray);
        self.refresh_tray_texts(cx);

        cx.spawn(async move |this, cx| {
            while let Ok(action) = actions.recv().await {
                let result = this.update(cx, |this, cx| match action {
                    TrayAction::OpenMainWindow => this.open_or_focus_main_window(cx),
                    TrayAction::Quit => this.request_quit(cx),
                });
                if result.is_err() || action == TrayAction::Quit {
                    break;
                }
            }
        })
        .detach();
    }

    /// 业务实体在有无窗口时都会发出通知；托盘状态因此始终反映真实运行状态。
    fn refresh_tray_texts(&self, cx: &App) {
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        if let Some(system_tray) = &self.system_tray {
            let texts = self.runtime.read(cx).tray_texts();
            if let Err(error) = system_tray.set_tooltip(&texts.tooltip) {
                eprintln!("更新托盘状态失败：{error:#}");
            }
            system_tray.set_menu_texts(&texts.open, &texts.quit);
        }

        #[cfg(target_os = "macos")]
        let _ = cx;
    }
}
