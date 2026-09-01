use std::{
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
    thread::{self, JoinHandle},
};

use anyhow::{Context, Result};
use async_channel::{Receiver, Sender};
use windows_sys::Win32::{
    Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_FAILED, WAIT_OBJECT_0},
    System::Threading::{CreateEventW, CreateMutexW, INFINITE, SetEvent, WaitForMultipleObjects},
};

const INSTANCE_MUTEX_NAME: &str =
    "Local\\PureClash.SingleInstance.4D606E11-782D-4FCE-91E3-8E3A56ED2642";
const ACTIVATE_EVENT_NAME: &str = "Local\\PureClash.Activate.4D606E11-782D-4FCE-91E3-8E3A56ED2642";

/// 单实例检查结果；次实例必须立即结束启动流程，是否通知首实例由启动模式决定。
pub(crate) enum SingleInstanceState {
    /// 当前进程是首实例，同时提供后续启动请求的异步接收端。
    Primary {
        guard: SingleInstance,
        activation_requests: Receiver<()>,
    },
    /// 已有首实例运行；交互启动已发送激活信号，自启启动保持静默。
    Secondary,
}

/// Windows 单实例资源，持有命名 Mutex 并管理激活事件等待线程。
pub(crate) struct SingleInstance {
    _mutex: OwnedHandle,
    shutdown_event: OwnedHandle,
    wait_thread: Option<JoinHandle<()>>,
}

impl SingleInstance {
    /// 在当前 Windows 会话中获取 Pure Clash 单实例资格。
    pub(crate) fn acquire(notify_primary: bool) -> Result<SingleInstanceState> {
        acquire_named(INSTANCE_MUTEX_NAME, ACTIVATE_EVENT_NAME, notify_primary)
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        // 先唤醒等待线程再回收句柄，避免应用退出时留下阻塞线程。
        let result = unsafe { SetEvent(self.shutdown_event.as_raw_handle() as HANDLE) };
        if result == 0 {
            log_warn!("app", "停止单实例监听失败：{}", io::Error::last_os_error());
        }
        if let Some(wait_thread) = self.wait_thread.take() {
            let _ = wait_thread.join();
        }
    }
}

fn acquire_named(
    mutex_name: &str,
    activate_event_name: &str,
    notify_primary: bool,
) -> Result<SingleInstanceState> {
    // Event 先于 Mutex 创建，确保并发启动的次实例不会错过首个激活请求。
    let activate_event =
        create_event(Some(activate_event_name)).context("无法创建单实例激活事件")?;
    let mutex_name = wide_null(mutex_name);
    let mutex = unsafe { CreateMutexW(ptr::null(), 0, mutex_name.as_ptr()) };
    if mutex.is_null() {
        return Err(io::Error::last_os_error()).context("无法创建 Pure Clash 单实例 Mutex");
    }
    // GetLastError 必须紧跟 CreateMutexW，其他 Win32 调用可能覆盖该状态。
    let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let mutex = unsafe { OwnedHandle::from_raw_handle(mutex) };

    if already_exists && notify_primary {
        let result = unsafe { SetEvent(activate_event.as_raw_handle() as HANDLE) };
        if result == 0 {
            return Err(io::Error::last_os_error()).context("无法通知已运行的 Pure Clash 实例");
        }
    }
    if already_exists {
        return Ok(SingleInstanceState::Secondary);
    }

    let shutdown_event = create_event(None).context("无法创建单实例监听关闭事件")?;
    let shutdown_wait_handle = shutdown_event
        .try_clone()
        .context("无法复制单实例监听关闭句柄")?;
    let (activation_sender, activation_requests) = async_channel::bounded(1);
    let wait_thread = thread::Builder::new()
        .name("pure-clash-single-instance".to_owned())
        .spawn(move || wait_for_activation(activate_event, shutdown_wait_handle, activation_sender))
        .context("无法启动单实例监听线程")?;

    Ok(SingleInstanceState::Primary {
        guard: SingleInstance {
            _mutex: mutex,
            shutdown_event,
            wait_thread: Some(wait_thread),
        },
        activation_requests,
    })
}

fn create_event(name: Option<&str>) -> io::Result<OwnedHandle> {
    let name = name.map(wide_null);
    let name_ptr = name.as_ref().map_or(ptr::null(), |name| name.as_ptr());
    // 自动重置事件会合并短时间内的重复启动，主线程每次只处理一个激活请求。
    let event = unsafe { CreateEventW(ptr::null(), 0, 0, name_ptr) };
    if event.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { OwnedHandle::from_raw_handle(event) })
    }
}

fn wait_for_activation(
    activate_event: OwnedHandle,
    shutdown_event: OwnedHandle,
    activation_sender: Sender<()>,
) {
    let handles = [
        activate_event.as_raw_handle() as HANDLE,
        shutdown_event.as_raw_handle() as HANDLE,
    ];
    loop {
        let wait_result =
            unsafe { WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, INFINITE) };
        match wait_result {
            WAIT_OBJECT_0 => {
                // 容量为 1 时自然合并连续启动请求，避免主线程堆积重复激活操作。
                let _ = activation_sender.try_send(());
            }
            result if result == WAIT_OBJECT_0 + 1 => break,
            WAIT_FAILED => {
                log_warn!(
                    "app",
                    "等待单实例激活事件失败：{}",
                    io::Error::last_os_error()
                );
                break;
            }
            result => {
                log_warn!("app", "等待单实例激活事件返回未知状态：{result}");
                break;
            }
        }
    }
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicU32, Ordering},
        time::{Duration, Instant},
    };

    use super::*;

    static TEST_SEQUENCE: AtomicU32 = AtomicU32::new(0);

    #[test]
    fn second_instance_notifies_primary_instance() {
        let suffix = format!(
            "{}.{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let mutex_name = format!("Local\\PureClash.Test.Mutex.{suffix}");
        let event_name = format!("Local\\PureClash.Test.Event.{suffix}");
        let primary = acquire_named(&mutex_name, &event_name, true).expect("首实例创建应成功");
        let (guard, activation_requests) = match primary {
            SingleInstanceState::Primary {
                guard,
                activation_requests,
            } => (guard, activation_requests),
            SingleInstanceState::Secondary => panic!("首次获取不应被判定为次实例"),
        };

        assert!(matches!(
            acquire_named(&mutex_name, &event_name, true).expect("次实例通知应成功"),
            SingleInstanceState::Secondary
        ));

        let deadline = Instant::now() + Duration::from_secs(1);
        let received = loop {
            if activation_requests.try_recv().is_ok() {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            thread::yield_now();
        };
        assert!(received, "首实例应在超时前收到激活请求");
        drop(guard);
    }

    #[test]
    fn autostart_secondary_instance_does_not_notify_primary() {
        let suffix = format!(
            "{}.{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let mutex_name = format!("Local\\PureClash.Test.Mutex.{suffix}");
        let event_name = format!("Local\\PureClash.Test.Event.{suffix}");
        let primary = acquire_named(&mutex_name, &event_name, false).expect("首实例创建应成功");
        let (guard, activation_requests) = match primary {
            SingleInstanceState::Primary {
                guard,
                activation_requests,
            } => (guard, activation_requests),
            SingleInstanceState::Secondary => panic!("首次获取不应被判定为次实例"),
        };

        assert!(matches!(
            acquire_named(&mutex_name, &event_name, false).expect("自启次实例检查应成功"),
            SingleInstanceState::Secondary
        ));
        assert!(activation_requests.try_recv().is_err());
        drop(guard);
    }
}
