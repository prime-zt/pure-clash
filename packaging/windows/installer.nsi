Unicode true
RequestExecutionLevel user
SetCompressor /SOLID lzma
ManifestDPIAware true

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"

!ifndef APP_VERSION
  !error "APP_VERSION is required"
!endif
!ifndef APP_VERSION_NUMERIC
  !error "APP_VERSION_NUMERIC is required"
!endif
!ifndef APP_EXE_SOURCE
  !error "APP_EXE_SOURCE is required"
!endif
!ifndef APP_ICON_SOURCE
  !error "APP_ICON_SOURCE is required"
!endif
!ifndef KERNEL_VERSION
  !error "KERNEL_VERSION is required"
!endif
!ifndef KERNEL_BINARY_NAME
  !error "KERNEL_BINARY_NAME is required"
!endif
!ifndef KERNEL_EXE_SOURCE
  !error "KERNEL_EXE_SOURCE is required"
!endif
!ifndef PROJECT_LICENSE_SOURCE
  !error "PROJECT_LICENSE_SOURCE is required"
!endif
!ifndef KERNEL_LICENSE_SOURCE
  !error "KERNEL_LICENSE_SOURCE is required"
!endif
!ifndef KERNEL_NOTICE_SOURCE
  !error "KERNEL_NOTICE_SOURCE is required"
!endif
!ifndef KERNEL_MANIFEST_SOURCE
  !error "KERNEL_MANIFEST_SOURCE is required"
!endif
!ifndef GEODATA_ROOT
  !error "GEODATA_ROOT is required"
!endif
!ifndef INSTALLER_OUTPUT
  !error "INSTALLER_OUTPUT is required"
!endif
!ifndef INSTALL_SIZE_KB
  !define INSTALL_SIZE_KB 0
!endif
!ifndef APP_NAME
  !define APP_NAME "Pure Clash"
!endif
!ifndef APP_ID
  !define APP_ID "PureClash"
!endif

!define APP_EXE "pure-clash.exe"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}"

Name "${APP_NAME}"
Caption "${APP_NAME} ${APP_VERSION} 安装程序"
OutFile "${INSTALLER_OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\${APP_NAME}"
InstallDirRegKey HKCU "${UNINSTALL_KEY}" "InstallLocation"
BrandingText "${APP_NAME}"

; 项目尚未声明版权主体，因此只保留可确认的版本字段并局部忽略可选版权键警告。
!pragma warning disable 9100
VIProductVersion "${APP_VERSION_NUMERIC}"
VIAddVersionKey "ProductName" "${APP_NAME}"
VIAddVersionKey "FileDescription" "${APP_NAME} 安装程序"
VIAddVersionKey "ProductVersion" "${APP_VERSION}"
VIAddVersionKey "FileVersion" "${APP_VERSION}"

; MUI2 会在插入页面时设置图标，必须通过其接口变量覆盖默认 NSIS 图标。
!define MUI_ICON "${APP_ICON_SOURCE}"
!define MUI_UNICON "${APP_ICON_SOURCE}"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP_EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "启动 ${APP_NAME}"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH

!insertmacro MUI_LANGUAGE "SimpChinese"

Function .onInit
  ; 当前发布目标为 x86_64，提前阻止不兼容系统继续安装。
  ${IfNot} ${RunningX64}
    MessageBox MB_OK|MB_ICONSTOP "${APP_NAME} 需要 64 位 Windows。"
    Abort
  ${EndIf}

  ; 安装范围限定为当前用户，并使用 64 位卸载注册表视图。
  SetShellVarContext current
  SetRegView 64
FunctionEnd

Function un.onInit
  SetShellVarContext current
  SetRegView 64
FunctionEnd

