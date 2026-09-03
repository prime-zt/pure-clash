# Pure Clash

## 项目目标与边界

- 使用 Rust 和 Zed GPUI 构建轻量、原生的 Mihomo 桌面客户端，产品名为 `Pure Clash`，Cargo 包名为 `pure-clash`。

- 面向 Windows 11 x64 + MSVC 与 Linux x64，负责配置管理、代理组选择、规则与连接查看、流量监控、系统代理/TUN 控制及 Mihomo 内核生命周期。

- 不在 Rust 中重写代理内核、协议栈或规则引擎；网络转发由独立的 Mihomo 进程完成。

- 当前仓库五个基础页面（概览/代理/连接/配置/设置）与关于页均使用真实数据；连接、流量与延迟测试经 controller 实时接入。

## 核心模块

- `src/main.rs`、`src/startup.rs`：应用入口、启动参数模式、快捷键和原生窗口创建；`--autostart` 用于登录后零窗口后台启动。

- `src/app/`：界面模块。`mod.rs` 持有 Pure Clash 状态与业务逻辑（内核生命周期、系统代理/TUN、订阅管线、后台任务与共享渲染助手），`shell.rs` 的 `AppShell` 持有长期业务实体、托盘和可重建的主窗口句柄；渲染按区域拆分子模块：`frame.rs`（标题栏、状态徽标、窗口按钮、Linux 客户端装饰）、`sidebar.rs`（侧栏与内核卡片）、`header.rs`（页面路由与页头开关芯片）、`overview.rs`/`proxies.rs`/`connections.rs`/`profiles.rs`/`settings.rs`/`about.rs`（五个基础页面加关于页，含版本、源码仓库、开源组件清单与 GitHub Releases 更新检查）。子模块经 `use super::*` 访问父模块的私有状态，跨页面复用的小组件（连接行、横幅等）以 `pub(super)` 暴露。

- `src/config.rs`：`AppConfig`、程序目录初始化、`config/app.json` 读取与即时原子持久化；通过随包版本 marker 迁移仍跟随旧随包内核的配置。

- `src/platform/mod.rs`：跨平台目录、内核进程守护接口、登录自启、托盘抽象和主窗口装饰策略；各平台行为差异统一从这里解析。`src/platform/{windows,linux}/autostart.rs` 分别维护当前用户 Run 注册值与 XDG Autostart desktop entry，`src/platform/file.rs` 提供 Windows `MoveFileExW` 与 unix `rename` 的同目录原子替换。

