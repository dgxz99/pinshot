#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"

EXT_UUID="pinshot@dgxz99"
APP_ID="io.github.dgxz99.pinshot"
BIN_DIR="${HOME}/.local/bin"
DATA_DIR="${XDG_DATA_HOME:-${HOME}/.local/share}"
EXT_PARENT_DIR="${DATA_DIR}/gnome-shell/extensions"
EXT_DEST_DIR="${EXT_PARENT_DIR}/${EXT_UUID}"
EXT_SRC_DIR="${REPO_ROOT}/extensions/${EXT_UUID}"
APPS_DIR="${DATA_DIR}/applications"
DESKTOP_SRC="${REPO_ROOT}/data/${APP_ID}.desktop"
DESKTOP_DEST="${APPS_DIR}/${APP_ID}.desktop"
ICON_DIR="${DATA_DIR}/icons/hicolor/scalable/apps"
ICON_DEST="${ICON_DIR}/${APP_ID}.svg"

USE_SYMLINK=false
SKIP_BUILD=false

for arg in "$@"; do
    case "$arg" in
        --symlink)
            USE_SYMLINK=true
            ;;
        --skip-build)
            SKIP_BUILD=true
            ;;
        -h|--help)
            cat <<EOF
用法: $0 [--symlink] [--skip-build]

选项:
  --symlink     使用软链接安装 GNOME Shell 扩展目录（便于开发调试）
  --skip-build  跳过 cargo build --release，直接安装已构建二进制
EOF
            exit 0
            ;;
        *)
            echo "未知参数: $arg" >&2
            exit 1
            ;;
    esac
done

if [[ "$SKIP_BUILD" == false ]]; then
    echo "[pinshot] 正在编译 release 二进制..."
    cargo build --release --locked --manifest-path "${REPO_ROOT}/Cargo.toml" -p pinshot-cli --features longshot
fi

BIN_SRC="${REPO_ROOT}/target/release/pinshot"
if [[ ! -x "$BIN_SRC" ]]; then
    echo "[pinshot] 未找到可执行文件: ${BIN_SRC}" >&2
    exit 1
fi

if ! "${BIN_SRC}" longshot --help >/dev/null 2>&1 ||
   ! "${BIN_SRC}" gif --help >/dev/null 2>&1 ||
   ! "${BIN_SRC}" internal probe-longshot --help >/dev/null 2>&1; then
    echo "[pinshot] 此二进制无法运行长截图或 GIF，请检查 GStreamer 运行库或不带 --skip-build 重新安装" >&2
    exit 1
fi

# GNOME Shell 和扩展首选项均从扩展目录加载编译后的 GSettings schema
if ! command -v glib-compile-schemas >/dev/null 2>&1; then
    echo "[pinshot] 未找到 glib-compile-schemas，请先安装 GLib 工具" >&2
    exit 1
fi
glib-compile-schemas --strict "${EXT_SRC_DIR}/schemas"

mkdir -p "${BIN_DIR}" "${EXT_PARENT_DIR}" "${APPS_DIR}" "${ICON_DIR}"
install -m 0755 "${BIN_SRC}" "${BIN_DIR}/pinshot"
echo "[pinshot] 已安装 CLI: ${BIN_DIR}/pinshot"

rm -rf "${EXT_DEST_DIR}"
if [[ "$USE_SYMLINK" == true ]]; then
    ln -s "${EXT_SRC_DIR}" "${EXT_DEST_DIR}"
    echo "[pinshot] 已软链接扩展: ${EXT_DEST_DIR} -> ${EXT_SRC_DIR}"
else
    cp -r "${EXT_SRC_DIR}" "${EXT_DEST_DIR}"
    echo "[pinshot] 已安装扩展: ${EXT_DEST_DIR}"
fi

install -m 0644 "${EXT_SRC_DIR}/icons/pinshot.svg" "${ICON_DEST}"
sed "s|^Exec=pinshot |Exec=${BIN_DIR}/pinshot |" "${DESKTOP_SRC}" > "${DESKTOP_DEST}"
chmod 0644 "${DESKTOP_DEST}"
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "${APPS_DIR}" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "${DATA_DIR}/icons/hicolor" >/dev/null 2>&1 || true
fi
echo "[pinshot] 已安装应用入口与图标: ${DESKTOP_DEST}"

if command -v gnome-extensions >/dev/null 2>&1; then
    if gnome-extensions list 2>/dev/null | grep -qx "${EXT_UUID}"; then
        gnome-extensions enable "${EXT_UUID}" || true
        echo "[pinshot] 已尝试启用 GNOME 扩展: ${EXT_UUID}"
    else
        echo "[pinshot] 提示: Wayland 会话首次安装扩展需重新登录后执行: gnome-extensions enable ${EXT_UUID}"
    fi
fi

if [[ -d "/usr/share/gnome-shell/extensions/${EXT_UUID}" ]]; then
    echo "[pinshot] 提示: 检测到系统级扩展 /usr/share/gnome-shell/extensions/${EXT_UUID}，用户级安装会优先覆盖它"
fi

if [[ ":${PATH}:" != *":${BIN_DIR}:"* ]]; then
    echo "[pinshot] 提示: 请将 ${BIN_DIR} 加入 PATH 环境变量"
fi

if ! command -v satty >/dev/null 2>&1; then
    echo "[pinshot] 可选提示: 未检测到 satty（标注功能依赖），如需截图/贴图标注可按需安装"
fi
if ! command -v notify-send >/dev/null 2>&1; then
    echo "[pinshot] 可选提示: 未检测到 notify-send（libnotify），如需桌面通知可按需安装"
fi

echo "[pinshot] 安装完成"
