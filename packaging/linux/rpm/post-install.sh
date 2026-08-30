#!/bin/sh
# RPM 安装后脚本：建立 /usr/bin 命令软链，指向 /opt 下的程序目录。
if [ ! -e /usr/bin/pure-clash ]; then
    ln -s /opt/pure-clash/pure-clash /usr/bin/pure-clash
fi