- `src/platform/windows/job.rs`：Windows Mihomo 专用 Job Object，启用 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`。

- `src/platform/linux/child_guard.rs`：Linux 普通内核进程守护，通过 `PR_SET_PDEATHSIG` 保证主进程异常退出时内核被回收，由 `KernelProcessGuard` 调用。

- `src/platform/linux/tun_service.rs`：Linux TUN 一次授权服务的安装/卸载、按 UID 隔离的长度前缀 JSON IPC、runtime bundle 物化与服务端 root 内核生命周期；服务先在同一状态目录 staging 完整 bundle 并执行 `mihomo -t`，通过后才停止旧内核和原子切换 runtime，新内核启动失败会恢复备份并尽力重启旧内核。服务版本与协议、随包内核版本共同决定是否刷新 root 副本。

- `src/platform/linux/elevation.rs`：Linux TUN 服务客户端，把 `launch_elevated`/`ElevatedProcess` 映射到服务 IPC，并在 GPUI 与单实例初始化前分流安装器与 systemd 服务模式。

- `src/platform/{windows,linux}/process_guard.rs`：各平台同名 `KernelProcessGuard`，统一内核子进程的守护（Job Object / pdeathsig）与终止策略（TerminateProcess / SIGTERM→SIGKILL），`mihomo` 模块保持平台无关。

- `src/platform/windows/single_instance.rs`：Windows 当前会话单实例锁；后续启动只通知首实例恢复并激活主窗口。

- `src/platform/tray.rs`：平台无关托盘抽象，定义 `TrayAction` 并按平台导出同名 `SystemTray`。

- `src/platform/windows/tray.rs`：Windows 系统托盘，复用 EXE 应用图标，转发单击与右键菜单事件并动态更新运行状态提示。

- `src/platform/linux/tray.rs`：Linux 托盘，基于 ksni（SNI/DBus 纯 Rust 实现）提供图标、菜单和状态文本，与 Windows 行为对齐。

- `src/platform/linux/window_ctrl.rs`：Linux 主窗口重建后的显示与激活，衔接 Wayland/X11 差异。

- `src/platform/windows/window_ctrl.rs`：Windows 主窗口重建后的显示与激活，支撑托盘唤起。

- `src/mihomo/process.rs`：使用 `-t` 校验默认配置，按 `-d` / `-f` 启停真实内核并回收子进程；平台进程管理全部委托 `platform::KernelProcessGuard`，模块内不含平台分支；普通内核的 stdout/stderr 经泵线程逐行写入 kernel.log（打不开日志也持续排空管道）。

- `config/mihomo/default.yaml`：嵌入程序的首次启动默认配置，仅包含一个内置 `DIRECT` 节点。

- `src/logging.rs`：零框架文件日志（`jiff` 仅提供本地时区时间戳）。运行日志 app.log 与内核日志 kernel.log 分文件、各自按大小轮转为 `.1`（1MB+3MB 单文件上限，磁盘合计约 8MB）；宏 `log_error!`/`log_warn!`/`log_info!`/`log_debug!` 以标签区分模块，所有消息写入前经 `redact` 脱敏（干净行零分配借用原文）；panic hook 记录 panic；初始化失败降级为空日志绝不阻断启动。写入路径按频率区分：app.log 低频用 `LineWriter` 逐行落盘保证崩溃诊断不丢尾，kernel.log 高频用 `BufWriter` 批量落盘（停止内核时等待泵线程排空并冲刷），轮转判断用累计字节计数而非每行查询文件元数据。

- `src/profile.rs`：URL 订阅和本地配置文件的读取、大小/编码限制、结构校验、落盘与运行时配置同步管线。

- `src/mihomo/config.rs`：客户端本地基线（端口/controller/secret）、订阅合并与结构预检。

- `src/mihomo/controller.rs`：external controller REST 客户端（版本/模式/代理组）与订阅下载。

- `src/mihomo/geodata.rs`、`geodata/`：随包 GeoSite/GeoIP/MMDB 数据的清单校验、首次离线安装、完整性状态和设置页官方更新事务。

- `src/ui/`：GPUI 缺失的通用控件（自绘单行文本输入，参照官方 input 示例实现）。

- `locales/`：`rust-i18n` 编译期加载的简体中文与英文界面资源。

- `src/theme.rs`：浅色/深色调色板、字重和透明色辅助 trait。

- `src/assets.rs`、`assets/icons/`：嵌入式 SVG 资源注册与本地图标；`app.svg` 是带背景的应用图标源文件。

- `build.rs`、`src/kernel.rs`、`kernel/{版本}/`：从随包 manifest 注入默认内核版本，并按启动配置解析运行时路径。

- `packaging/windows/`：Windows 专属的 NSIS 安装器定义和 PowerShell 7 打包入口。

- `packaging/linux/`：Linux 发行包资源（desktop 条目、图标、deb 维护脚本、rpm scriptlet、AppImage 组装脚本）；deb/rpm 元数据在 `Cargo.toml` 的 `package.metadata.deb` / `generate-rpm`，安装布局为 `/opt/pure-clash` + `/usr/bin` 软链，内核版本目录升级时同步两段 assets。完整卸载会在程序文件移除前调用内部 root 服务清理入口，升级事务不会删除软链或服务。

- `.github/workflows/release.yml`：推送 `v*` 标签触发的发布流水线，构建 Windows NSIS 与 Linux deb/rpm/AppImage 并发布 GitHub Release；标签版本与 Cargo 版本不一致时直接失败。构建 job 默认只有 `contents: read`，仅发布 job 可写；外部 AppImage 工具固定不可变版本并校验官方 SHA-256。

- `docs/pure-clash-architecture.md`：Mihomo 进程、REST/WebSocket 控制、安全、配置和产品化技术基线。

## 技术栈、目录与约定

- Rust 2024 edition；GPUI 使用 Zed `v1.17.2` 对应提交 `c8e44cfa7bda9b2e22c8d6934d78969352e7f61a`，平台后端使用同提交的 `gpui_platform`；`rust-i18n = 4.2.1`；Windows 托盘使用 `tray-icon = 0.24.2`；unix 目标使用 `libc` 发送 SIGTERM 与设置父进程死亡信号；非 Windows 目标使用 `directories = 6.0` 解析标准用户目录。

- 当前 Cargo 包版本为 `0.2.4`；正式发布标签必须使用匹配的 `v0.2.4`，否则发布流水线会拒绝构建。

- UI、业务说明和代码注释使用中文；协议字段、类型名和函数名保留英文。

- Cargo/可执行文件/安装包前缀统一为 `pure-clash`，界面和 Windows 发行名统一为 `Pure Clash`。

- 保持单包、小依赖；平台无关路径留在 `src/platform/mod.rs`，只有实际接入系统代理、凭据、进程监管等能力时才新增 `src/platform/windows/`、`linux/` 或 `macos/` 子模块。

- 所有 Mihomo API、CLI 和配置字段均以官方文档及当前锁定内核版本为准，不凭记忆补字段。

## 常用命令

- 运行：`cargo run`

- 检查：`cargo check`

- 测试：`cargo test`

- 格式化：`cargo fmt --check`

- 发布构建：`cargo build --release`

- NSIS 打包：`pwsh -NoLogo -NoProfile -File .\packaging\windows\build-installer.ps1`

## 架构决策与限制

- Mihomo 作为独立 sidecar 运行；Pure Clash 通过仅监听 loopback 的 REST/WebSocket external controller 通信，并为每次安装生成高强度随机 secret。

- 客户端只接受本机 controller，不开放局域网控制；日志与诊断信息必须脱敏，不记录订阅 URL、认证头或 controller secret。

- 日志分两个文件：`log/app.log` 记录客户端运行日志，`log/kernel.log` 记录普通内核的 stdout/stderr 原始输出；Windows 日志目录在可执行文件同级 `log/`，Linux 按 XDG 放 `~/.local/state/pure-clash/log/`（`AppPaths.log_dir` 统一解析，macOS 回退本地数据目录）。单文件超阈值（app 1MB、kernel 3MB）轮转为 `.1` 覆盖旧备份，磁盘占用合计约 8MB；打开文件时总是先把上一段归档，app.log 即本次会话、kernel.log 即当前内核的输出。日志在单实例判定后、配置加载前初始化，次实例不写日志；初始化失败降级为空日志，绝不阻断启动；panic hook 把 panic 落盘。所有消息（含内核行与 `-t` 校验失败详情）写入前经 `redact` 统一脱敏：URL 只保留 scheme+host（内嵌凭据与路径/查询丢弃），`secret=`/`token=`/`password=`/`authorization:` 的值掩码。插桩遵循单层记录（app 层记用户可见操作与结果，platform/mihomo 层只记内部细节）与边沿触发（运行配置和连接两条 controller 请求各自只在转坏/恢复时记一条）；日志宏标签统一用 app/core/kernel/proxy/tun/profile/tray/controller/geodata/autostart/update/panic。提权内核（Windows UAC / Linux systemd 服务）的 stdout 不经客户端捕获：app.log 记录生命周期与失败原因，Windows 提权内核输出不落 kernel.log，Linux TUN 内核输出在 systemd journal。

- 启动真实内核前先用目标版本的 Mihomo 校验配置；校验失败不得替换当前可用配置或重启正在工作的内核。

- 内核子进程的平台守护与终止统一经 `platform::KernelProcessGuard`（`new`/`prepare_command`/`attach`/`terminate`/Drop 约定）：Windows 用 Job Object 与 TerminateProcess，Linux 用 pdeathsig 与 SIGTERM→5 秒→SIGKILL，unix 内核独立成进程组。Windows 的 UAC `ShellExecuteExW(runas)` 必须在专用 STA 线程执行，禁止在 GPUI 实体更新回调中同步弹 UAC，以免 Shell 嵌套消息循环重入 `App`；启动结果带代次回传，过期进程必须立即回收。Linux 启动必须保持在长寿命线程（当前为 GPUI 主线程），避免创建子进程的短寿命线程退出触发 pdeathsig。macOS 仅预留进程组隔离与优雅终止，异常退出兑底在正式支持前单独实现。`mihomo` 模块不得直接依赖平台 API。

- 系统代理和 TUN 是两条独立能力。系统代理由客户端实现：Windows 写 HKCU Internet Settings 并经 InternetSetOption 立即生效；Linux GNOME/Cinnamon 兼容会话通过 `gsettings` 管理 HTTP/HTTPS/FTP/SOCKS，其他桌面明确返回不支持；macOS 暂不支持。启用前捕获用户既有设置存入 `data/system-proxy.json`，关闭时恢复，异常退出后下次启动按该文件自愈；状态文件必须先原子落盘再改系统设置，修改失败需自动恢复；停内核（手动停止/退出/内核失联）时自动恢复代理设置，配置切换重启内核不中断托管。TUN 由内核实现：开关写入本地基线 `tun_enable` 并重新合并 `-t` 校验、运行中自动重启内核；内核未运行时拒绝开启，手动停止内核时 TUN 经 `revert_tun` 同步回退（状态、基线与 runtime 一起关闭并持久化），配置切换重启与托盘退出不改变 TUN 基线（下次启动如实恢复）。Windows 经 `runas`（UAC 弹窗）以管理员权限启动 Mihomo。Linux 首次经 `pkexec` 安装 root 所有的 systemd 服务与锁定内核副本，后续通过按 UID 隔离且校验 `SO_PEERCRED` 的 Unix socket 启停，不再重复授权；对齐 Clash Verge Rev 的服务模型，服务保持 root 直接启动 Mihomo，不降权、不设置 ambient capabilities、不替换系统网络工具。GUI 不向服务传内核路径，只提交 runtime bundle（YAML + 本地资源 + 远程 provider 列表）；服务原子物化到 `/var/lib/pure-clash-service/users/<uid>/runtime`，拒绝未开启 TUN、非回环监听或资源路径越界的 bundle，先以锁定内核 `-t` 校验再启动，GUI PID 消失时自动回收内核。拒绝授权或启动失败即回退关闭 TUN 并以普通权限重启；内核就绪后仅经 controller `/configs` 核对 TUN 真实生效状态，不做外部公网探测，未生效即自动回退关闭、重启内核并在概览/设置页提示原因。Windows 的 wintun.dll 随包捆绑在内核版本目录（manifest `targets.windows-amd64.wintun` 记录版本、官方来源与双重 SHA-256，build.rs 校验存在性，NSIS 随内核一起安装/卸载）。Linux TUN 配置对齐同机 Clash Verge Rev 的已验证结构：使用 `gvisor`、`auto-route`、`dns-hijack: any:53`、Mihomo 默认设备/地址和完整双栈 fake-IP DNS；关闭 TUN 时 `ipv6` 回到 false、不注入 DNS，保留订阅自带配置。检测到其他客户端的 `Meta` TUN 设备时拒绝并行启动。

- Windows 配置与数据继续存放在可执行文件同级的 `config/` 和 `data/`，主配置为 `config/app.json`；Mihomo 默认配置为 `config/mihomo/default.yaml`，运行数据目录为 `data/mihomo/`。安装目录必须对当前用户可写，NSIS per-user 安装符合该约束。Linux/macOS 预留使用 `directories` 提供的标准用户配置和数据目录，避免写入只读系统安装目录。

- `default.yaml` 通过 `include_str!` 嵌入程序，只在目标文件缺失时创建，不得覆盖用户修改；默认监听本机 `127.0.0.1:7890`，启用 `unified-delay` 与 `tcp-concurrent`，策略组只有 Mihomo 内置 `DIRECT` 出站。这两个行为字段同时由本地基线强制注入所有 runtime，订阅不得关闭。设置页"强制匹配进程名"开关（`AppConfig.find_process_always`，存 app.json，是唯一事实来源）开启时本地基线向 runtime 注入 `find-process-mode: always`，关闭时不注入、保留订阅自带值（内核默认 strict 只在规则需要时查找进程，无 PROCESS 规则的订阅连接页进程名恒为空）；启动时把开关对齐到基线（local.yaml）再生成 runtime，切换开关走 `-t` 终审 + 基线/runtime 原子落盘事务，运行中的内核经 controller `PATCH /configs` 热切换（不重启、不断连）；PATCH 异步失败时按切换前快照完整回滚四处状态（内存开关、app.json、local.yaml、runtime.yaml）并在设置页提示"已恢复原设置"，保证界面显示与内核实际运行模式一致。

- 随包内核统一重命名为 `pc-mihomo`（Windows 为 `pc-mihomo.exe`），避免与其他代理客户端的 Mihomo 进程重名。内核文件随仓库提交：Windows 与 Linux x64 已入库，macOS 二进制暂不入库，需按 manifest 锁定的下载地址与 SHA-256 手动放置。macOS 资源根目录预留为 `.app/Contents/Resources/kernel`；Linux deb/rpm 将程序和资源安装到 `/opt/pure-clash`，AppImage 按可执行文件旁布局解析。Linux 自动补齐内核可执行位；macOS 要求文件预先具备可执行权限。

- `kernel/{版本}/manifest.json` 是随包版本和各平台运行时文件名的唯一来源：顶层记录版本、源码与许可证信息，`targets` 按编译目标（如 `windows-amd64`、`linux-amd64`、`macos-amd64`、`macos-aarch64`）记录二进制名、官方下载地址、大小和 SHA-256。构建与打包只要求当前编译目标的内核文件存在，`build.rs` 与安装器脚本各自从 `targets` 读取对应条目，要求目录名与 `version` 一致，`binary` 必须是安全的单文件名。

- `geodata/manifest.json` 是随包 `GeoSite.dat`、`GeoIP.dat`、`Country.mmdb` 的唯一来源，锁定 MetaCubeX/meta-rules-dat `release` 分支同一 commit、大小、SHA-256 与 GPL-3.0 许可证。`build.rs`、Windows 打包脚本及 Linux 发行 assets 均携带清单、三份数据、LICENSE 与 NOTICE；资源目录为 Windows/Linux 的可执行文件同级 `geodata/`，macOS 预留 `.app/Contents/Resources/geodata/`。

- 启动时把随包 Geo 数据原子安装到 `mihomo_data_dir`，以 `.pure-clash-geodata.json` 记录来源提交、大小和哈希；已完整在线更新的数据不得被较旧随包版本覆盖，缺失或损坏时离线恢复整套随包快照。配置/订阅校验只检查本地数据，不得隐式联网下载；设置页手动更新先查询官方 release commit，再从该 commit 下载完整三件套并带回滚提交，成功后重启正在运行的内核。

- Windows 安装器只安装和卸载当前 manifest `targets.windows-amd64` 中的 `pc-mihomo.exe`，升级时不主动清理历史安装遗留的 `mihomo.exe`。

- `AppConfig` 使用 serde 默认值兼容缺失字段；`mihomo_version` 默认由构建脚本从随包 manifest 注入，`bundled_mihomo_version` 记录上次随包版本：仍跟随随包版本或所选内核已经不存在时自动迁移到当前随包版本，实际存在的手动选择予以保留。主配置、Mihomo 基线、默认配置、profile 和 runtime 均使用同目录临时文件原子替换；`theme` 支持 `dark` / `light`，首次启动默认 `light`，界面修改后立即写回。

- `AppConfig.language` 使用 `zh-CN` / `en-US`，默认 `zh-CN`；切换语言后立即调用 `rust_i18n::set_locale` 并持久化。界面文案必须加入两个 locale 文件，不得继续在渲染代码中硬编码业务文案。

- 后续敏感凭据仍应使用 Windows Credential Manager 或 DPAPI，不写入日志和普通 JSON/YAML。

- Pure Clash 项目自身代码以 GPL-3.0 发布：根目录 `LICENSE` 是唯一许可文本，Cargo 声明 `license = "GPL-3.0"`，修改代码不得移除或弱化许可证声明。随包 Mihomo 内核同样使用 GPL-3.0，义务经 `kernel/<版本>/` 目录内的 `LICENSE`、`NOTICE.md` 与 `manifest.json` 源码地址履行，安装器随内核一并安装。`Pure Clash` 名称不包含上游限制的 `mihomo` 字样。

- 使用 `include_bytes!` 和自定义 `AssetSource` 嵌入 SVG；Windows 无边框窗口使用 GPUI `WindowControlArea` 和自绘窗口按钮；Linux Wayland 使用参考 Zed 的客户端装饰，包含窗口按钮、拖动、缩放、圆角和阴影，X11 不支持时回退系统装饰；macOS 保留原生标题栏。

- Windows 应用图标由 `assets/icons/app.svg` 派生为 `assets/windows/pure-clash.ico`；ICO 包含多分辨率帧，并统一用于 GPUI 自绘标题栏、EXE 资源、快捷方式及 NSIS 安装器/卸载器。

- 托盘在 Windows 和 Linux 上提供一致体验：单击图标或菜单“打开”显示并激活主窗口，悬浮提示/状态文本按当前语言同步内核、系统代理和 TUN 的真实状态，并在相关开关变化后立即更新。主窗口关闭会真正销毁原生窗口及窗口渲染资源，但 `AppShell`、托盘、业务实体和内核继续运行；托盘菜单“打开”、第二实例唤起以及 macOS Dock reopen 会重新创建并激活窗口，托盘菜单“退出”先恢复系统代理、回收内核再真实结束应用。Linux 差异：SNI 桌面普遍把左键用于弹出菜单（KDE 等会触发 Activate）；GNOME 需要 AppIndicator 扩展，且顶栏不显示 tooltip，状态改由 SNI Title 承担；Wayland 依赖 xdg\_activation，部分合成器可能拒绝托盘来源的激活请求。

- Windows/Linux 登录自启以平台真实状态为准，不写入 `app.json`：Windows 使用 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 的 `PureClash` 值，Linux 使用 `$XDG_CONFIG_HOME/autostart/pure-clash.desktop`（缺省 `~/.config/autostart`），命令统一携带 `--autostart`。登录启动只省略主窗口，业务实体、托盘和内核仍必须启动；自启次实例静默退出，用户主动启动的次实例仍唤起首实例窗口。AppImage 必须优先记录运行时 `$APPIMAGE` 原始绝对路径，文件移动后需重新开关自启。后台模式托盘初始化失败时必须清理内核并退出，避免不可操作的隐形进程。Windows 恢复已配置 TUN 时允许弹 UAC；Linux 只经已安装且版本匹配的服务静默启动 TUN，服务不可用时关闭 TUN 配置并自动回退普通内核，不在登录阶段弹 polkit。TUN 状态只在内核运行且 controller 确认生效后显示开启，内核停止、启动中或失败时必须显示关闭。Windows 卸载器必须删除 Run 值；macOS 暂不实现登录自启。

- 当前用户会话只允许一个 Pure Clash 实例：Windows 用 `Local\\` 命名 Mutex + 自动重置 Event，Linux 用抽象命名空间 Unix socket（按 UID 隔离多用户，内核保证 bind 原子性，进程退出自动释放）；用户主动启动的后续进程通知首实例后于配置初始化和 GPUI 启动前退出，首实例把通知转交 GPUI 主线程恢复并激活主窗口，携带 `--autostart` 的后续进程则静默退出。macOS 尚未实现对应单实例锁。

- 当前锁定 GPUI 的 `svg()` 元素按单色 alpha mask 绘制且必须设置 `text_color`；带背景色的 `app.svg` 在标题栏中必须使用 `img()`，其他单色界面图标继续使用 `svg()`。GPUI 应升级前按目标提交的官方示例核对 API。

- Windows release 使用 GUI 子系统，debug 保留控制台；NSIS 继续采用 per-user 安装模型。TUN 等提权能力不得借此安装器静默获取管理员权限。

- 当前实现的平台能力：Windows x64 支持内核启停、系统代理、TUN、托盘、单实例与当前用户登录自启；Linux x64 支持内核启停（异常退出由 pdeathsig 回收）、GNOME/Cinnamon `gsettings` 系统代理、基于 polkit 一次授权 root systemd 服务的 TUN（对齐 Clash Verge Rev 的服务模型）、SNI 托盘（GNOME 需 AppIndicator 扩展）、单实例与 XDG Autostart，其中 Linux TUN 已在 Fedora 44 x64（Wayland/GNOME）完成真实路由/DNS 授权验证，登录自启和其他桌面/发行版尚未完成真机验证。Linux 凭据存储仍未实现，deb/rpm/AppImage 由 GitHub Actions 在推送标签时构建并发布 Release；macOS 只完成目录、资源路径与窗口装饰边界预留，不支持登录自启。

- 页面上的内核启动/停止已接入真实 Mihomo 进程；启动固定使用 `config/mihomo/runtime.yaml` 和 `data/mihomo/`，并先以同一内核执行 `-t`。应用启动时按 `app.json` 记录的激活配置自动拉起内核；runtime 合并、`-t` 或原子提交任一步失败时保留旧文件且不得启动/重启内核，profile 激活态只在 runtime 提交成功后更新。runtime.yaml 由客户端本地基线 `config/mihomo/local.yaml`（含随机 controller secret，禁止写入日志）与激活的配置文件合并生成，端口等本机字段一律以本地基线为准，订阅不得开启 TUN。配置页首行是内置默认配置（仅 `DIRECT` 出站，`active_profile` 为空即选中态，点击经同一 `-t` 链路切回），其下支持 URL 订阅下载以及 GPUI 原生文件选择器导入本地 YAML；两种来源共用结构预检、同版本内核 `-t`、原子保存与激活链路，远程订阅另支持更新，所有配置均可删除与切换。导入只保存内容副本，不记录源路径或复制其相对引用资源。激活/切换配置会真实重启内核；代理页与运行模式切换通过仅回环的 external controller（`PATCH /configs`、`GET/PUT /proxies`）真实生效，节点/分组延迟测试用 `GET /proxies/{name}/delay` 与 `GET /group/{name}/delay`（gstatic 204 探测、5 秒超时，手动结果覆盖 /proxies 历史值，失败显示超时）。连接与流量为真实实现：应用常驻任务每秒轮询 `GET /connections`，差分累计字节数得到实时网速，连接列表支持单条与全部关闭（`DELETE /connections[/{id}]`），渲染上限 200 行；响应中 `connections` 可为 null、失败延迟记为 0，解析时均已兜底，接口字段以官方文档和内核实测为准。设置页展示真实 controller 地址；系统代理与 TUN 开关为真实实现，标题栏以同款徽标展示两者开关状态。

- 页面借鉴 Clash Verge Rev 的功能分区，但采用独立的紧凑 GPUI 原生设计；页面保留内核开关、模式切换、代理节点与延迟测试、连接、配置和系统集成状态。

- 订阅定时更新：`ProfileMeta.update_interval_minutes`（0=关闭，serde default 兼容旧 app.json；合法区间 10..=43200 分钟，debug 构建下限 1 分钟便于验证）与 `last_auto_attempt_at`（无论成败的最近尝试时间）存 app.json。到期判定 `profile::subscription_due`：`now >= max(updated_at, last_auto_attempt_at) + 间隔×60` 的墙钟比较——失败也顺延一个完整间隔防锤击，睡眠唤醒后立即补跑，时钟回拨只顺延；从未更新也从未尝试（base=0）视为到期。调度器 `spawn_profile_update_scheduler` 挂 PureClash 实体（与连接轮询同模式），主窗口关闭、`--autostart` 后台模式照常运行；首跳延迟 90 秒避开内核启动与代理自愈，之后每 60 秒一跳、每跳至多更新一个到期订阅，`profile_busy`/`auto_update_in_flight`/`CoreState::Starting` 任一成立即跳过本跳。自动更新与手动更新共用下载→`validate_and_store`（`-t` 终审、原子落盘）链路但静默执行（不占 profile\_busy、失败不弹界面错误，只在行副标题显示"上次自动更新失败"），下载期间配置被删除则按 id 定位自然丢弃。自动更新的应用策略三级递进（对齐 Clash Verge Rev）：① 下载内容与磁盘 profile 文件直接字符串比对（本地比较语义等同哈希且无碰撞），一致则只刷 updated\_at/last\_auto\_attempt\_at 并记日志，跳过落盘与 `-t` 终审、不动内核；② 有变更且命中激活订阅时写 runtime 后优先 controller `PUT /configs?force=true`（`payload` 字段直接携带 runtime YAML 内容，60s 超时——内核对 `path` 参数做 home/SAFE\_PATHS 安全校验，runtime 位于 `config/mihomo` 必被拒绝，payload 不受路径限制）进程内热重载——内核进程存活、既有连接不断开，v1.19.30 实测约 40ms；③ 热重载失败（或 controller 不可用）才回退 `restart_core` 整进程重启（系统代理托管不断、TUN 基线不变）；内核未运行（含启动中）只写 runtime，下次启动生效。互斥与成功标记时序：`auto_update_in_flight` 保持到应用真正结束（热重载完成或回退重启发起）才释放，热重载进行期间配置页全部动作锁定，避免晚到的热重载结果覆盖用户刚做的变更（回退重启后释放是安全的：runtime 已落盘，后续由 Starting 状态与启动代次机制接管）；`updated_at` 只在内容确定持久化后刷新——激活订阅写 runtime 失败时回滚 profile 文件（下载前读的旧内容原子写回，原本无文件则删除）并保留旧 updated\_at，否则下次到期会因下载内容与磁盘相同走跳过路径、旧 runtime 永远得不到修复。手动更新保持"写 runtime + 重启内核"的原语义不变。配置页动作互斥统一经 `profile_actions_locked`（手动忙态/行内编辑/自动更新进行中）；间隔可经添加表单的可选输入框直接设置（留空或 0 = 关闭，先于下载校验、非法值不触发下载）或订阅行内编辑器修改（复用同一 TextInput 实体，`set_content` 回填——因此添加表单打开时禁止打开行内编辑，避免回填值被添加提交读到），首次为从未更新的订阅设置间隔时以保存时刻为基准起算，避免立即触发一次意外更新。日志记 profile 标签、只含 host。

- GPUI 仍处于 pre-1.0 阶段，升级前必须按目标版本官方示例核对 API。

<br />
