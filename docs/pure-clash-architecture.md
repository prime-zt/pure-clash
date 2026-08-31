# Pure Clash 技术方案

> 状态：迁移后的产品技术基线
> 更新时间：2026-08-31
> 当前范围：Windows x64 与 Linux x64，均支持内核生命周期、系统代理与 TUN

Pure Clash 是使用 Rust 和 Zed GPUI 构建的 Mihomo 原生桌面客户端。GPUI 负责界面与产品状态，Mihomo 作为独立进程负责代理协议、DNS、规则匹配与流量转发。

## 1. 目标与非目标

### 1.1 目标

- 管理本地 YAML 和远程订阅配置，更新失败时保留最后可用版本。
- 可靠启动、停止、重启和诊断 Mihomo，应用退出后不残留子进程。
- 通过本机 REST/WebSocket controller 展示内核状态、代理组、规则、连接、流量、内存和日志。
- 切换代理节点与 `rule`、`global`、`direct` 运行模式。
- 控制 Windows 与 Linux GNOME/Cinnamon 当前用户的系统代理，记录并恢复修改前状态。
- 通过 UAC（Windows）或 polkit（Linux）明确授权 TUN，并在未真实生效时自动回退。
- 分发经过版本锁定与 SHA-256 校验的 Windows x64 内核，并提供明确的升级和回滚路径。

### 1.2 非目标

- 不重写 Mihomo 的协议栈、DNS、规则引擎或 provider 更新逻辑。
- 不向局域网或公网开放 external controller。
- 不把 controller secret、订阅 URL 或供应商认证信息写入日志。
- 不静默安装服务、驱动或获取管理员权限。
- TUN 不与普通系统代理混为一项，也不通过安装器静默获得长期管理员权限。
- 当前不支持 macOS、Linux 非 x64、ARM64 和移动端。

## 2. 已核验技术基线

2026-08-28 对官方资料和当前随包发行清单核验：

- Mihomo 提供完整 HTTP RESTful controller；`/connections` 支持单次 GET 快照，连接与实时流量经 1 秒轮询消费（取舍见 4.3），`/memory`、`/logs` 等流式能力尚未接入。
- 项目随包内核版本以版本目录中的 `manifest.json` 为唯一来源；后续升级仍需更新受控清单并重新校验，应用不会在启动时自动选择“最新”。
- 官方入口支持 `-d` 指定配置目录、`-f` 指定配置文件、`-t` 校验配置、`-ext-ctl` 覆盖 controller 地址、`-secret` 覆盖 REST API secret。
- Mihomo 采用 GPL-3.0。其 README 还要求非 MetaCubeX 下游项目名称不得包含 `mihomo`；`Pure Clash` 不触碰这一命名限制。

实现时以安装清单锁定的目标版本为准，并再次核对：