Section "主程序（必需）" SecMain
  SectionIn RO

  SetOutPath "$INSTDIR"
  SetOverwrite on
  File "/oname=${APP_EXE}" "${APP_EXE_SOURCE}"
  ; 项目自身的 GPL-3.0 许可文本安装在根目录；内核许可证单独放在版本目录。
  File "/oname=LICENSE" "${PROJECT_LICENSE_SOURCE}"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  ; 内核、许可证和来源清单与主程序一起安装到 manifest 指定的版本目录。
  SetOutPath "$INSTDIR\kernel\${KERNEL_VERSION}"
  File "/oname=${KERNEL_BINARY_NAME}" "${KERNEL_EXE_SOURCE}"
  File "/oname=LICENSE" "${KERNEL_LICENSE_SOURCE}"
  File "/oname=NOTICE.md" "${KERNEL_NOTICE_SOURCE}"
  File "/oname=manifest.json" "${KERNEL_MANIFEST_SOURCE}"
  ; wintun 驱动仅在打包脚本发现 manifest 声明时安装（TUN 能力）。
  !ifdef KERNEL_WINTUN_SOURCE
    File "/oname=wintun.dll" "${KERNEL_WINTUN_SOURCE}"
  !endif

  ; 三份 Geo 基础库随包安装，应用首次启动复制到 data/mihomo 后供内核使用。
  SetOutPath "$INSTDIR\geodata"
  File "/oname=GeoSite.dat" "${GEODATA_ROOT}\GeoSite.dat"
  File "/oname=GeoIP.dat" "${GEODATA_ROOT}\GeoIP.dat"
  File "/oname=Country.mmdb" "${GEODATA_ROOT}\Country.mmdb"
  File "/oname=manifest.json" "${GEODATA_ROOT}\manifest.json"
  File "/oname=LICENSE" "${GEODATA_ROOT}\LICENSE"
  File "/oname=NOTICE.md" "${GEODATA_ROOT}\NOTICE.md"

  ; 开始菜单始终提供启动和卸载入口，避免用户只能依赖安装目录。
  CreateDirectory "$SMPROGRAMS\${APP_NAME}"
  CreateShortcut "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}"
  CreateShortcut "$SMPROGRAMS\${APP_NAME}\卸载 ${APP_NAME}.lnk" "$INSTDIR\Uninstall.exe"

  ; 使用 HKCU 注册标准卸载信息，整个安装过程无需管理员权限。
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${APP_EXE},0"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "EstimatedSize" ${INSTALL_SIZE_KB}
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
SectionEnd

Section "桌面快捷方式" SecDesktop
  CreateShortcut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}"
SectionEnd

LangString DESC_SecMain ${LANG_SIMPCHINESE} "安装 Pure Clash 主程序和卸载器。"
LangString DESC_SecDesktop ${LANG_SIMPCHINESE} "在当前用户桌面创建启动快捷方式。"

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecMain} $(DESC_SecMain)
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} $(DESC_SecDesktop)
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Section "Uninstall"
  ; config 和 data 可能包含用户配置，卸载时默认保留，避免静默删除用户数据。
  Delete "$DESKTOP\${APP_NAME}.lnk"
  Delete "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk"
  Delete "$SMPROGRAMS\${APP_NAME}\卸载 ${APP_NAME}.lnk"
  RMDir "$SMPROGRAMS\${APP_NAME}"

  Delete "$INSTDIR\${APP_EXE}"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\kernel\${KERNEL_VERSION}\${KERNEL_BINARY_NAME}"
  Delete "$INSTDIR\kernel\${KERNEL_VERSION}\LICENSE"
  Delete "$INSTDIR\kernel\${KERNEL_VERSION}\NOTICE.md"
  Delete "$INSTDIR\kernel\${KERNEL_VERSION}\manifest.json"
  ; 兼容带与不带 wintun 驱动的历史安装布局。
  Delete "$INSTDIR\kernel\${KERNEL_VERSION}\wintun.dll"
  RMDir "$INSTDIR\kernel\${KERNEL_VERSION}"
  RMDir "$INSTDIR\kernel"
  ; 仅清理安装资源；data/mihomo 下可能是用户更新版本，继续保留。
  Delete "$INSTDIR\geodata\GeoSite.dat"
  Delete "$INSTDIR\geodata\GeoIP.dat"
  Delete "$INSTDIR\geodata\Country.mmdb"
  Delete "$INSTDIR\geodata\manifest.json"
  Delete "$INSTDIR\geodata\LICENSE"
  Delete "$INSTDIR\geodata\NOTICE.md"
  RMDir "$INSTDIR\geodata"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
SectionEnd
