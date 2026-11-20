#!/usr/bin/env bash
set -euo pipefail

EXT_UUID="pinshot@dgxz99"
BIN_PATH="${HOME}/.local/bin/pinshot"
EXT_DEST_DIR="${HOME}/.local/share/gnome-shell/extensions/${EXT_UUID}"
CACHE_DIR="${XDG_CACHE_HOME:-${HOME}/.cache}/pinshot"
CONFIG_DIR="${XDG_CONFIG_HOME:-${HOME}/.config}/pinshot"

PURGE=false

for arg in "$@"; do
    case "$arg" in
        --purge)
            PURGE=true
            ;;
        -h|--help)
            cat <<EOF
用法: $0 [--purge]

选项:
  --purge  同时删除缓存目录 (${CACHE_DIR}) 与配置目录 (${CONFIG_DIR})
EOF
            exit 0
            ;;
        *)
            echo "未知参数: $arg" >&2
            exit 1
            ;;
    esac
done

if command -v gnome-extensions >/dev/null 2>&1; then
    gnome-extensions disable "${EXT_UUID}" 2>/dev/null || true
fi

if [[ -e "${BIN_PATH}" || -L "${BIN_PATH}" ]]; then
    rm -f "${BIN_PATH}"
    echo "[pinshot] 已移除 CLI: ${BIN_PATH}"
fi

if [[ -e "${EXT_DEST_DIR}" || -L "${EXT_DEST_DIR}" ]]; then
    rm -rf "${EXT_DEST_DIR}"
    echo "[pinshot] 已移除扩展: ${EXT_DEST_DIR}"
fi

if [[ "$PURGE" == true ]]; then
    rm -rf "${CACHE_DIR}" "${CONFIG_DIR}"
    echo "[pinshot] 已清理缓存与配置目录"
fi

echo "[pinshot] 卸载完成"
