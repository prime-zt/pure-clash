use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() {
    // 内核 manifest 是发行版本与随包文件名的唯一来源，避免运行时重复硬编码。
    println!("cargo:rerun-if-changed=kernel");
    println!("cargo:rerun-if-changed=assets/windows/pure-clash.ico");

    let project_root =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo 应提供 CARGO_MANIFEST_DIR"));
    compile_windows_resources(&project_root);
    let manifest_path = find_kernel_manifest(&project_root.join("kernel"));
    let manifest_content = fs::read_to_string(&manifest_path).unwrap_or_else(|error| {
        panic!(
            "无法读取内置 Mihomo manifest {}：{error}",
            manifest_path.display()
        )
    });
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_content).unwrap_or_else(|error| {
            panic!(
                "内置 Mihomo manifest 格式无效 {}：{error}",
                manifest_path.display()
            )
        });
    let version = manifest
        .get("version")
        .and_then(serde_json::Value::as_str)
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| {
            panic!(
                "内置 Mihomo manifest 缺少有效 version：{}",
                manifest_path.display()
            )
        });
    if version == "."
        || version == ".."
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        panic!("内置 Mihomo manifest version 不是安全的版本目录名：{version}");
    }

    let version_dir = manifest_path
        .parent()
        .expect("内置 Mihomo manifest 应位于版本目录")
        .file_name()
        .and_then(|name| name.to_str())
        .expect("内置 Mihomo 版本目录名必须是 UTF-8");
    if version_dir != version {
        panic!(
            "内置 Mihomo 目录名与 manifest version 不一致：目录 {version_dir}，manifest {version}"
        );
    }

    // manifest 的 targets 按 `<os>-<arch>` 键记录各发行目标；只要求当前编译目标
    // 的内核文件存在，允许仓库只为部分平台携带二进制。
    let targets = manifest
        .get("targets")
        .and_then(serde_json::Value::as_object)
        .unwrap_or_else(|| {
            panic!(
                "内置 Mihomo manifest 缺少 targets 对象：{}",
                manifest_path.display()
            )
        });
    let target_key = current_target_key();
    let target = targets.get(&target_key).unwrap_or_else(|| {
        panic!(
            "内置 Mihomo manifest 缺少当前编译目标 {target_key} 的条目：{}",
            manifest_path.display()
        )
    });

    let binary = target
        .get("binary")
        .and_then(serde_json::Value::as_str)
        .filter(|binary| !binary.is_empty())
        .unwrap_or_else(|| {
            panic!(
                "内置 Mihomo manifest 目标 {target_key} 缺少有效 binary：{}",
                manifest_path.display()
            )
        });
    if binary == "."
        || binary == ".."
        || !binary
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        panic!("内置 Mihomo manifest binary 不是安全的文件名：{binary}");
    }
    let binary_path = manifest_path
        .parent()
        .expect("内置 Mihomo manifest 应位于版本目录")
        .join(binary);
    if !binary_path.is_file() {
        panic!(
            "内置 Mihomo 文件不存在：{}（目标 {target_key} 需要 {binary}，请按 README 放置对应内核）",
            binary_path.display()
        )
    }

    // Windows 依赖 wintun 驱动才能启用 TUN，manifest 声明了它的版本与哈希来源，
    // 编译期与内核一起校验存在性，避免发布缺少 TUN 能力的安装包。
    if let Some(wintun) = target.get("wintun") {
        let file = wintun
            .get("file")
            .and_then(serde_json::Value::as_str)
            .filter(|file| {
                !file.is_empty()
                    && file.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')
                    })
            })
            .unwrap_or_else(|| {
                panic!("内置 Mihomo manifest 目标 {target_key} 的 wintun.file 不是安全的文件名")
            });
        let wintun_path = manifest_path.parent().expect("应位于版本目录").join(file);
        if !wintun_path.is_file() {
            panic!(
                "内置 Mihomo 文件不存在：{}（目标 {target_key} 的 TUN 需要 {file}，请从 manifest 记录的官方地址获取）",
                wintun_path.display()
            )
        }
    }

    println!("cargo:rustc-env=PURE_CLASH_DEFAULT_MIHOMO_VERSION={version}");
    println!("cargo:rustc-env=PURE_CLASH_DEFAULT_MIHOMO_BINARY={binary}");
}

/// 按当前编译目标合成 manifest `targets` 的键名，与随包内核条目一一对应。
fn current_target_key() -> String {
    let os = env::var("CARGO_CFG_TARGET_OS").expect("Cargo 应提供 CARGO_CFG_TARGET_OS");
    let arch = env::var("CARGO_CFG_TARGET_ARCH").expect("Cargo 应提供 CARGO_CFG_TARGET_ARCH");
    match (os.as_str(), arch.as_str()) {
        ("windows", "x86_64") => "windows-amd64".to_owned(),
        ("linux", "x86_64") => "linux-amd64".to_owned(),
        ("macos", "x86_64") => "macos-amd64".to_owned(),
        ("macos", "aarch64") => "macos-aarch64".to_owned(),
        (os, arch) => panic!(
            "Pure Clash 尚未支持编译目标 {os}-{arch}，请在 manifest targets 中补充或更换目标"
        ),
    }
}

fn compile_windows_resources(project_root: &Path) {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    // GPUI 0.2.2 会读取资源 ID 1 作为 Windows 窗口图标，winresource 的默认 ID 正好为 1。
    let icon_path = project_root.join("assets/windows/pure-clash.ico");
    if !icon_path.is_file() {
        panic!("Windows 应用图标不存在：{}", icon_path.display());
    }
    let icon_path = icon_path
        .to_str()
        .expect("Windows 应用图标路径必须是有效 UTF-8");
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(icon_path);
    resource
        .compile()
        .unwrap_or_else(|error| panic!("无法编译 Windows 应用图标资源：{error}"));
}

fn find_kernel_manifest(kernel_root: &Path) -> PathBuf {
    let mut manifests = fs::read_dir(kernel_root)
        .unwrap_or_else(|error| {
            panic!(
                "无法读取内置 Mihomo 目录 {}：{error}",
                kernel_root.display()
            )
        })
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if !file_type.is_dir() {
                return None;
            }
            let manifest = entry.path().join("manifest.json");
            manifest.is_file().then_some(manifest)
        });

    let first = manifests
        .next()
        .unwrap_or_else(|| panic!("内置 Mihomo 目录中没有找到版本 manifest.json"));
    if manifests.next().is_some() {
        panic!("内置 Mihomo 目录只能包含一个默认版本 manifest.json");
    }
    first
}
