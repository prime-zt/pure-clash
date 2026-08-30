use anyhow::{Context, Result};
use async_channel::{Receiver, Sender, unbounded};
use rust_i18n::t;
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};

use crate::platform::tray::TrayAction;

const TRAY_ID: &str = "pure-clash-main";
const APP_ICON_RESOURCE_ID: u16 = 1;
const MENU_OPEN_ID: &str = "pure-clash-menu-open";
const MENU_QUIT_ID: &str = "pure-clash-menu-quit";

/// Windows 系统托盘资源；实例存活期间托盘图标保持可见，释放时自动移除。
pub(crate) struct SystemTray {
    icon: TrayIcon,
    open_item: MenuItem,
    quit_item: MenuItem,
}

impl SystemTray {
    /// 创建托盘图标和右键菜单，并返回操作接收端；接收端必须在 GPUI 主线程持续消费。
    pub(crate) fn new() -> Result<(Self, Receiver<TrayAction>)> {
        let (action_sender, action_receiver) = unbounded();
        install_tray_icon_event_handler(action_sender.clone());
        install_menu_event_handler(action_sender);

        // 资源 ID 1 与 build.rs、GPUI Windows 窗口图标使用同一份多分辨率 ICO。
        let icon = Icon::from_resource(APP_ICON_RESOURCE_ID, Some((32, 32)))
            .context("无法从 Pure Clash 可执行文件读取托盘图标")?;
        let open_item = MenuItem::with_id(MENU_OPEN_ID, t!("tray.menu_open"), true, None);
        let quit_item = MenuItem::with_id(MENU_QUIT_ID, t!("tray.menu_quit"), true, None);
        let menu = Menu::new();
        menu.append_items(&[&open_item, &quit_item])
            .context("无法创建 Pure Clash 托盘右键菜单")?;

        let icon = TrayIconBuilder::new()
            .with_id(TRAY_ID)
            .with_icon(icon)
            .with_tooltip("Pure Clash")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .context("无法创建 Pure Clash 系统托盘图标")?;

        Ok((
            Self {
                icon,
                open_item,
                quit_item,
            },
            action_receiver,
        ))
    }

    /// 更新 Windows 托盘悬浮提示；Shell 最多保留 127 个 UTF-16 字符。
    pub(crate) fn set_tooltip(&self, tooltip: &str) -> Result<()> {
        self.icon
            .set_tooltip(Some(tooltip))
            .context("无法更新 Pure Clash 托盘状态")
    }

    /// 语言切换后同步更新右键菜单文案。
    pub(crate) fn set_menu_texts(&self, open_text: &str, quit_text: &str) {
        self.open_item.set_text(open_text);
        self.quit_item.set_text(quit_text);
    }
}

fn install_tray_icon_event_handler(action_sender: Sender<TrayAction>) {
    // tray-icon 的回调来自 Win32 消息处理流程，先写入通道，再由 GPUI 主线程操作窗口。
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if event.id() != TRAY_ID {
            return;
        }

        if matches!(
            event,
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
        ) {
            // 通道无界且操作频率受人工点击限制，不会堆积。
            let _ = action_sender.try_send(TrayAction::OpenMainWindow);
        }
    }));
}

fn install_menu_event_handler(action_sender: Sender<TrayAction>) {
    // muda 的菜单事件同样是全局回调，按菜单项 ID 分发到同一个操作通道。
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let action = match event.id.as_ref() {
            MENU_OPEN_ID => Some(TrayAction::OpenMainWindow),
            MENU_QUIT_ID => Some(TrayAction::Quit),
            _ => None,
        };
        if let Some(action) = action {
            let _ = action_sender.try_send(action);
        }
    }));
}
