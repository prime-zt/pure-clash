<#
.SYNOPSIS
构建 release 可执行文件并生成 NSIS 安装包。

.PARAMETER NsisPath
可选的 makensis.exe 完整路径。为空时依次检查 PATH 和 NSIS 默认安装目录。

.OUTPUTS
在 dist 目录生成带 Cargo 包版本号的 Windows x64 安装程序。
#>
[CmdletBinding()]
param(
    [string]$NsisPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$manifestPath = Join-Path $projectRoot "Cargo.toml"
$nsisScript = Join-Path $PSScriptRoot "installer.nsi"

Push-Location $projectRoot
try {
    & cargo build --release
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build --release 执行失败。"
    }

    $metadataJson = (& cargo metadata --no-deps --format-version 1 | Out-String)
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata 执行失败。"
    }

    $metadata = $metadataJson | ConvertFrom-Json
    $package = $metadata.packages |
        Where-Object { [IO.Path]::GetFullPath($_.manifest_path) -eq [IO.Path]::GetFullPath($manifestPath) } |
        Select-Object -First 1
    if (-not $package) {
        throw "无法从 cargo metadata 中定位当前包。"
    }

    $version = [string]$package.version
    $coreVersion = ($version -split "[-+]")[0]
    $versionParts = $coreVersion -split "\."
    if ($versionParts.Count -lt 3) {
        throw "Cargo 包版本必须至少包含 major.minor.patch。"
    }
    $numericVersion = "{0}.{1}.{2}.0" -f $versionParts[0], $versionParts[1], $versionParts[2]

    # 使用 Cargo 返回的实际 target 目录，兼容 CARGO_TARGET_DIR 和隔离构建。
    $cargoTargetDir = [IO.Path]::GetFullPath([string]$metadata.target_directory)
    $appExe = Join-Path $cargoTargetDir "release\pure-clash.exe"
    if (-not (Test-Path -LiteralPath $appExe -PathType Leaf)) {
        throw "release 可执行文件不存在：$appExe"
    }
    $appIcon = Join-Path $projectRoot "assets\windows\pure-clash.ico"
    if (-not (Test-Path -LiteralPath $appIcon -PathType Leaf)) {
        throw "Windows 应用图标不存在：$appIcon"
    }

    # 随包内核 manifest 是版本唯一来源；多个候选会导致默认版本不明确并中止打包。
    $kernelRoot = Join-Path $projectRoot "kernel"
    $kernelManifests = @(Get-ChildItem -LiteralPath $kernelRoot -Directory | ForEach-Object {
        $candidate = Join-Path $_.FullName "manifest.json"
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            $candidate
        }
    })
    if ($kernelManifests.Count -ne 1) {
        throw "kernel 目录必须且只能包含一个随包内核 manifest.json，当前找到 $($kernelManifests.Count) 个。"
    }

    $kernelManifest = $kernelManifests[0]
    $kernelMetadata = Get-Content -Raw -LiteralPath $kernelManifest | ConvertFrom-Json
    $kernelVersion = [string]$kernelMetadata.version
    if ([string]::IsNullOrWhiteSpace($kernelVersion)) {
        throw "内核 manifest 缺少有效 version：$kernelManifest"
    }
    $kernelDir = Split-Path -Parent $kernelManifest
    if ((Split-Path -Leaf $kernelDir) -ne $kernelVersion) {
        throw "内核目录名与 manifest version 不一致：$kernelDir"
    }

    # manifest 的 targets 按编译目标记录各平台发行条目；安装器只消费 windows-amd64。
    $windowsTarget = $kernelMetadata.targets.'windows-amd64'
    if (-not $windowsTarget) {
        throw "内核 manifest 缺少 windows-amd64 发行目标：$kernelManifest"
    }

    # 随包文件名由 manifest 统一控制；Windows 当前产品名固定为 pc-mihomo.exe。
    $kernelBinaryName = [string]$windowsTarget.binary
    if ($kernelBinaryName -ne "pc-mihomo.exe") {
        throw "内核 manifest binary 必须为 pc-mihomo.exe，当前为：$kernelBinaryName"
    }
    $kernelExe = Join-Path $kernelDir $kernelBinaryName
    $kernelLicense = Join-Path $kernelDir "LICENSE"
    $kernelNotice = Join-Path $kernelDir "NOTICE.md"
    foreach ($requiredKernelFile in @($kernelExe, $kernelLicense, $kernelNotice, $kernelManifest)) {
        if (-not (Test-Path -LiteralPath $requiredKernelFile -PathType Leaf)) {
            throw "内置 Mihomo 文件不存在：$requiredKernelFile"
        }
    }

    # 打包前按受控清单复核供应链信息，避免误把错误版本或损坏文件发布出去。
    if ($windowsTarget.platform -ne "windows" -or
        $windowsTarget.architecture -ne "amd64" -or
        $windowsTarget.build -ne "compatible") {
        throw "内核 manifest 与 Windows amd64-compatible 发行目标不一致。"
    }
    if ((Get-Item -LiteralPath $kernelExe).Length -ne $windowsTarget.binary_size) {
        throw "内置 Mihomo 文件大小与 manifest 不一致。"
    }
    $kernelHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $kernelExe).Hash.ToLowerInvariant()
    if ($kernelHash -ne $windowsTarget.binary_sha256) {
        throw "内置 Mihomo SHA-256 与 manifest 不一致。"
    }
    $licenseHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $kernelLicense).Hash.ToLowerInvariant()
    if ($licenseHash -ne $kernelMetadata.license_sha256) {
        throw "Mihomo LICENSE SHA-256 与 manifest 不一致。"
    }

    # wintun.dll 是 Windows 目标启用 TUN 的必要驱动，manifest 锁定来源与哈希。
    $wintunSource = $null
    if ($windowsTarget.wintun) {
        $wintunName = [string]$windowsTarget.wintun.file
        $wintunSource = Join-Path $kernelDir $wintunName
        if (-not (Test-Path -LiteralPath $wintunSource -PathType Leaf)) {
            throw "内置 wintun 驱动不存在：$wintunSource"
        }
        if ((Get-Item -LiteralPath $wintunSource).Length -ne $windowsTarget.wintun.file_size) {
            throw "内置 wintun 驱动大小与 manifest 不一致。"
        }
        $wintunHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $wintunSource).Hash.ToLowerInvariant()
        if ($wintunHash -ne $windowsTarget.wintun.file_sha256) {
            throw "内置 wintun 驱动 SHA-256 与 manifest 不一致。"
        }
    }

    # 项目自身的 GPL-3.0 许可文本随安装包分发，与内核许可证分别履行义务。
    $projectLicense = Join-Path $projectRoot "LICENSE"
    if (-not (Test-Path -LiteralPath $projectLicense -PathType Leaf)) {
        throw "未找到项目 LICENSE 文件：$projectLicense"
    }

    $distDir = Join-Path $projectRoot "dist"
    New-Item -ItemType Directory -Force -Path $distDir | Out-Null
    $installerOutput = Join-Path $distDir "pure-clash-$version-windows-x64-setup.exe"
    $packagedFiles = @($appExe, $kernelExe, $kernelLicense, $kernelNotice, $kernelManifest, $projectLicense)
    if ($wintunSource) {
        $packagedFiles += $wintunSource
    }
    $installSizeBytes = ($packagedFiles |
        ForEach-Object { (Get-Item -LiteralPath $_).Length } |
        Measure-Object -Sum).Sum
    $installSizeKb = [int][Math]::Ceiling($installSizeBytes / 1KB)

    # 优先尊重显式参数，其次检查 PATH，最后检查 NSIS 的常见安装目录。
    $nsisCandidates = @()
    if ($NsisPath) {
        $nsisCandidates += $NsisPath
    }
    $nsisCommand = Get-Command "makensis.exe" -ErrorAction SilentlyContinue
    if ($nsisCommand) {
        $nsisCandidates += $nsisCommand.Source
    }
    $nsisCandidates += Join-Path $env:ProgramFiles "NSIS\makensis.exe"
    $nsisCandidates += Join-Path ${env:ProgramFiles(x86)} "NSIS\makensis.exe"

    $makensis = $nsisCandidates |
        Where-Object { $_ -and (Test-Path -LiteralPath $_ -PathType Leaf) } |
        Select-Object -First 1
    if (-not $makensis) {
        throw "未找到 makensis.exe。请安装 NSIS 3.x，或通过 -NsisPath 指定编译器路径。"
    }

    $nsisArguments = @(
        "/V3"
        "/WX"
        # 明确按 UTF-8 读取脚本，避免中文文案受 Windows 系统代码页影响。
        "/INPUTCHARSET"
        "UTF8"
        "/DAPP_VERSION=$version"
        "/DAPP_VERSION_NUMERIC=$numericVersion"
        "/DAPP_EXE_SOURCE=$appExe"
        "/DAPP_ICON_SOURCE=$appIcon"
        "/DKERNEL_VERSION=$kernelVersion"
        "/DKERNEL_BINARY_NAME=$kernelBinaryName"
        "/DKERNEL_EXE_SOURCE=$kernelExe"
        "/DKERNEL_LICENSE_SOURCE=$kernelLicense"
        "/DKERNEL_NOTICE_SOURCE=$kernelNotice"
        "/DKERNEL_MANIFEST_SOURCE=$kernelManifest"
        "/DPROJECT_LICENSE_SOURCE=$projectLicense"
        "/DINSTALLER_OUTPUT=$installerOutput"
        "/DINSTALL_SIZE_KB=$installSizeKb"
        $nsisScript
    )
    # wintun 驱动随内核打包；NSIS 脚本按 define 是否存在决定是否安装。
    if ($wintunSource) {
        $nsisArguments += "/DKERNEL_WINTUN_SOURCE=$wintunSource"
    }

    & $makensis @nsisArguments
    if ($LASTEXITCODE -ne 0) {
        throw "NSIS 安装包编译失败。"
    }
    if (-not (Test-Path -LiteralPath $installerOutput -PathType Leaf)) {
        throw "NSIS 未生成预期安装包：$installerOutput"
    }

    Write-Host "安装包已生成：$installerOutput"
}
finally {
    Pop-Location
}
