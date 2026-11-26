#!/usr/bin/env bash
set -euo pipefail

EXT_UUID="pinshot@dgxz99"
APP_ID="io.github.dgxz99.pinshot"
REPO="${PINSHOT_REPO:-dgxz99/pinshot}"

BIN_DIR="${HOME}/.local/bin"
DATA_DIR="${XDG_DATA_HOME:-${HOME}/.local/share}"
EXT_PARENT_DIR="${DATA_DIR}/gnome-shell/extensions"
EXT_DEST_DIR="${EXT_PARENT_DIR}/${EXT_UUID}"
APPS_DIR="${DATA_DIR}/applications"
DESKTOP_DEST="${APPS_DIR}/${APP_ID}.desktop"
ICON_DIR="${DATA_DIR}/icons/hicolor/scalable/apps"
ICON_DEST="${ICON_DIR}/${APP_ID}.svg"

USE_SYMLINK=false
SKIP_BUILD=false
FROM_RELEASE=false
RELEASE_VERSION="${PINSHOT_VERSION:-}"
ARCHIVE_FILE=""
TEMP_DIR=""

cleanup() {
    if [[ -n "$TEMP_DIR" && -d "$TEMP_DIR" ]]; then
        rm -rf "$TEMP_DIR"
    fi
}
trap cleanup EXIT

usage() {
    cat <<EOF
用法:
  本地源码安装: ./scripts/install.sh [--symlink] [--skip-build]
  预编译包安装: curl -fsSL https://raw.githubusercontent.com/${REPO}/main/scripts/install.sh | bash
  指定版本安装: ./scripts/install.sh --from-release [--version v0.1.0]
  本地压缩包测试: ./scripts/install.sh --archive dist/pinshot-v0.1.0-linux-x86_64.tar.gz

选项:
  --symlink         使用软链接安装本地 GNOME Shell 扩展目录（仅本地源码模式）
  --skip-build      跳过 cargo build --release，直接安装已构建二进制（仅本地源码模式）
  --from-release    强制从 GitHub Releases 下载预编译包安装
  --version VER     指定要下载的 Release 版本号（如 v0.1.0，隐含 --from-release）
  --archive FILE    直接从本地 Release .tar.gz 压缩包安装（用于离线安装或发版前测试）
  -h, --help        显示此帮助信息
EOF
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --symlink)
                USE_SYMLINK=true
                shift
                ;;
            --skip-build)
                SKIP_BUILD=true
                shift
                ;;
            --from-release)
                FROM_RELEASE=true
                shift
                ;;
            --version)
                if [[ $# -lt 2 ]]; then
                    echo "缺少参数值: --version" >&2
                    exit 1
                fi
                RELEASE_VERSION="$2"
                FROM_RELEASE=true
                shift 2
                ;;
            --archive)
                if [[ $# -lt 2 ]]; then
                    echo "缺少参数值: --archive" >&2
                    exit 1
                fi
                ARCHIVE_FILE="$2"
                FROM_RELEASE=true
                shift 2
                ;;
            -h|--help)
                usage
                exit 0
                ;;
            *)
                echo "未知参数: $1" >&2
                exit 1
                ;;
        esac
    done
}

detect_local_repo() {
    local src="${BASH_SOURCE[0]:-}"
    if [[ -z "$src" || ! -f "$src" ]]; then
        return 1
    fi
    local script_dir repo_root
    script_dir="$(cd -- "$(dirname -- "$src")" && pwd)"
    repo_root="$(cd -- "${script_dir}/.." && pwd)"
    if [[ -f "${repo_root}/Cargo.toml" && -f "${repo_root}/extensions/${EXT_UUID}/metadata.json" ]]; then
        printf '%s\n' "$repo_root"
        return 0
    fi
    return 1
}

verify_binary() {
    local bin_path="$1"
    if [[ ! -x "$bin_path" ]]; then
        echo "[pinshot] 未找到可执行文件: ${bin_path}" >&2
        exit 1
    fi
    if ! "${bin_path}" longshot --help >/dev/null 2>&1 ||
       ! "${bin_path}" gif --help >/dev/null 2>&1 ||
       ! "${bin_path}" capture-fullscreen --help >/dev/null 2>&1 ||
       ! "${bin_path}" internal probe-longshot --help >/dev/null 2>&1; then
        echo "[pinshot] 此二进制无法运行长截图、GIF 或全屏截图，请检查 GStreamer 运行库或重新编译安装" >&2
        exit 1
    fi
}

