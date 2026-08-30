#!/usr/bin/env bash
# 组装 AppImage：程序 + 随包内核整体进入 AppDir/usr/bin，保证内核目录
# 相对可执行文件解析；共享库由 linuxdeploy 按需收集。
#
# 用法：build-appimage.sh <内核版本目录名> [包版本]
# 前置：target/release/pure-clash 已构建；linuxdeploy 可执行文件在 PATH。
# 内核版本目录名用于定位随包内核；AppImage 文件名使用包版本（缺省时读
# 取 Cargo 元数据）。
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
kernel_version="${1:?用法: build-appimage.sh <内核版本目录名> [包版本]}"
binary="$root/target/release/pure-clash"
appdir="$root/target/appimage/AppDir"
if [ -n "${2:-}" ]; then
    version="$2"
else
    version="$(cargo metadata --no-deps --format-version 1 |
        jq -r '.packages[] | select(.name == "pure-clash") | .version')"
fi

if [ ! -x "$binary" ]; then
    echo "缺少 release 可执行文件：$binary" >&2
    exit 1
fi
if [ ! -x "$root/kernel/$kernel_version/pc-mihomo" ]; then
    echo "内核文件缺失：$root/kernel/$kernel_version/pc-mihomo" >&2
    exit 1
fi

rm -rf "$appdir"
mkdir -p "$appdir/usr/bin/kernel/$kernel_version" \
    "$appdir/usr/share/applications" \
    "$appdir/usr/share/icons/hicolor/64x64/apps"

install -m 755 "$binary" "$appdir/usr/bin/pure-clash"
install -m 755 "$root/kernel/$kernel_version/pc-mihomo" "$appdir/usr/bin/kernel/$kernel_version/pc-mihomo"
install -m 644 "$root/kernel/$kernel_version/LICENSE" "$root/kernel/$kernel_version/NOTICE.md" \
    "$root/kernel/$kernel_version/manifest.json" "$appdir/usr/bin/kernel/$kernel_version/"
install -m 644 "$root/packaging/linux/pure-clash.desktop" "$appdir/usr/share/applications/"
install -m 644 "$root/packaging/linux/pure-clash.png" \
    "$appdir/usr/share/icons/hicolor/64x64/apps/pure-clash.png"

cd "$root/target/appimage"
# linuxdeploy 在 APPIMAGE_EXTRACT_AND_RUN=1 下可能把产物写到 $HOME 而非当前
# 目录；先打时间戳，再从多个候选位置取最新的 AppImage。
stamp="$(mktemp)"
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 linuxdeploy \
    --appdir "$appdir" \
    --output appimage

generated="$(find "$root/target/appimage" "$root" "$HOME" -maxdepth 1 \
    -name '*.AppImage' -newer "$stamp" 2>/dev/null | head -n1)"
if [ -z "$generated" ]; then
    echo "未找到 linuxdeploy 生成的 AppImage" >&2
    exit 1
fi
mv -f "$generated" "$root/target/appimage/Pure_Clash-$version-x86_64.AppImage"
rm -f "$stamp"
echo "产物：$root/target/appimage/Pure_Clash-$version-x86_64.AppImage"
