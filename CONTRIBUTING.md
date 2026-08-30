# 参与贡献

感谢关注 Pure Clash。本文档说明开发环境、代码约定与提交规范。

## 关于 AI vibe coding

Pure Clash 自身包含大量 AI vibe coding 产出，根目录的 [AGENTS.md](AGENTS.md) 是项目维护的 AI 编码代理工作约定。我们接受并欢迎以 vibe coding 方式产出的内容提交；无论由人还是由代理编写，提交都需要满足本文档的验证流程与代码约定，并由提交者对改动负责。

## 环境准备

- Rust stable（Windows 需 MSVC 工具链）
- Linux 需要 Wayland 或 X11 会话及对应开发库；TUN 调试需要 `pkexec` 与 polkit
- PowerShell 7 + NSIS 3.x（仅打包 Windows 安装包时需要）

随包内核已提交到 `kernel/<版本>/`，clone 后无需额外下载。

## 构建与验证

提交前必须全部通过：

```bash
cargo fmt --check
cargo check
cargo test
cargo build
```

测试约定：解析与格式化逻辑使用 fixture 单测；需要真实内核、网络或桌面会话的测试标记 `#[ignore]` 并说明原因，不得让默认 `cargo test` 依赖外部环境。

## 代码约定

- 界面文案与代码注释使用中文；协议字段、类型名和函数名保留英文。
- 所有用户可见文案必须进入 `locales/zh-CN.yml` 与 `locales/en-US.yml`，两个文件键保持对齐；不得在渲染代码硬编码业务文案。
- Mihomo 的 API、CLI 与配置字段一律以 [官方文档](https://wiki.metacubex.one/) 和当前锁定内核（`kernel/<版本>/manifest.json`）实测为准，不凭记忆补字段；新增接口前先用随包内核实测响应结构，并把实测响应固化为解析测试 fixture。
- 平台差异集中在 `src/platform/mod.rs` 与对应平台的 `windows/`、`linux/` 子模块；`mihomo` 模块保持平台无关。
- GPUI 锁定 `0.2.2`，升级前必须按目标版本官方示例核对 API。
- 日志与错误输出不得包含订阅 URL、认证头、controller secret 与节点凭据；配置校验的原始错误只展示给当前用户。
- 保持单包、小依赖：新增依赖前先确认无法用现有依赖与标准库合理实现。

## 提交规范

提交信息使用 Conventional Commits 风格（`feat:` / `fix:` / `refactor:` / `docs:` 等），正文说明动机与关键改动；一次提交聚焦一件事。

## 许可证

Pure Clash 以 GPL-3.0 发布（见根目录 [LICENSE](LICENSE)）。提交即表示同意代码以该许可证随项目分发；随包 Mihomo 内核的上游许可证义务见 `kernel/<版本>/` 内的 `LICENSE` 与 `NOTICE.md`。