- [官方文档](https://wiki.metacubex.one/)
- [配置文档](https://wiki.metacubex.one/config/)
- [API 文档](https://wiki.metacubex.one/api/)
- [官方仓库](https://github.com/MetaCubeX/mihomo)
- [Releases](https://github.com/MetaCubeX/mihomo/releases)

## 3. 总体架构

```text
┌──────────────────────── Pure Clash Desktop ────────────────────────┐
│ GPUI 页面                                                          │
│ 总览 / 代理 / 连接 / 规则 / 日志 / 配置 / 设置                    │
│                         │                                         │
│ AppState ← MihomoClient ┼─ REST 请求                              │
│                         └─ WebSocket 事件流                        │
│                                   │                               │
│ ConfigStore / CredentialStore / SystemProxy / RuntimeManager      │
└───────────────────────────────────┼───────────────────────────────┘
                                    │ 127.0.0.1 + Bearer secret
                                    ▼
┌──────────────────────── Mihomo sidecar ────────────────────────────┐
│ 配置解析 / DNS / 规则 / 代理 / provider / connections / logs     │
└───────────────────────────────────────────────────────────────────┘
```

### 3.1 进程边界

Rust 侧负责产品状态、配置版本、controller 调用、系统代理、凭据、安装升级和进程生命周期。Mihomo 侧保持独立，不与 GPUI 进程链接；内核崩溃时 UI 仍可展示诊断、恢复配置或切换内核版本。

### 3.2 推荐模块

```text
src/
  main.rs
  platform/
    mod.rs            # 平台目录、随包资源、内核文件名和窗口选项边界
    windows/
      autostart.rs     # HKCU Run 登录自启注册与状态读取
      job.rs           # Job Object
      system_proxy.rs  # 当前用户系统代理快照、设置与恢复
      credential.rs    # Credential Manager 或 DPAPI
    linux/
      autostart.rs     # XDG Autostart desktop entry 与 AppImage 路径处理
      child_guard.rs   # PR_SET_PDEATHSIG 父进程死亡守护，等价 Job Object 兑底
      elevation.rs     # TUN 服务客户端：启动/状态/停止 IPC 与内部模式分流
      tun_service.rs   # 一次授权服务安装、runtime bundle 物化与 root 内核守护
      system_proxy.rs  # GNOME/Cinnamon GSettings 快照、设置与恢复
  app/
    mod.rs
    dashboard.rs
    proxies.rs
    connections.rs
    rules.rs
    logs.rs
    profiles.rs
    settings.rs
  mihomo/
    process.rs       # 子进程、Job Object / pdeathsig、stdout/stderr 和退出状态
    client.rs        # REST 请求、Bearer 鉴权和统一错误
    stream.rs        # WebSocket 重连、背压和事件批处理
    protocol.rs      # 已核验的 API 入参、出参与事件结构
    runtime.rs       # 内核清单、下载、哈希校验和回滚
  config/
    store.rs         # profile 元数据、原始配置和可回滚版本
    subscription.rs  # 订阅获取、大小限制和原子更新
    validation.rs    # 使用目标 Mihomo 版本执行配置校验
```

当前已实现平台目录、进程守护、单实例、托盘、系统代理和 TUN 提权边界：默认配置先由同版本内核执行 `-t`，通过后再启动；Windows 子进程加入 Job Object，普通 Linux 子进程设置 `PR_SET_PDEATHSIG`（SIGKILL）并独立成进程组，unix 停止流程先发送 SIGTERM 最多等待 5 秒再升级 SIGKILL。Windows 系统代理写当前用户注册表，Linux GNOME/Cinnamon 使用 GSettings；Windows TUN 经 UAC 启动内核，Linux 首次经 polkit 安装 root 所有的 systemd 服务，后续通过按 UID 授权的受限 IPC 启动 root Mihomo。Windows 的 `ShellExecuteExW(runas)` 在专用 STA 线程执行，避免 UAC 的嵌套消息循环重入 GPUI 实体更新；启动结果携带代次回传，配置变更、停止或退出后才返回的旧进程会立即回收。Linux DNS 由 Mihomo 的 `dns-hijack` 与双栈 fake-IP 随 TUN 路由接管，桌面进程不额外修改 NetworkManager 设备。controller client、配置订阅、代理选择和运行模式也已接入真实内核。

## 4. 本地 controller 与鉴权

### 4.1 启动参数

当前基础启停使用（`<binary>` 由 manifest 按平台解析，Windows 为 `pc-mihomo.exe`，Linux/macOS 为 `pc-mihomo`；其他平台使用当前平台的进程 API）：

```text
<binary> -d <data-dir>/mihomo -f <config-dir>/mihomo/default.yaml
```

启动前使用完全相同的目录与配置执行：

```text
<binary> -d <data-dir>/mihomo -f <config-dir>/mihomo/default.yaml -t
```

后续接入 controller 时，为每个运行实例分配可用的 loopback 端口并增加：

```text
pc-mihomo.exe -d <managed-home> -f <active-config> \
  -ext-ctl 127.0.0.1:<port> -secret <random-secret>
```

参数必须逐项传给目标平台的进程 API，不拼接成经 shell 解释的命令字符串。controller secret 至少使用 256 bit 的系统安全随机数，只保存在当前进程内存或受保护凭据中。

### 4.2 连接规则

- controller 只绑定 `127.0.0.1`，不使用 `0.0.0.0`。
- 所有 REST 与 WebSocket 请求都携带相同的 Bearer secret。
- 启动完成不能只依赖固定延时；应轮询 controller 版本/状态端点并设置总超时。
- WebSocket 断开采用有上限的指数退避；controller 不可用时保留最近 UI 状态并明确标记“已断开”。
- 高频流量与连接事件按帧或短时间窗合并，避免每条事件触发完整 GPUI 重绘。

### 4.3 API 能力（当前实现状态）

| 产品能力 | controller 资源 | 状态 |
| --- | --- | --- |
| 内核与配置状态 | `GET /version`、`GET /configs` | 已接入 |
| 代理组与节点选择 | `GET /proxies`、`PUT /proxies/{name}` | 已接入 |
| 运行模式 | `PATCH /configs` | 已接入 |
| 节点/分组延迟测试 | `GET /proxies/{name}/delay`、`GET /group/{name}/delay` | 已接入（gstatic 204、5 秒超时） |
| 活动连接与关闭连接 | `GET /connections`、`DELETE /connections[/{id}]` | 已接入 |
| 实时流量 | `GET /connections` 相邻快照差分累计字节数 | 已接入（1 秒轮询，无 WebSocket 依赖） |
| 订阅下载 | 订阅 URL 直接 GET（10MB 上限） | 已接入 |
| 实时内存、日志 | `GET /memory`（流式）、`/logs` | 未接入（当前页面无此需求） |
| 规则列表 | `GET /rules` | 未接入（当前无规则页） |
| provider 更新 | `providers/proxies` | 未接入 |

连接与流量的轮询模型：应用内常驻任务在内核 Running 期间每秒请求一次 `GET /connections`，该端点为单次 JSON 快照（含 `downloadTotal`/`uploadTotal` 与连接数组），速度由相邻快照差分得出；`connections` 字段空闲时为 `null`，内核把失败延迟记录为 0，客户端解析均已兜底。延迟测试使用独立的 10 秒 HTTP 超时 agent（宽于 5 秒探测时长），手动测速结果覆盖 `/proxies` 历史值，未通过测速的节点显示超时。以上字段均已对 1.19.30 内核实测响应与官方文档核验。

无 WebSocket 依赖是有意取舍：轮询单次快照即可覆盖连接、流量与累计值，避免为 speed/memory 再引入 `tungstenite` 级依赖与重连/背压管理；若后续接入 `/logs` 或 `/memory` 流式能力，再评估引入 WebSocket 客户端。

## 5. 配置与订阅

### 5.1 应用目录与平台边界

所有运行时路径由 `AppPaths` 集中解析，不能使用进程工作目录。当前 Windows 产品约定保持不变，后续平台采用可写的系统用户目录：

| 平台 | 配置目录 | 数据目录 | 随包只读资源 |
| --- | --- | --- | --- |
| Windows（当前支持） | `<program-dir>/config` | `<program-dir>/data` | `<program-dir>/kernel/<version>/pc-mihomo.exe`、`<program-dir>/geodata` |
| Linux（x64 已支持） | `$XDG_CONFIG_HOME/pure-clash` 或 `~/.config/pure-clash` | `$XDG_DATA_HOME/pure-clash` 或 `~/.local/share/pure-clash` | `<program-dir>/kernel/<version>/pc-mihomo`、`<program-dir>/geodata` |
| macOS（预留） | `~/Library/Application Support/pure-clash` | `~/Library/Application Support/pure-clash` | `.app/Contents/Resources/kernel/<version>/pc-mihomo`、`.app/Contents/Resources/geodata` |

Linux/macOS 用户目录使用 `directories 6.0` 的 `ProjectDirs` 规则。Linux deb/rpm 将程序和资源安装到 `/opt/pure-clash` 并从 `/usr/bin/pure-clash` 软链启动，AppImage 把资源放到 AppDir 的可执行文件旁；Windows 当前目录示例为：

```text
<program-dir>\
  pure-clash.exe
  kernel\
    <bundled-version>\
      pc-mihomo.exe        # 重命名后的 Windows amd64-compatible 内核
      LICENSE              # Mihomo GPL-3.0 许可证
      NOTICE.md            # 上游归属、源码和非官方关系说明
      manifest.json        # 按编译目标记录下载地址、构建信息和 SHA-256
  geodata\
    GeoSite.dat            # 随包域名规则库
    GeoIP.dat              # 随包二进制 GeoIP 规则库
    Country.mmdb           # 随包 MMDB GeoIP 规则库
    manifest.json          # 锁定同一官方提交、大小和 SHA-256
    LICENSE                # 数据仓库 GPL-3.0 许可证
    NOTICE.md              # 上游来源说明
  config\
    app.json               # AppConfig 主配置
    mihomo\
      default.yaml         # 首次启动创建、仅含 DIRECT 节点的默认内核配置
  data\
    mihomo\                # geodata、cache、provider 等内核运行数据
    profiles\
      <profile-id>\
        source.yaml        # 导入或订阅的原始配置
        active.yaml        # 最近一次校验通过的配置
        history\           # 有上限的回滚版本
    logs\                  # 脱敏诊断日志
```

Linux 布局相同但二进制名为 `pc-mihomo`，内核文件随仓库提交；macOS 需按 manifest 锁定的下载地址与 SHA-256 手动放置内核。

Windows 程序目录必须对当前用户可写；当前 per-user 安装位置满足该约束，不支持放入需要管理员权限才能写入的目录。Linux/macOS 不依赖应用资源目录可写，配置和运行数据进入用户目录。`app.json` 中每个字段都必须有默认值，缺失字段按默认值加载。订阅凭据、controller secret 和其他敏感字段在 Windows 进入 Credential Manager 或 DPAPI 保护的存储；其他平台需要使用对应系统凭据服务，`app.json` 只保存凭据引用。

`config/mihomo/default.yaml` 作为编译期资源嵌入程序，只在运行目录缺少该文件时创建，不覆盖用户修改。默认配置仅监听 loopback 的 mixed port `7890`，启用 `unified-delay` 与 `tcp-concurrent`，策略组只包含 Mihomo 内置 `DIRECT` 出站；这两个行为字段也由本地基线注入所有订阅合并产物。启动按钮固定把 `-f` 指向合并并校验后的 `config/mihomo/runtime.yaml`，把 `-d` 指向 `data/mihomo/`。

配置页同时支持 URL 订阅和本地 YAML 导入。本地文件由 GPUI 原生文件选择器获取，Windows 使用系统对话框、Linux 通过 XDG portal、macOS 使用原生面板；应用只读取至多 10MB 的 UTF-8 内容并保存校验后的副本，不记录源路径。两种来源共用结构预检、本地基线合并、目标内核 `-t` 与原子保存链路，任一步失败均不改变现有 profile、runtime 或运行内核。本地配置引用的相对 provider/规则文件不会随导入复制，用户应选择可独立校验的配置或把资源放入 Mihomo 数据目录。

`geodata/manifest.json` 锁定 MetaCubeX/meta-rules-dat `release` 分支同一 commit 下的 `GeoSite.dat`、`GeoIP.dat`、`Country.mmdb`，并记录大小、SHA-256 和许可证。应用启动时校验随包资源并把完整快照原子复制到 Mihomo 数据目录，以 `.pure-clash-geodata.json` 记录来源与完整性；已有完整官方在线更新时不因应用升级回退到较旧随包快照，文件缺失或损坏则离线恢复整套随包版本。订阅下载、结构预检和内核 `-t` 校验均不得隐式下载 Geo 数据，因此用户在受限网络下仍能完成配置切换。

设置页的手动更新是唯一 Geo 在线更新入口：先读取官方 `release` 分支当前 commit，再从该固定 commit 下载三份文件，限制单文件大小并计算 SHA-256，三件套和状态标记全部提交成功后才视为更新完成；中途失败尽力回滚原文件。更新成功时重启正在运行的 Mihomo，未运行时留待下次启动加载。Windows NSIS 与 Linux deb/rpm/AppImage 均携带数据、manifest、LICENSE 和 NOTICE；卸载只删除随包资源，用户数据目录中的更新版本继续保留。

`app.json` 的 `mihomo_version` 表示启动时选用的内核版本。其默认值和运行时文件名由 `build.rs` 从唯一的随包 `manifest.json` 按当前编译目标注入，运行时按 `AppPaths.kernel_dir/<mihomo_version>/<manifest binary>` 解析；版本配置值只能是单个目录名，manifest 的 `binary` 也只能是安全的单文件名。界面语言由 `language` 字段控制，支持 `zh-CN` 和 `en-US`，缺省为 `zh-CN`。翻译资源由 `rust-i18n 4.2.1` 在编译期从 `locales/` 嵌入可执行文件；切换语言时同步更新全局 locale 和配置文件，不依赖运行时外部语言包。

### 5.2 安全更新流程

1. 下载到同目录临时文件，限制响应大小、重定向次数和总超时。
2. 拒绝空响应、明显的 HTML 错误页和不支持的编码。
3. 使用候选内核执行 `-t` 校验，不在 UI 进程中自行模拟完整 Mihomo schema。
4. 校验成功后用原子替换更新 `active.yaml`，并保留最近若干历史版本。
5. 请求内核重载；若重载失败，恢复上一个可用版本并展示可复制的脱敏错误。

导入配置可能引用相对路径。Pure Clash 必须为每个 profile 明确 home 语义，不得把不可信相对路径解析到应用目录之外；Mihomo 的 safe path 行为也必须按锁定版本验证。

## 6. 桌面生命周期与系统集成

### 6.1 Job Object 与进程守护

Windows：启动 Mihomo 后立即加入专用 Job Object，并设置 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`。当前基础停止流程先发送终止请求并回收子进程，关闭窗口和状态销毁也会执行同一流程；主进程崩溃或被强制终止时由 Job Object 关闭句柄并清理内核。接入 controller 后再评估是否存在可用的优雅退出接口。

Linux：子进程在 spawn 时通过 `pre_exec` 设置 `PR_SET_PDEATHSIG`（SIGKILL），主进程死亡（包括被 SIGKILL 强杀）时内核被同步回收；spawn 闭包内同时用 getppid 校验关闭 fork 与 prctl 之间的竞争窗口。内核独立成进程组（`process_group(0)`），终端信号不直接送达内核；停止流程先发送 SIGTERM 最多等待 5 秒，超时后升级 SIGKILL。pdeathsig 监视的是创建子进程的线程，因此内核 spawn 必须保持在 GPUI 主线程等长寿命线程上。macOS 尚无等价兑底，正式支持前需要单独实现。

### 6.2 系统代理

启用前读取当前用户代理快照并先原子写入 `data/system-proxy.json`，再修改系统设置；写入中途失败时立即尝试恢复，恢复也失败则保留状态文件供下次启动自愈。关闭代理、停内核、应用正常退出或启动恢复流程时还原快照。

Windows 保存 `ProxyEnable` / `ProxyServer` 并经 WinINet 广播变更。Linux 仅在 GNOME/Cinnamon 兼容会话中使用 `gsettings`，完整保存 `mode`、`use-same-proxy`、HTTP/HTTPS/FTP/SOCKS 端点与 HTTP 认证开关的原始 GVariant 文本；启用时四种协议统一指向 loopback mixed-port，恢复时最后切回原 mode。KDE 等未接入的桌面明确报错，不能因安装了 GNOME schema 而宣称成功。macOS 暂不支持。

### 6.3 TUN

TUN 开关写入客户端本地基线，重新合并配置并用同版本 Mihomo 执行 `-t`，运行中的内核随后重启。Windows 通过 `runas` 触发 UAC，只提升 Mihomo 进程并使用随包 wintun。Linux 参考 Clash Verge Rev 的服务模型：首次启用通过 `pkexec` 重新执行内部安装器，验证 `PKEXEC_UID`、Pure Clash 父进程与随包内核后，把服务程序及锁定版本内核原子复制到 root 所有、普通用户不可写的 `/usr/libexec/pure-clash-service` 与 `/usr/lib/pure-clash/kernel/`，写入并启动 `pure-clash-service.service`；后续启停不再调用 `pkexec`。

Linux 服务 socket 位于 `/run/pure-clash-service/service.sock`（`0660`，root 与授权用户私有组），服务通过 `SO_PEERCRED` 复核调用者 UID；IPC 为长度前缀 JSON，只接受 Ping/Start/Status/Stop 四个请求，不接受客户端传入内核路径，只能启动 `/usr/lib/pure-clash/kernel/` 下 root 所有的固定副本。Start 请求携带 runtime bundle：运行时 YAML、本地资源（provider 与 GeoSite/GeoIP/Country 数据）和远程 provider 列表。服务把 bundle 原子物化到 `/var/lib/pure-clash-service/users/<uid>/runtime`——资源来源必须属于授权用户且组/其他位不可写，目标路径禁止穿越、符号链接和保留名；物化以 `.pure-clash-runtime.json` 清单管理增量，先以锁定内核在运行目录内 `-t` 校验，再以 root 启动 Mihomo。对齐 Clash Verge Rev 当前服务模型，服务与内核保持 root 运行，不降 UID、不设置 ambient capabilities、不替换系统网络工具。服务负责进程守护，GUI PID 消失时自动回收内核，协议版本不匹配时强制重新走一次安装授权。Windows 继续用 Job Object 管理提权句柄。两端在 controller 就绪后读取 `/configs` 核对 TUN 真实生效状态。用户交互启动中拒绝授权、缺少服务/wintun/polkit 或 TUN 未生效时，客户端自动关闭 TUN、重新生成配置并以普通权限恢复内核；登录自启的无交互例外见 6.6。

服务以 root 运行意味着 sing-tun 的 `resolvectl` 调用和 TUN 设备管理天然具备权限，无需任何 shim 或 capabilities 补丁。实际 DNS 接管对齐 Clash Verge Rev 的工作配置，由 Mihomo 的双栈 fake-IP、`dns-hijack: any:53` 与 auto-route 完成；桌面进程不使用 `nmcli device modify` 二次修改刚创建的虚拟网卡，只经 controller 核对 TUN 状态，避免路由切换期间引入 DNS 竞态。

开启 TUN 时本地基线注入 `gvisor` 栈、`auto-route`、自动网卡检测、DNS 劫持和完整双栈 fake-IP DNS；设备名与 TUN 地址沿用 Mihomo 默认值，避免偏离锁定内核与 Clash Verge Rev 已验证的 Linux 行为。关闭时移除客户端 DNS 注入并禁止订阅自行开启 TUN。检测到其他 Mihomo `Meta` TUN 设备时直接拒绝启动；同一时间运行多个 TUN 客户端仍可能争用策略路由表，不属于支持场景。休眠、网络切换、服务升级/卸载和发行包级恢复仍需在各目标桌面做进一步验证。

### 6.4 系统托盘与窗口生命周期

当前 GPUI 没有系统托盘 API，Windows 目标使用 `tray-icon 0.24.2`，Linux 目标使用 `ksni 0.3`（纯 Rust 的 StatusNotifierItem/DBus 实现，不依赖 GTK 和主线程）。托盘对象与长期业务实体由应用级 `AppShell` 持有，不依赖主窗口是否存在。GPUI 使用 `QuitMode::Explicit`：关闭最后一个窗口只销毁原生窗口、渲染资源和窗口句柄，不结束应用事件循环，因此 Mihomo、后台轮询、托盘和 `AppShell` 继续存活。托盘菜单“打开”、Windows/Linux 第二实例唤起以及 macOS Dock reopen 统一调用 `AppShell::open_or_focus_main_window`，在无窗口状态下把同一个业务实体挂到新窗口，避免重复启动内核或重复创建轮询任务。

Linux 桌面对左键语义不统一：SNI 规范下普遍弹出菜单（KDE 可能触发 Activate），因此菜单“打开”项是主入口，Activate 同时接到同一动作。GNOME 需要 AppIndicator 扩展；顶栏不展示 tooltip 时状态由 SNI Title 承担。Wayland 依赖 `xdg_activation`，部分合成器可能拒绝托盘来源的激活请求；X11 可正常恢复并激活新建窗口。

悬浮提示按当前语言展示内核、系统代理和 TUN 的真实状态，并在内核启停、系统代理/TUN 开关及语言切换后同步更新。

### 6.5 单实例

Windows 在读取配置和启动 GPUI 前创建当前会话范围的 `Local\\` 命名 Mutex。首实例持续持有 Mutex；用户主动启动的后续进程发现对象已存在后，设置同一命名的自动重置 Event 并立即退出，登录自启的后续进程则不发送 Event。因此 debug、release、安装版和升级后的可执行文件使用相同应用身份，不会并行维护同一份配置或启动多份内核。

首实例使用独立等待线程监听 Event，并通过有界异步通道把请求交给 GPUI 主线程；收到请求后调用 `AppShell` 恢复已有窗口，或在零窗口状态下创建新窗口并激活。自动重置 Event 可在首实例窗口尚未完成创建时保存一次并发启动信号，有界通道则合并短时间内的重复请求。应用退出时通过私有关闭 Event 唤醒并回收等待线程。

Linux 使用抽象命名空间 Unix domain socket（名称按 UID 隔离多用户）：内核保证 `bind` 的原子性，监听成功即为首实例；用户主动启动的后续实例 `connect` 到同名 socket 通知首实例后立即退出，连接建立本身即激活请求，登录自启的后续实例只确认地址已占用并静默退出。接收线程以非阻塞轮询消费连接（关闭 `try_clone` 副本无法唤醒阻塞 accept），退出标志加 join 保证应用退出时回收线程；抽象 socket 随进程退出自动释放，无文件残留。macOS 尚未实现同等单实例锁，但 Dock reopen 已接入 `AppShell` 的窗口重建路径。

### 6.6 登录自启

设置页直接读取和修改平台登录项，不在 `app.json` 保存第二份布尔状态。Windows 使用当前用户 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 下的 `PureClash` 字符串值，命令为带引号的当前 EXE 绝对路径加 `--autostart`，无需管理员权限；NSIS 卸载时删除该值。Linux 按 XDG Autostart 规范维护 `$XDG_CONFIG_HOME/autostart/pure-clash.desktop`，变量缺失或不是绝对路径时回退到 `~/.config/autostart`；desktop entry 的 `Exec` 参数按规范引用和转义。AppImage 的 `/proc/self/exe` 指向临时挂载目录，因此注册时优先使用 AppImage 运行时提供的 `$APPIMAGE` 原始绝对路径；文件被移动或改名后用户需要重新开关自启。macOS 本阶段不实现。

`--autostart` 首实例创建长期业务实体、托盘并按持久配置启动内核，但不创建主窗口。Windows 恢复已配置 TUN 时允许弹出 UAC；用户拒绝授权或启动失败后，客户端关闭 TUN 配置并自动拉起普通内核。Linux 只有版本匹配且 Ping 成功的既有服务允许在后台静默启动 TUN；服务未安装、版本不匹配或不可达时不弹 polkit，同样关闭 TUN 配置并回退普通内核。后台模式若托盘初始化失败会恢复系统代理、回收内核并退出，避免留下不可操作的零窗口进程。

TUN 的持久配置与运行状态必须分离：`local.yaml` 中的 `tun.enable` 只表示下一次启动意图，界面和托盘只有在内核进入 Running 且 controller `/configs` 确认 TUN 生效后才显示开启；启动中、停止、失败或重启期间一律显示关闭。这样不会出现内核未启动但 TUN 显示开启的伪状态。

单实例请求区分来源：普通第二实例发送激活信号并唤起首实例窗口，`--autostart` 第二实例只确认已有进程后静默退出。因此先由登录项启动的后台实例仍可在用户稍后手动运行程序时创建窗口，已经打开的应用也不会被登录项重复打扰。

实现依据：[Windows Run/RunOnce](https://learn.microsoft.com/en-us/windows/win32/setupapi/run-and-runonce-registry-keys)、[XDG Autostart](https://specifications.freedesktop.org/autostart-spec/latest/)、[Desktop Entry Exec](https://specifications.freedesktop.org/desktop-entry-spec/latest/exec-variables.html)、[AppImage 运行时变量](https://docs.appimage.org/packaging-guide/environment-variables.html)。

## 7. 内核供应链与许可证

Pure Clash 项目自身代码以 GPL-3.0 发布：许可文本只有仓库根目录 `LICENSE` 一份，Cargo 声明 `license = "GPL-3.0"`；对外分发（安装器、便携包）须随附该文本。

不在应用启动时盲目跟随 GitHub latest。项目仓库和安装包维护受控的 runtime manifest，顶层记录版本、源码与许可证信息，`targets` 按编译目标记录各平台发行条目：

```text
version / source_url / license / license_url / license_sha256
targets.<os-arch>：
  platform / architecture / build / binary
  archive / download URL / SHA-256 / size
  binary size / binary sha256
```

`build.rs` 按 `CARGO_CFG_TARGET_OS`/`CARGO_CFG_TARGET_ARCH` 合成目标键（如 `windows-amd64`、`linux-amd64`、`macos-amd64`、`macos-aarch64`），只要求当前编译目标的内核文件存在；Windows 安装器脚本从 `targets.windows-amd64` 读取供应链信息。下载先进入临时文件，校验大小和 SHA-256 后再解压、执行 `mihomo -v` 核对版本，并原子切换当前版本。至少保留一个已知可用版本用于回滚。

当前 NSIS 安装器从唯一的随包 manifest 读取版本并捆绑 Windows amd64-compatible 构建，安装目录为 `kernel\<manifest.version>\`，同时包含：

- 重命名后的 Mihomo `pc-mihomo.exe`、GPL-3.0 `LICENSE` 和第三方 `NOTICE.md`；
- Pure Clash 自身的 GPL-3.0 `LICENSE` 文本；
- manifest `targets.windows-amd64` 中与二进制完全对应的版本、下载 URL、文件大小和 SHA-256；
- 上游版权与第三方 notices；
- 与二进制完全对应的源码获取 URL或书面提供方式；
- Pure Clash 与 Mihomo 独立许可证和非官方关系说明。

`packaging/windows/build-installer.ps1` 会在构建前检查内核、许可证与 manifest 是否齐全，再将它们交给 NSIS 打包。卸载时删除随包内核，但保留用户的 `config/` 和 `data/`。Linux 按便携式布局直接使用源码树内的 `kernel/<版本>/pc-mihomo`；发布流水线在推送 `v*` 标签时构建 deb/rpm（安装到 `/opt/pure-clash` 并软链 `/usr/bin`）与 AppImage，均随包内核、许可证与 manifest，供应链校验与 Windows 共用同一份 manifest。

## 8. 错误与隐私模型

- **配置错误**：保留当前内核与可用配置，展示 Mihomo 原始错误的脱敏版本。
- **controller 错误**：区分鉴权失败、连接失败、超时、非 2xx 和响应解析失败。
- **进程错误**：记录退出码及有限长度 stderr；短时间连续崩溃后停止自动重启。
- **系统代理错误**：不宣称已启用；提供当前值、期望值和恢复操作。
- **订阅错误**：不覆盖 `active.yaml`，保留上次成功更新时间。

默认日志禁止包含订阅 URL 查询参数、`Authorization`、`Proxy-Authorization`、controller secret、节点密码和完整配置正文。诊断导出前再次执行脱敏，并让用户预览文件列表。

## 9. 分阶段实施

### Phase 0：迁移基线（已完成）

- 项目品牌、Cargo、安装器、README、AGENTS 和技术方案迁移为 Pure Clash。
- GPUI 页面骨架与导航可编译运行。

### Phase 1：Mihomo 控制 Spike（已完成）

- 默认运行时路径、`-t` 校验、进程启停；Windows Job Object、Linux pdeathsig 兑底异常退出。
- loopback controller 接入：版本、配置、代理组、运行模式、订阅下载与激活。
- 用 fixture 测试 API 解析，用真实 Mihomo 做可选 smoke test。

### Phase 2：桌面 MVP（当前阶段）

已完成：总览/代理/连接/配置/设置五页真实数据；URL 订阅下载与本地 YAML 导入、更新、删除和激活；节点选择与模式切换；连接列表与实时流量（1 秒轮询差分）；连接关闭；单节点与整组延迟测试；Windows/Linux 系统代理托管与自愈；托盘、单实例、登录自启与国际化。

待实现：规则页、日志/内存监控、订阅历史回滚、内核下载与版本回滚。

### Phase 3：Beta

- provider 管理、连接筛选、诊断导出。
- 崩溃恢复、睡眠唤醒、网络变化、配置热重载和自动更新。
- 代码签名、发行流程和干净环境验证。

### Phase 4：TUN 与跨平台评估（Windows/Linux 基础链路已完成）

- Windows 已通过 UAC/wintun、Linux 已通过一次授权 root systemd 服务（对齐 Clash Verge Rev 服务模型）与 Mihomo 双栈 fake-IP 接入 TUN，并具备 controller 生效核对和自动回退。
- 继续补齐休眠/网络切换、Linux 发行包和不同桌面环境验证；macOS 尚未实现。

## 10. 发布验收

- 配置错误不会破坏最后可用配置。
- controller 仅监听 loopback，缺失或错误 secret 无法访问。
- 代理组、规则、连接、流量和日志来自真实内核而非 mock。
- 开关系统代理前后可正确保存与恢复原值，异常启动能检测遗留状态。
- 应用正常退出、崩溃和强制结束后不残留 Mihomo 进程。
- 安装、升级、回滚和卸载在干净 Windows x64 环境验证通过。
- 发行包包含准确的版本、哈希、许可证和源码获取信息。
