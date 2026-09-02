<div align="center">
  <img src="assets/icons/app.svg" width="88" alt="Pure Clash icon" />
  <h1>Pure Clash</h1>
  <p>A lightweight native Mihomo desktop client built with Rust and Zed GPUI</p>
  <p><a href="./README.md">简体中文</a> · <a href="./README.en-US.md">English</a></p>
</div>

Pure Clash manages configuration subscriptions, proxy groups, connections and live traffic through a clean, fast native interface, and safely controls a standalone [Mihomo](https://github.com/MetaCubeX/mihomo) kernel — it does not reimplement proxy protocols or the rule engine in Rust.

Currently available for Windows x64 and Linux x64 (Wayland / X11); macOS keeps directory and resource boundaries reserved but is not yet implemented.

## Preview

| Light theme | Dark theme |
| --- | --- |
| ![Overview in light theme](docs/images/overview-light.png) | ![Overview in dark theme](docs/images/overview-dark.png) |

## Features

- **Kernel lifecycle** — configuration is validated with the same kernel build via `-t` before launch; child processes are guarded by a Job Object (Windows) or `PR_SET_PDEATHSIG` (Linux) so the kernel is reaped when the main process dies; manual start/stop plus full cleanup on tray quit
- **Profiles and subscriptions** — a built-in default profile (DIRECT-only), URL subscriptions, and local YAML imports through the native file picker; every source passes structural pre-check and kernel `-t` validation before atomic storage and activation
- **Offline Geo databases** — `GeoSite.dat`, `GeoIP.dat`, and `Country.mmdb` ship with every package, are restored locally without network access, and can be updated explicitly from Settings
- **Proxy groups and latency tests** — subscription nodes rendered per group, rule/global/direct mode switched live through the controller; per-node and per-group delay tests with threshold-based coloring
- **Connections and live traffic** — a per-second controller snapshot poll showing process, target, chain, rule and per-connection up/down bytes; close a single connection or all of them; the overview shows live speed and active connections
- **System proxy** — Windows writes the current user's Internet Settings and broadcasts via WinINet; Linux supports GNOME/Cinnamon sessions through `gsettings`. The user's original settings are saved atomically before enabling and restored on disable, core stop or crash recovery
- **TUN mode** — Windows elevates only the kernel via UAC and uses the bundled wintun; Linux follows the Clash Verge Rev service model: a one-time `pkexec` installs a root systemd service, afterwards start/stop goes through a UID-restricted IPC and never asks for the password again; TUN automatically reverts when it does not take effect
- **Desktop integration** — tray icon with live multi-language status, single-instance lock, close-to-tray, dark/light themes and a Chinese/English interface
- **Minimal memory footprint** — the desktop client itself typically stays under 50 MB of memory (excluding the Mihomo kernel)

## Pages

Five primary pages plus an About page: Overview (stat cards, current outbound, runtime status, recent connections), Proxies (groups, node selection, delay tests), Connections (live list), Profiles (subscription management), Settings (kernel and system integration, language and theme). The titlebar carries pills mirroring the system proxy and TUN switches.

## Platform support

| Capability | Windows x64 | Linux x64 | macOS |
| --- | --- | --- | --- |
| Kernel start/stop and guarding | ✅ | ✅ | Reserved |
| Profiles and proxy control | ✅ | ✅ | Reserved |
| Connections and live traffic | ✅ | ✅ | Reserved |
| System proxy | ✅ | ✅ GNOME/Cinnamon | Not implemented |
| TUN mode | ✅ UAC + wintun | ✅ polkit + root service | Not implemented |
| Tray | ✅ | ✅ SNI (AppIndicator extension needed on GNOME) | Not implemented |
| Single instance | ✅ | ✅ | Not implemented |
| Installer | ✅ NSIS per-user | ✅ deb / rpm / AppImage | Not implemented |

Linux uses XDG directories (`~/.config/pure-clash`, `~/.local/share/pure-clash`); Wayland gets client-side decorations with rounded corners, shadow and resize edges, and X11 falls back to system decorations.

## Tested environments

Real-world verification so far covers only the two setups below; other Windows versions, distros and desktop environments are untested, and feedback is welcome:

- Windows 11 x64 (MSVC build)
- Fedora 44 x64 (Wayland / GNOME)

## Getting started

### Requirements

- Windows 10/11 x64: Rust stable + MSVC toolchain
- Linux x64: Rust stable (Wayland/X11 session; TUN needs `pkexec` and a polkit authentication agent)
- PowerShell 7 + NSIS 3.x (only for building the Windows installer)

### Build and run

```bash
cargo run
```

The pinned kernel is committed with the source (`kernel/<version>/pc-mihomo.exe` on Windows, `kernel/<version>/pc-mihomo` on Linux), so a fresh clone can launch the real kernel right away. macOS binaries are not committed; download the release listed in `kernel/<version>/manifest.json` under `targets.macos-*` and verify its SHA-256.

On first launch the app initializes `config/` and `data/` on the current platform, including a DIRECT-only default config, a randomly generated controller secret, and an offline-verified copy of the bundled Geo databases.

### Packages and releases

```powershell
pwsh -NoLogo -NoProfile -File .\packaging\windows\build-installer.ps1
```

The output is `dist\pure-clash-<version>-windows-x64-setup.exe`, a per-user installation into `%LOCALAPPDATA%\Programs\Pure Clash` that never asks for administrator rights.

Linux deb / rpm / AppImage and the Windows NSIS installer are built automatically by GitHub Actions when a `v*` tag is pushed, then published to [Releases](https://github.com/prime-zt/pure-clash/releases); the tag version must match the Cargo package version. deb/rpm install into `/opt/pure-clash` and create a `/usr/bin/pure-clash` symlink, the AppImage runs as-is, and every package bundles the kernel, Geo databases, manifests, and license files.

### Development checks

```bash
cargo fmt --check
cargo check
cargo test
cargo build --release
```

GPUI is still pre-1.0; this project pins `0.2.2`. Verify the target version's official examples and changelog before upgrading.

## Kernel supply chain

The bundled Mihomo kernel is version-pinned: `kernel/<version>/manifest.json` records the version, license and source URL, while `targets` keeps the download URL, size and SHA-256 per build target. Build scripts and the installer verify file presence and hashes; the app never blindly follows GitHub latest. The kernel is renamed `pc-mihomo` to avoid colliding with other proxy clients' processes.

The Geo snapshot is pinned independently in `geodata/manifest.json` to one commit of the official MetaCubeX rule-data repository. Build and packaging checks verify all three files; Settings updates them as one rollback-capable snapshot and never performs a hidden download while validating a subscription.

## Security model

- The controller listens on `127.0.0.1` only, with a strong random secret generated per install and never written to logs
- Subscription URLs, credentials and the controller secret never enter logs or diagnostics; raw validation errors are shown only to the current user
- The Linux TUN service runs a pinned kernel copy as root, materializes configuration atomically into a protected directory with re-validation, and rejects bundles that do not enable TUN, bind beyond loopback, or escape their paths
- System proxy state is written to disk atomically before touching system settings, and self-heals on the next launch after any abnormal exit

See the [technical design document](docs/pure-clash-architecture.md) for the full picture.

## Roadmap

- Rules page and log/memory monitoring (controller `/rules`, `/logs`, `/memory`)
- Linux credential storage and system proxy for other desktops such as KDE
- Full macOS support (kernel guarding, window behavior, TUN boundary)
- Code signing and an update channel

## Contributing

This project itself contains a large amount of AI vibe coding, and the repository maintains [AGENTS.md](AGENTS.md) as the working agreement for AI coding agents. Vibe-coded issues and PRs are accepted and welcome — just follow the same code conventions and verification steps in the [contribution guide](CONTRIBUTING.md).

## Acknowledgments

- [Zed](https://github.com/zed-industries/zed) and GPUI — the UI framework; client-side decorations follow Zed's official examples
- [Mihomo](https://github.com/MetaCubeX/mihomo) (MetaCubeX) — the proxy kernel that handles all traffic forwarding
- [Clash Verge Rev](https://github.com/clash-verge-rev/clash-verge-rev) — reference for the Linux TUN service model and config templates
- [rust-i18n](https://github.com/longbridge/rust-i18n), [ksni](https://github.com/frostwind/ksni), [tray-icon](https://github.com/tauri-apps/tray-icon) and other great open-source libraries

## License

This project is licensed under [GPL-3.0](LICENSE).

The bundled Mihomo kernel is an unmodified upstream GPL-3.0 binary: the full license text (`LICENSE`) and third-party notice (`NOTICE.md`) live in `kernel/<version>/`, `manifest.json` records the matching source URL, and the installer ships these files together with the kernel. The bundled MetaCubeX rule data is also GPL-3.0; its pinned source, license, and notice live in `geodata/`.

Pure Clash is not affiliated with MetaCubeX, and nothing here represents official endorsement from the upstream project. The name Pure Clash deliberately avoids the `mihomo` token that the upstream reserves.
