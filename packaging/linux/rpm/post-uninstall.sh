#!/bin/sh
# RPM 卸载后脚本：仅删除本包创建的命令软链，不动 /opt 程序目录。
if [ -L /usr/bin/pure-clash ]; then
    rm -f /usr/bin/pure-clash
fi
