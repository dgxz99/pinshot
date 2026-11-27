#!/usr/bin/env bash
set -euo pipefail

EXT_UUID="pinshot@dgxz99"
APP_ID="io.github.dgxz99.pinshot"
DCONF_PATH="/io/github/dgxz99/pinshot/"
BIN_PATH="${HOME}/.local/bin/pinshot"
DATA_DIR="${XDG_DATA_HOME:-${HOME}/.local/share}"
EXT_DEST_DIR="${DATA_DIR}/gnome-shell/extensions/${EXT_UUID}"
APPS_DIR="${DATA_DIR}/applications"
DESKTOP_DEST="${APPS_DIR}/${APP_ID}.desktop"
ICON_DEST="${DATA_DIR}/icons/hicolor/scalable/apps/${APP_ID}.svg"
CACHE_DIR="${XDG_CACHE_HOME:-${HOME}/.cache}/pinshot"
CONFIG_DIR="${XDG_CONFIG_HOME:-${HOME}/.config}/pinshot"

PURGE=false

usage() {
    cat <<EOF
用法:
  普通卸载: ./scripts/uninstall.sh
  完全卸载: ./scripts/uninstall.sh --purge
  远程完全卸载: curl -fsSL https://raw.githubusercontent.com/dgxz99/pinshot/main/scripts/uninstall.sh | bash -s -- --purge

选项:
  --purge  完全卸载：同时删除缓存 (${CACHE_DIR})、配置 (${CONFIG_DIR}) 与扩展 GSettings 设置
EOF
}

parse_args() {
    for arg in "$@"; do
        case "$arg" in
            --purge)
                PURGE=true
                ;;
            -h|--help)
                usage
                exit 0
                ;;
            *)
                echo "未知参数: $arg" >&2
                exit 1
                ;;
        esac
    done
}

remove_from_gsettings_list() {
    local key="$1"
    if ! command -v gsettings >/dev/null 2>&1 || ! gsettings writable org.gnome.shell "$key" >/dev/null 2>&1; then
        return 0
    fi
    local current
    current="$(gsettings get org.gnome.shell "$key" 2>/dev/null || true)"
    if [[ "$current" != *"'${EXT_UUID}'"* ]]; then
        return 0
    fi
    local inner="${current#\[}"
    inner="${inner%\]}"
    local kept=()
    local entry trimmed
    local IFS=','
    for entry in $inner; do
        trimmed="${entry#"${entry%%[![:space:]]*}"}"
        trimmed="${trimmed%"${trimmed##*[![:space:]]}"}"
        if [[ -n "$trimmed" && "$trimmed" != "'${EXT_UUID}'" ]]; then
            kept+=("$trimmed")
        fi
    done
    unset IFS
    if [[ ${#kept[@]} -eq 0 ]]; then
        gsettings set org.gnome.shell "$key" "@as []" 2>/dev/null || true
    else
        local joined=""
        for entry in "${kept[@]}"; do
            joined="${joined:+${joined}, }${entry}"
        done
        gsettings set org.gnome.shell "$key" "[${joined}]" 2>/dev/null || true
    fi
}

main() {
    if [[ "${EUID}" -eq 0 ]]; then
        echo "[pinshot] 请使用普通用户运行卸载脚本（AUR 安装请使用 sudo pacman -Rns）" >&2
        exit 1
    fi

    parse_args "$@"

    if command -v gnome-extensions >/dev/null 2>&1; then
        gnome-extensions disable "${EXT_UUID}" 2>/dev/null || true
    fi
    remove_from_gsettings_list enabled-extensions
    remove_from_gsettings_list disabled-extensions

    if [[ -e "${BIN_PATH}" || -L "${BIN_PATH}" ]]; then
        rm -f "${BIN_PATH}"
        echo "[pinshot] 已移除 CLI: ${BIN_PATH}"
    fi

    if [[ -e "${EXT_DEST_DIR}" || -L "${EXT_DEST_DIR}" ]]; then
        rm -rf "${EXT_DEST_DIR}"
        echo "[pinshot] 已移除扩展: ${EXT_DEST_DIR}"
    fi

    if [[ -e "${DESKTOP_DEST}" || -L "${DESKTOP_DEST}" ]]; then
        rm -f "${DESKTOP_DEST}"
        if command -v update-desktop-database >/dev/null 2>&1; then
            update-desktop-database "${APPS_DIR}" >/dev/null 2>&1 || true
        fi
        echo "[pinshot] 已移除应用入口: ${DESKTOP_DEST}"
    fi

    if [[ -e "${ICON_DEST}" || -L "${ICON_DEST}" ]]; then
        rm -f "${ICON_DEST}"
        if command -v gtk-update-icon-cache >/dev/null 2>&1; then
            gtk-update-icon-cache -f -t "${DATA_DIR}/icons/hicolor" >/dev/null 2>&1 || true
        fi
        echo "[pinshot] 已移除应用图标: ${ICON_DEST}"
    fi

    if [[ "$PURGE" == true ]]; then
        rm -rf "${CACHE_DIR}" "${CONFIG_DIR}"
        if command -v dconf >/dev/null 2>&1; then
            dconf reset -f "${DCONF_PATH}" 2>/dev/null || true
        fi
        echo "[pinshot] 已完全清理缓存、配置与扩展设置（已导出的截图/GIF 保留）"
    fi

    echo "[pinshot] 卸载完成"
}

main "$@"
