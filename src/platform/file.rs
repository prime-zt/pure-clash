//! 跨平台原子文件替换。
//!
//! 配置和运行时文件始终先写入同目录临时文件，完整同步后再替换正式文件。
//! Windows 需要 `MoveFileExW(REPLACE_EXISTING)`，unix 可直接使用原子 `rename`。

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

use anyhow::{Context, Result};
use uuid::Uuid;

/// 原子写入完整文件内容；失败时保留原文件并尽力清理临时文件。
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("原子写入目标缺少父目录")?;
    fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let temporary = parent.join(format!(".{name}.tmp-{}", Uuid::new_v4().simple()));

    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .with_context(|| format!("无法创建临时文件：{}", temporary.display()))?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        replace_file(&temporary, path)?;

        // unix 可同步目录项；Windows 由 MOVEFILE_WRITE_THROUGH 保证替换落盘。
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// 用同一文件系统中的临时文件原子替换目标。
#[cfg(unix)]
fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    fs::rename(source, destination).with_context(|| {
        format!(
            "无法原子替换文件 {} -> {}",
            source.display(),
            destination.display()
        )
    })
}

#[cfg(target_os = "windows")]
fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        return Err(std::io::Error::last_os_error()).context("Windows 无法原子替换文件");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_existing_content_without_temp_files() {
        let root = std::env::temp_dir().join(format!(
            "pure-clash-atomic-write-{}-{}",
            std::process::id(),
            Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).expect("应创建原子写入测试目录");
        let target = root.join("config.json");
        fs::write(&target, b"old").expect("应写入旧内容");

        atomic_write(&target, b"new").expect("应原子替换已有文件");

        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).expect("应清理原子写入测试目录");
    }
}
