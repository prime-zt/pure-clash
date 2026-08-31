#!/bin/sh
# RPM 的升级事务也会执行旧包 %postun，只有 $1=0 才是完整卸载。
LINK=/usr/bin/pure-clash
TARGET=/opt/pure-clash/pure-clash

if [ "$1" -eq 0 ] && [ -L "$LINK" ] && [ "$(readlink "$LINK")" = "$TARGET" ]; then
    rm -f "$LINK"
fi
