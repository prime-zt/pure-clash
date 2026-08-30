// `Path` 的使用方：unix 平台的 ensure_executable（release 也编译）与
// debug 构建的开发目录回退逻辑；Windows release 不需要。
#[cfg(any(debug_assertions, unix))]
use std::path::Path;
use std::path::PathBuf;

use crate::platform::{AppPaths, kernel_binary_name};

/// 确保 unix 内核文件具备可执行位；随包文件由应用管理，缺失时直接补齐。
#[cfg(target_os = "linux")]
fn ensure_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::metadata(path)?;
    if metadata.permissions().mode() & 0o111 != 0 {
        return Ok(());
    }

    let mut permissions = metadata.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

/// macOS 尚未验证内核生命周期，路径解析保持可用但要求文件已具备可执行位。
#[cfg(target_os = "macos")]
fn ensure_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    if std::fs::metadata(path)?.permissions().mode() & 0o111 == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "内核文件缺少可执行权限，请先执行 chmod +x",
        ));
    }
    Ok(())
}

fn bundled_path_in(paths: &AppPaths, version: &str) -> PathBuf {
    paths.kernel_dir.join(version).join(kernel_binary_name())
}

/// 返回当前平台随包资源目录中的 Mihomo 路径。
///
/// Windows 使用程序目录，macOS 预留应用包 Resources，Linux 预留便携式资源目录。
pub(crate) fn bundled_path(paths: &AppPaths, version: &str) -> std::io::Result<PathBuf> {
    if !is_valid_version(version) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Mihomo 版本只能是单个目录名",
        ));
    }

    let installed_path = bundled_path_in(paths, version);

    // `cargo run` 的可执行文件位于 target 目录，开发构建回退到仓库内核；release 始终使用安装目录。
    #[cfg(debug_assertions)]
    let installed_path = if !installed_path.is_file() {
        let project_paths = AppPaths::portable(Path::new(env!("CARGO_MANIFEST_DIR")));
        let project_path = bundled_path_in(&project_paths, version);
        if project_path.is_file() {
            project_path
        } else {
            installed_path
        }
    } else {
        installed_path
    };

    // unix 内核文件必须具备可执行位（linux 自动补齐，macOS 要求用户先行授权）。
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    ensure_executable(&installed_path)?;

    Ok(installed_path)
}

/// 判断当前平台资源目录是否包含预期版本的内核文件。
pub(crate) fn is_available(paths: &AppPaths, version: &str) -> bool {
    bundled_path(paths, version)
        .map(|path| path.is_file())
        .unwrap_or(false)
}

fn is_valid_version(version: &str) -> bool {
    // 配置来自本地文件，只接受版本目录允许的字符，避免路径穿越或特殊路径语义。
    !version.is_empty()
        && version != "."
        && version != ".."
        && version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_version_and_binary_come_from_kernel_manifest() {
        assert!(!env!("PURE_CLASH_DEFAULT_MIHOMO_VERSION").is_empty());
        assert_eq!(
            kernel_binary_name(),
            env!("PURE_CLASH_DEFAULT_MIHOMO_BINARY")
        );
        #[cfg(target_os = "windows")]
        assert_eq!(kernel_binary_name(), "pc-mihomo.exe");
    }

    #[test]
    fn builds_versioned_path_from_program_directory() {
        let root = PathBuf::from(r"C:\Pure Clash");
        let paths = AppPaths::portable(&root);
        let version = env!("PURE_CLASH_DEFAULT_MIHOMO_VERSION");
        assert_eq!(
            bundled_path_in(&paths, version),
            root.join("kernel").join(version).join(kernel_binary_name())
        );
    }

    #[test]
    fn rejects_version_path_traversal() {
        assert!(!is_valid_version(""));
        assert!(!is_valid_version(".."));
        assert!(!is_valid_version(r"test-version\other"));
        assert!(is_valid_version("test-version"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn adds_executable_bits_to_bundled_kernel() {
        use std::os::unix::fs::PermissionsExt;

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("系统时间应晚于 UNIX_EPOCH")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "pure-clash-kernel-exec-{}-{nonce}",
            std::process::id()
        ));
        std::fs::write(&path, b"fake-kernel").expect("应写入测试内核文件");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("应移除可执行位");

        ensure_executable(&path).expect("应自动补齐可执行位");
        let mode = std::fs::metadata(&path)
            .expect("应读取测试内核文件")
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0);

        std::fs::remove_file(&path).expect("应清理测试文件");
    }
}
