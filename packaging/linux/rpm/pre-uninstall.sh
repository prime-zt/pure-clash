#!/bin/sh
# RPM 完整卸载前清理动态安装的 root TUN 服务；升级时 $1 大于 0，必须保留服务。
set -e

TARGET=/opt/pure-clash/pure-clash
SERVICE_UNIT=/etc/systemd/system/pure-clash-service.service
SERVICE_BINARY=/usr/libexec/pure-clash-service
SERVICE_ROOT=/usr/lib/pure-clash
SERVICE_STATE=/var/lib/pure-clash-service

if [ "$1" -eq 0 ] && [ -x "$TARGET" ] && { [ -e "$SERVICE_UNIT" ] || [ -e "$SERVICE_BINARY" ] || [ -e "$SERVICE_ROOT" ] || [ -e "$SERVICE_STATE" ]; }; then
    "$TARGET" --linux-tun-service-uninstall
fi
