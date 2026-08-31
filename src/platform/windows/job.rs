use std::{io, mem::size_of, os::windows::io::AsRawHandle, process::Child, ptr};

use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    },
};

/// 持有 Mihomo 专用 Job Object；关闭句柄时 Windows 会终止仍在运行的子进程。
pub(crate) struct JobObject {
    handle: HANDLE,
}

// SAFETY: Job Object 是由本类型独占的 Windows 内核句柄，没有线程亲和性；所有操作
// 都通过 Win32 线程安全 API 完成，移动所有权不会让句柄被并发关闭或重复释放。
unsafe impl Send for JobObject {}

impl JobObject {
    /// 创建启用了 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` 的匿名 Job Object。
    pub(crate) fn new() -> io::Result<Self> {
        // SAFETY: 传入空安全属性和空名称，返回值由本类型独占并在 Drop 中关闭。
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }

        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `limits` 的类型和长度与 JobObjectExtendedLimitInformation 完全匹配。
        let configured = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            let error = io::Error::last_os_error();
            // SAFETY: `handle` 是本函数刚创建且尚未交给其他对象的有效句柄。
            unsafe { CloseHandle(handle) };
            return Err(error);
        }

        Ok(Self { handle })
    }

    /// 将已启动的 Mihomo 子进程加入当前 Job Object。
    pub(crate) fn assign(&self, child: &Child) -> io::Result<()> {
        let process_handle = child.as_raw_handle() as HANDLE;
        self.assign_handle(process_handle)
    }

    /// 按原生句柄加入 Job Object；提权内核不是本进程的 CreateProcess 子进程。
    pub(crate) fn assign_handle(&self, process_handle: HANDLE) -> io::Result<()> {
        // SAFETY: 两个句柄在调用期间均有效，Job Object 由 `self` 持续持有。
        if unsafe { AssignProcessToJobObject(self.handle, process_handle) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for JobObject {
    fn drop(&mut self) {
        // SAFETY: 句柄由本对象独占且只在这里关闭一次；关闭会清理仍归属该 Job 的进程。
        unsafe { CloseHandle(self.handle) };
    }
}
