use std::{
    io::{self, Write},
    os::linux::net::SocketAddrExt,
    os::unix::net::{SocketAddr, UnixListener, UnixStream},
    sync::Arc,
    sync::atomic::{AtomicBool, Ordering},
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::{Context, Result};
use async_channel::{Receiver, Sender, bounded};

/// 抽象命名空间 socket 名称前缀；后缀拼接当前用户 UID，避免多用户互相干扰。
const INSTANCE_SOCKET_BASE: &str = "pure-clash-single-instance";

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

/// Linux 单实例资源，持有激活接收线程与退出标志；
/// 监听 socket 由接收线程持有，随线程结束自动释放。
pub(crate) struct SingleInstance {
    shutdown: Arc<AtomicBool>,
    accept_thread: Option<JoinHandle<()>>,
}

impl SingleInstance {
    /// 在当前用户会话中获取 Pure Clash 单实例资格。
    pub(crate) fn acquire(notify_primary: bool) -> Result<SingleInstanceState> {
        // 抽象 socket 名在内核网络命名空间内全局可见，按用户隔离。
        let uid = unsafe { libc::getuid() };
        acquire_named(&format!("{INSTANCE_SOCKET_BASE}-{uid}"), notify_primary)
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        // 置退出标志；accept 线程在轮询间隔内自然退出，join 最多等待一个轮询周期。
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(accept_thread) = self.accept_thread.take() {
            let _ = accept_thread.join();
        }
    }
}

fn acquire_named(name: &str, notify_primary: bool) -> Result<SingleInstanceState> {
    let address = SocketAddr::from_abstract_name(name.as_bytes())
        .with_context(|| format!("无法解析单实例 socket 名称：{name}"))?;

    match UnixListener::bind_addr(&address) {
        Ok(listener) => {
            // 非阻塞轮询 accept：try_clone 出的 fd 与原 fd 共享内核文件描述，
            // 关闭原 fd 无法唤醒阻塞 accept，只能用退出标志 + 轮询保证可回收。
            listener
                .set_nonblocking(true)
                .context("无法配置单实例监听 socket")?;
            // 容量 1 时自然合并连续启动请求，避免主线程堆积重复激活操作。
            let (activation_sender, activation_requests) = bounded::<()>(1);
            let shutdown = Arc::new(AtomicBool::new(false));
            let accept_shutdown = Arc::clone(&shutdown);
            let accept_thread = thread::Builder::new()
                .name("pure-clash-single-instance".to_owned())
                .spawn(move || accept_loop(listener, accept_shutdown, activation_sender))
                .context("无法启动单实例监听线程")?;

            Ok(SingleInstanceState::Primary {
                guard: SingleInstance {
                    shutdown,
                    accept_thread: Some(accept_thread),
                },
                activation_requests,
            })
        }
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
            if notify_primary {
                notify_primary(&address)?;
            }
            Ok(SingleInstanceState::Secondary)
        }
        Err(error) => Err(error).context("无法创建 Pure Clash 单实例监听"),
    }
}

/// 首实例的接收循环：每个新连接即一次激活请求，连接内容不解析。
fn accept_loop(listener: UnixListener, shutdown: Arc<AtomicBool>, activation_sender: Sender<()>) {
    loop {
        match listener.accept() {
            // 立即关闭连接：建立连接本身就代表一次“打开主窗口”请求。
            Ok((stream, _)) => drop(stream),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(error) => {
                eprintln!("单实例监听结束：{error}");
                break;
            }
        }
        let _ = activation_sender.try_send(());
    }
}

/// 通知已运行的首实例恢复主窗口；连接建立后写入内容仅为语义完整。
fn notify_primary(address: &SocketAddr) -> Result<()> {
    let mut stream =
        UnixStream::connect_addr(address).context("无法连接已运行的 Pure Clash 实例")?;
    // 即使写入失败，连接建立已足够让首实例的 accept 收到激活请求。
    let _ = stream.write_all(b"activate");
    let _ = stream.flush();
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicU32, Ordering},
        time::{Duration, Instant},
    };

    use super::*;

    static TEST_SEQUENCE: AtomicU32 = AtomicU32::new(0);

    fn unique_name() -> String {
        format!(
            "pure-clash-test-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        )
    }

    #[test]
    fn second_instance_notifies_primary_instance() {
        let name = unique_name();
        let primary = acquire_named(&name, true).expect("首实例创建应成功");
        let (guard, activation_requests) = match primary {
            SingleInstanceState::Primary {
                guard,
                activation_requests,
            } => (guard, activation_requests),
            SingleInstanceState::Secondary => panic!("首次获取不应被判定为次实例"),
        };

        assert!(matches!(
            acquire_named(&name, true).expect("次实例通知应成功"),
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
    fn released_guard_allows_new_primary() {
        let name = unique_name();
        drop(acquire_named(&name, true).expect("首实例创建应成功"));

        // 抽象 socket 随进程内 socket 关闭立即释放，重新获取应恢复首实例资格。
        assert!(matches!(
            acquire_named(&name, true).expect("释放后重新获取应成功"),
            SingleInstanceState::Primary { .. }
        ));
    }

    #[test]
    fn autostart_secondary_instance_does_not_notify_primary() {
        let name = unique_name();
        let primary = acquire_named(&name, false).expect("首实例创建应成功");
        let (guard, activation_requests) = match primary {
            SingleInstanceState::Primary {
                guard,
                activation_requests,
            } => (guard, activation_requests),
            SingleInstanceState::Secondary => panic!("首次获取不应被判定为次实例"),
        };

        assert!(matches!(
            acquire_named(&name, false).expect("自启次实例检查应成功"),
            SingleInstanceState::Secondary
        ));
        assert!(activation_requests.try_recv().is_err());
        drop(guard);
    }
}
