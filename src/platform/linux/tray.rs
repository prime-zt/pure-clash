use std::sync::LazyLock;

use anyhow::{Context, Result};
use async_channel::{Receiver, Sender, unbounded};
use image::{ImageFormat, load_from_memory_with_format};
use ksni::blocking::TrayMethods;
use rust_i18n::t;

use crate::platform::tray::TrayAction;

const TRAY_ID: &str = "pure-clash";
const TRAY_TITLE: &str = "Pure Clash";

/// 随包提交的多尺寸托盘图标；SNI 端按桌面缩放偏好自动选择合适尺寸。
const TRAY_ICON_PNGS: &[(i32, &[u8])] = &[
    (16, include_bytes!("../../../assets/tray/pure-clash-16.png")),
    (24, include_bytes!("../../../assets/tray/pure-clash-24.png")),
    (32, include_bytes!("../../../assets/tray/pure-clash-32.png")),
    (48, include_bytes!("../../../assets/tray/pure-clash-48.png")),
    (64, include_bytes!("../../../assets/tray/pure-clash-64.png")),
];

/// 解码并缓存托盘图标；SNI 要求 ARGB32（网络字节序）像素数据。
/// 内嵌 PNG 属于构建期资产，解码失败等价于构建产物损坏，直接 panic 暴露问题。
static TRAY_ICONS: LazyLock<Vec<ksni::Icon>> = LazyLock::new(|| {
    TRAY_ICON_PNGS
        .iter()
        .map(|(size, png)| decode_tray_icon(*size, png).expect("内嵌托盘图标必须是有效 PNG"))
        .collect()
});

fn decode_tray_icon(size: i32, png: &[u8]) -> Option<ksni::Icon> {
    let image = load_from_memory_with_format(png, ImageFormat::Png).ok()?;
    let mut data = image.into_rgba8().into_vec();
    // SNI 规范要求数据为 ARGB32：把每个 RGBA 像素的字节整体右旋一位。
    for pixel in data.chunks_exact_mut(4) {
        pixel.rotate_right(1);
    }
    Some(ksni::Icon {
        width: size,
        height: size,
        data,
    })
}

/// ksni 的 DBus 服务线程与 GPUI 主线程之间共享的托盘状态；
/// 主线程通过 `Handle::update` 修改字段并触发 SNI 属性重导出。
struct TrayState {
    /// SNI Title；部分桌面（如 GNOME AppIndicator 扩展）会直接显示在图标旁边。
    title: String,
    /// SNI tooltip 描述，按当前语言展示内核、系统代理和 TUN 状态。
    description: String,
    open_label: String,
    quit_label: String,
    /// 菜单与 Activate 事件写回 GPUI 主线程的通道；无界但操作频率受人工点击限制。
    actions: Sender<TrayAction>,
}

impl TrayState {
    fn new(actions: Sender<TrayAction>) -> Self {
        // 初始文案使用默认语言，主窗口创建后 refresh_tray_texts 会立即覆盖。
        Self {
            title: TRAY_TITLE.to_owned(),
            description: t!(
                "tray.tooltip",
                core = t!("tray.stopped"),
                system_proxy = t!("tray.off"),
                tun = t!("tray.off")
            )
            .into_owned(),
            open_label: t!("tray.menu_open").into_owned(),
            quit_label: t!("tray.menu_quit").into_owned(),
            actions,
        }
    }
}

impl ksni::Tray for TrayState {
    fn id(&self) -> String {
        TRAY_ID.to_owned()
    }

