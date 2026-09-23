mod autostart;
mod elevated_helper;
mod elevation;
mod job;
pub(crate) mod process_guard;
mod single_instance;
pub(crate) mod system_proxy;
mod tray;
mod window_ctrl;

pub(crate) use autostart::{autostart_status, set_autostart};
pub(crate) use elevated_helper::run_if_requested as run_elevated_helper_if_requested;
pub(crate) use elevation::{ElevatedProcess, launch_elevated};
pub(crate) use single_instance::{SingleInstance, SingleInstanceState};
pub(crate) use system_proxy::{capture_system_proxy, restore_system_proxy, set_system_proxy};
pub(crate) use tray::SystemTray;
pub(crate) use window_ctrl::show_main_window;