install_desktop_and_icon() {
    local icon_src="$1"
    local desktop_src="$2"

    mkdir -p "${APPS_DIR}" "${ICON_DIR}"
    install -m 0644 "${icon_src}" "${ICON_DEST}"
    sed "s|^Exec=pinshot |Exec=${BIN_DIR}/pinshot |" "${desktop_src}" > "${DESKTOP_DEST}"
    chmod 0644 "${DESKTOP_DEST}"

    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "${APPS_DIR}" >/dev/null 2>&1 || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -f -t "${DATA_DIR}/icons/hicolor" >/dev/null 2>&1 || true
    fi
    echo "[pinshot] 已安装应用入口与图标: ${DESKTOP_DEST}"
}

post_install_checks() {
    if command -v gnome-extensions >/dev/null 2>&1; then
        if gnome-extensions list 2>/dev/null | grep -qx "${EXT_UUID}"; then
            gnome-extensions enable "${EXT_UUID}" || true
            echo "[pinshot] 已尝试启用 GNOME 扩展: ${EXT_UUID}"
        else
            echo "[pinshot] 提示: Wayland 会话首次安装或更新扩展后，请注销并重新登录，再执行: gnome-extensions enable ${EXT_UUID}"
        fi
    else
        echo "[pinshot] 提示: 未检测到 gnome-extensions 命令，请确认当前处于 GNOME Shell 桌面环境"
    fi

    if [[ -d "/usr/share/gnome-shell/extensions/${EXT_UUID}" ]]; then
        echo "[pinshot] 提示: 检测到系统级扩展 /usr/share/gnome-shell/extensions/${EXT_UUID}，用户级安装会优先覆盖它"
    fi

    if [[ ":${PATH}:" != *":${BIN_DIR}:"* ]]; then
        echo "[pinshot] 提示: 请将 ${BIN_DIR} 加入 PATH 环境变量"
    fi

    local missing_req=()
    for cmd in wl-copy wl-paste pango-view; do
        if ! command -v "$cmd" >/dev/null 2>&1; then
            missing_req+=("$cmd")
        fi
    done
    if [[ ${#missing_req[@]} -gt 0 ]]; then
        echo "[pinshot] 运行依赖提示: 未检测到 ${missing_req[*]}，请安装 wl-clipboard 与 pango 工具包"
    fi

    if ! command -v satty >/dev/null 2>&1; then
        echo "[pinshot] 可选提示: 未检测到 satty（标注功能依赖），如需截图/贴图标注可按需安装"
    fi
    if ! command -v notify-send >/dev/null 2>&1; then
        echo "[pinshot] 可选提示: 未检测到 notify-send（libnotify），如需桌面通知可按需安装"
    fi

    echo "[pinshot] 安装完成"
}

install_from_source() {
    local repo_root="$1"
    local ext_src_dir="${repo_root}/extensions/${EXT_UUID}"
    local desktop_src="${repo_root}/data/${APP_ID}.desktop"

    if [[ "$SKIP_BUILD" == false ]]; then
        echo "[pinshot] 正在编译 release 二进制..."
        cargo build --release --locked --manifest-path "${repo_root}/Cargo.toml" -p pinshot-cli --features longshot
    fi

    local bin_src="${repo_root}/target/release/pinshot"
    verify_binary "$bin_src"

    if ! command -v glib-compile-schemas >/dev/null 2>&1; then
        echo "[pinshot] 未找到 glib-compile-schemas，请先安装 GLib 工具" >&2
        exit 1
    fi
    glib-compile-schemas --strict "${ext_src_dir}/schemas"

    mkdir -p "${BIN_DIR}" "${EXT_PARENT_DIR}"
    install -m 0755 "${bin_src}" "${BIN_DIR}/pinshot"
    echo "[pinshot] 已安装 CLI: ${BIN_DIR}/pinshot"

    rm -rf "${EXT_DEST_DIR}"
    if [[ "$USE_SYMLINK" == true ]]; then
        ln -s "${ext_src_dir}" "${EXT_DEST_DIR}"
        echo "[pinshot] 已软链接扩展: ${EXT_DEST_DIR} -> ${ext_src_dir}"
    else
        cp -r "${ext_src_dir}" "${EXT_DEST_DIR}"
        echo "[pinshot] 已安装扩展: ${EXT_DEST_DIR}"
    fi

    install_desktop_and_icon "${ext_src_dir}/icons/pinshot.svg" "${desktop_src}"
    post_install_checks
}

resolve_latest_version() {
    local latest_url version
    latest_url="$(curl -fsSL -o /dev/null -w '%{url_effective}' "https://github.com/${REPO}/releases/latest")"
    version="${latest_url##*/}"
    if [[ -z "$version" || "$version" == "releases" || "$version" == "latest" ]]; then
        echo "[pinshot] 无法解析最新 Release 版本，请检查 https://github.com/${REPO}/releases 是否已有发布版本" >&2
        exit 1
    fi
    printf '%s\n' "$version"
}

install_from_release() {
    if [[ "$USE_SYMLINK" == true || "$SKIP_BUILD" == true ]]; then
        echo "[pinshot] --symlink 与 --skip-build 仅适用于本地源码安装模式" >&2
        exit 1
    fi

    if ! command -v tar >/dev/null 2>&1; then
        echo "[pinshot] 未找到 tar 命令" >&2
        exit 1
    fi

    TEMP_DIR="$(mktemp -d)"
    local archive_path=""

    if [[ -n "$ARCHIVE_FILE" ]]; then
        if [[ ! -f "$ARCHIVE_FILE" ]]; then
            echo "[pinshot] 未找到压缩包: ${ARCHIVE_FILE}" >&2
            exit 1
        fi
        archive_path="$ARCHIVE_FILE"
        echo "[pinshot] 正在从本地发布包安装: ${archive_path}"
    else
        for cmd in curl sha256sum; do
            if ! command -v "$cmd" >/dev/null 2>&1; then
                echo "[pinshot] 未找到 ${cmd} 命令，请先安装" >&2
                exit 1
            fi
        done

        local arch version archive_name base_url
        arch="$(uname -m)"
        if [[ "$arch" != "x86_64" ]]; then
            echo "[pinshot] 暂未提供 ${arch} 架构预编译包，请克隆仓库从源码构建" >&2
            exit 1
        fi

        if [[ -n "$RELEASE_VERSION" ]]; then
            version="$RELEASE_VERSION"
        else
            echo "[pinshot] 正在获取最新版本信息..."
            version="$(resolve_latest_version)"
        fi
        [[ "$version" == v* ]] || version="v${version}"

        archive_name="pinshot-${version}-linux-${arch}.tar.gz"
        base_url="https://github.com/${REPO}/releases/download/${version}"
        archive_path="${TEMP_DIR}/${archive_name}"

        echo "[pinshot] 正在下载 ${archive_name}..."
        curl -fsSL "${base_url}/${archive_name}" -o "${archive_path}"
        curl -fsSL "${base_url}/SHA256SUMS" -o "${TEMP_DIR}/SHA256SUMS"

        echo "[pinshot] 正在校验 SHA256..."
        (
            cd "${TEMP_DIR}"
            grep "  ${archive_name}$" SHA256SUMS | sha256sum -c -
        )
    fi

    local extract_dir="${TEMP_DIR}/extract"
    mkdir -p "${extract_dir}"
    tar -xzf "${archive_path}" -C "${extract_dir}"

    local bundle_dir
    bundle_dir="$(find "${extract_dir}" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
    if [[ -z "$bundle_dir" || ! -x "${bundle_dir}/bin/pinshot" ]]; then
        echo "[pinshot] 发布包结构无效，缺少 bin/pinshot" >&2
        exit 1
    fi

    verify_binary "${bundle_dir}/bin/pinshot"

    mkdir -p "${BIN_DIR}" "${EXT_PARENT_DIR}"
    install -m 0755 "${bundle_dir}/bin/pinshot" "${BIN_DIR}/pinshot"
    echo "[pinshot] 已安装 CLI: ${BIN_DIR}/pinshot"

    rm -rf "${EXT_DEST_DIR}"
    cp -r "${bundle_dir}/share/gnome-shell/extensions/${EXT_UUID}" "${EXT_DEST_DIR}"
    echo "[pinshot] 已安装扩展: ${EXT_DEST_DIR}"

    install_desktop_and_icon \
        "${bundle_dir}/share/icons/hicolor/scalable/apps/${APP_ID}.svg" \
        "${bundle_dir}/share/applications/${APP_ID}.desktop"
    post_install_checks
}

main() {
    parse_args "$@"

    local repo_root=""
    if [[ "$FROM_RELEASE" == false ]] && repo_root="$(detect_local_repo)"; then
        install_from_source "$repo_root"
    else
        install_from_release
    fi
}

main "$@"