    fn title(&self) -> String {
        self.title.clone()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        TRAY_ICONS.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            icon_name: String::new(),
            icon_pixmap: TRAY_ICONS.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        // KDE 等桌面把左键映射为 Activate，与 Windows 单击图标打开主窗口对齐；
        // GNOME AppIndicator 扩展左键固定弹菜单，主入口仍是菜单“打开”项。
        let _ = self.actions.try_send(TrayAction::OpenMainWindow);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            standard_item(self.open_label.clone(), |state| {
                let _ = state.actions.try_send(TrayAction::OpenMainWindow);
            })
            .into(),
            ksni::menu::MenuItem::Separator,
            standard_item(self.quit_label.clone(), |state| {
                let _ = state.actions.try_send(TrayAction::Quit);
            })
            .into(),
        ]
    }
}

fn standard_item(
    label: String,
    activate: impl Fn(&mut TrayState) + Send + 'static,
) -> ksni::menu::StandardItem<TrayState> {
    ksni::menu::StandardItem {
        label,
        activate: Box::new(activate),
        ..Default::default()
    }
}

/// Linux SNI 系统托盘资源；实例存活期间图标保持可见，释放时注销 DBus 服务。
pub(crate) struct SystemTray {
    handle: ksni::blocking::Handle<TrayState>,
}

impl SystemTray {
    /// 创建托盘图标和右键菜单，并返回操作接收端；接收端必须在 GPUI 主线程持续消费。
    pub(crate) fn new() -> Result<(Self, Receiver<TrayAction>)> {
        let (action_sender, action_receiver) = unbounded();
        // SNI watcher（如 GNOME AppIndicator 扩展）可能稍后才出现：
        // 标记 watcher 可用让 ksni 保持服务导出并等待自动注册，而不是直接失败。
        let handle = TrayState::new(action_sender)
            .assume_sni_available(true)
            .spawn()
            .context("无法创建 Pure Clash 系统托盘图标")?;

        Ok((Self { handle }, action_receiver))
    }

    /// 更新托盘状态文本；SNI tooltip 描述跟随语言与运行状态变化。
    pub(crate) fn set_tooltip(&self, tooltip: &str) -> Result<()> {
        // 服务已退出时 update 返回 None，状态会随下一次界面刷新自然丢弃。
        self.handle
            .update(|state| state.description = tooltip.to_owned());
        Ok(())
    }

    /// 语言切换后同步更新右键菜单文案。
    pub(crate) fn set_menu_texts(&self, open_text: &str, quit_text: &str) {
        self.handle.update(|state| {
            state.open_label = open_text.to_owned();
            state.quit_label = quit_text.to_owned();
        });
    }
}

impl Drop for SystemTray {
    fn drop(&mut self) {
        // 真实退出或状态销毁时注销 DBus 名，托盘图标随桌面立即消失。
        self.handle.shutdown().wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_all_bundled_tray_icons() {
        for (size, png) in TRAY_ICON_PNGS {
            let image = load_from_memory_with_format(png, ImageFormat::Png)
                .unwrap_or_else(|error| panic!("内嵌托盘图标 {size}px 应可解码：{error}"));
            assert_eq!(image.width(), *size as u32);
            assert_eq!(image.height(), *size as u32);
        }
        assert!(TRAY_ICON_PNGS.len() >= 3, "托盘图标应覆盖多个常用尺寸");
    }

    #[test]
    fn converts_rgba_pixels_to_argb_network_order() {
        // RGBA [r=1, g=2, b=3, a=4] 旋转后应变为 ARGB [a=4, r=1, g=2, b=3]。
        let mut data = vec![1, 2, 3, 4];
        for pixel in data.chunks_exact_mut(4) {
            pixel.rotate_right(1);
        }
        assert_eq!(data, vec![4, 1, 2, 3]);
    }

    #[test]
    fn tray_icons_cover_expected_pixel_sizes() {
        let sizes: Vec<i32> = TRAY_ICONS.iter().map(|icon| icon.width).collect();
        assert!(sizes.contains(&16) && sizes.contains(&32) && sizes.contains(&64));
        for icon in TRAY_ICONS.iter() {
            assert_eq!(icon.data.len(), (icon.width * icon.height * 4) as usize);
        }
    }
}
