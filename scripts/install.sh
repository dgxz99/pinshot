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
DEPS_MODE="ask"
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
  本地源码安装: ./scripts/install.sh [--symlink] [--skip-build] [--install-deps|--no-deps]
  预编译包安装: curl -fsSL https://raw.githubusercontent.com/${REPO}/main/scripts/install.sh | bash
  指定版本安装: ./scripts/install.sh --from-release [--version v0.1.0]
  本地压缩包测试: ./scripts/install.sh --archive dist/pinshot-v0.1.0-linux-x86_64.tar.gz

选项:
  --symlink         使用软链接安装本地 GNOME Shell 扩展目录（仅本地源码模式）
  --skip-build      跳过 cargo build --release，直接安装已构建二进制（仅本地源码模式）
  --from-release    强制从 GitHub Releases 下载预编译包安装
  --version VER     指定要下载的 Release 版本号（如 v0.1.0，隐含 --from-release）
  --archive FILE    直接从本地 Release .tar.gz 压缩包安装（用于离线安装或发版前测试）
  --install-deps    缺失必需运行依赖时自动调用 sudo 安装，不询问
  --no-deps         跳过自动安装缺失依赖，仅打印安装命令
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
            --install-deps)
                DEPS_MODE="yes"
                shift
                ;;
            --no-deps)
                DEPS_MODE="no"
                shift
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

has_gst_plugin() {
    local plugin_name="$1"
    local so_name="$2"
    if command -v gst-inspect-1.0 >/dev/null 2>&1; then
        if gst-inspect-1.0 "$plugin_name" >/dev/null 2>&1; then
            return 0
        fi
    fi
    compgen -G "/usr/lib*/gstreamer-1.0/${so_name}" >/dev/null 2>&1 ||
        compgen -G "/usr/lib/*/gstreamer-1.0/${so_name}" >/dev/null 2>&1
}

detect_missing_runtime_deps() {
    MISSING_LABELS=()
    NEED_WL_CLIPBOARD=false
    NEED_PANGO_VIEW=false
    NEED_GST_PIPEWIRE=false
    NEED_GST_BASE=false

    if ! command -v wl-copy >/dev/null 2>&1 || ! command -v wl-paste >/dev/null 2>&1; then
        NEED_WL_CLIPBOARD=true
        MISSING_LABELS+=("wl-clipboard (wl-copy/wl-paste)")
    fi
    if ! command -v pango-view >/dev/null 2>&1; then
        NEED_PANGO_VIEW=true
        MISSING_LABELS+=("pango-view")
    fi
    if ! has_gst_plugin pipewiresrc libgstpipewire.so; then
        NEED_GST_PIPEWIRE=true
        MISSING_LABELS+=("GStreamer pipewiresrc")
    fi
    if ! has_gst_plugin videorate libgstvideorate.so || ! has_gst_plugin videoconvert libgstvideoconvertscale.so; then
        if ! has_gst_plugin videorate libgstvideorate.so || ! has_gst_plugin videoconvert libgstvideoconvert.so; then
            NEED_GST_BASE=true
            MISSING_LABELS+=("GStreamer plugins-base")
        fi
    fi
}

build_dep_install_cmd() {
    DEP_PKG_MGR=""
    DEP_PKGS=()

    if command -v apt-get >/dev/null 2>&1; then
        DEP_PKG_MGR="apt"
        [[ "$NEED_WL_CLIPBOARD" == true ]] && DEP_PKGS+=("wl-clipboard")
        [[ "$NEED_PANGO_VIEW" == true ]] && DEP_PKGS+=("pango1.0-tools")
        [[ "$NEED_GST_PIPEWIRE" == true ]] && DEP_PKGS+=("gstreamer1.0-pipewire")
        [[ "$NEED_GST_BASE" == true ]] && DEP_PKGS+=("gstreamer1.0-plugins-base")
    elif command -v pacman >/dev/null 2>&1; then
        DEP_PKG_MGR="pacman"
        [[ "$NEED_WL_CLIPBOARD" == true ]] && DEP_PKGS+=("wl-clipboard")
        [[ "$NEED_PANGO_VIEW" == true ]] && DEP_PKGS+=("pango")
        [[ "$NEED_GST_PIPEWIRE" == true ]] && DEP_PKGS+=("gst-plugin-pipewire")
        [[ "$NEED_GST_BASE" == true ]] && DEP_PKGS+=("gst-plugins-base")
    elif command -v dnf >/dev/null 2>&1; then
        DEP_PKG_MGR="dnf"
        [[ "$NEED_WL_CLIPBOARD" == true ]] && DEP_PKGS+=("wl-clipboard")
        [[ "$NEED_PANGO_VIEW" == true ]] && DEP_PKGS+=("pango")
        [[ "$NEED_GST_PIPEWIRE" == true ]] && DEP_PKGS+=("pipewire-gstreamer")
        [[ "$NEED_GST_BASE" == true ]] && DEP_PKGS+=("gstreamer1-plugins-base")
    elif command -v zypper >/dev/null 2>&1; then
        DEP_PKG_MGR="zypper"
        [[ "$NEED_WL_CLIPBOARD" == true ]] && DEP_PKGS+=("wl-clipboard")
        [[ "$NEED_PANGO_VIEW" == true ]] && DEP_PKGS+=("pango-tools")
        [[ "$NEED_GST_PIPEWIRE" == true ]] && DEP_PKGS+=("gstreamer-plugin-pipewire")
        [[ "$NEED_GST_BASE" == true ]] && DEP_PKGS+=("gstreamer-plugins-base")
    fi
}

format_dep_install_cmd() {
    case "$DEP_PKG_MGR" in
        apt)
            echo "sudo apt-get update && sudo apt-get install -y ${DEP_PKGS[*]}"
            ;;
        pacman)
            echo "sudo pacman -S --needed ${DEP_PKGS[*]}"
            ;;
        dnf)
            echo "sudo dnf install -y ${DEP_PKGS[*]}"
            ;;
        zypper)
            echo "sudo zypper install -y ${DEP_PKGS[*]}"
            ;;
        *)
            echo ""
            ;;
    esac
}

run_dep_install() {
    case "$DEP_PKG_MGR" in
        apt)
            sudo apt-get update && sudo apt-get install -y "${DEP_PKGS[@]}"
            ;;
        pacman)
            sudo pacman -S --needed "${DEP_PKGS[@]}"
            ;;
        dnf)
            sudo dnf install -y "${DEP_PKGS[@]}"
            ;;
        zypper)
            sudo zypper install -y "${DEP_PKGS[@]}"
            ;;
        *)
            return 1
            ;;
    esac
}

ensure_runtime_deps() {
    detect_missing_runtime_deps
    if [[ ${#MISSING_LABELS[@]} -eq 0 ]]; then
        return 0
    fi

    build_dep_install_cmd
    local cmd_str
    cmd_str="$(format_dep_install_cmd)"

    echo "[pinshot] 检测到缺少必需运行依赖: ${MISSING_LABELS[*]}"
    if [[ -z "$cmd_str" ]]; then
        echo "[pinshot] 未识别当前发行版的包管理器，请手动安装 wl-clipboard、pango-view 与 GStreamer PipeWire/base 插件"
        return 1
    fi

    echo "[pinshot] 建议安装命令: ${cmd_str}"

    local should_install=false
    if [[ "$DEPS_MODE" == "yes" ]]; then
        should_install=true
    elif [[ "$DEPS_MODE" == "ask" && -r /dev/tty && -w /dev/tty ]]; then
        local reply=""
        printf '[pinshot] 是否立即使用 sudo 自动安装缺失的运行依赖？[Y/n] ' > /dev/tty
        read -r reply < /dev/tty || reply="n"
        if [[ -z "$reply" || "$reply" =~ ^[Yy]$ ]]; then
            should_install=true
        fi
    fi

    if [[ "$should_install" == true ]]; then
        if run_dep_install; then
            detect_missing_runtime_deps
            if [[ ${#MISSING_LABELS[@]} -eq 0 ]]; then
                echo "[pinshot] 运行依赖已自动安装"
                return 0
            fi
        else
            echo "[pinshot] 自动安装运行依赖失败，请稍后手动执行: ${cmd_str}" >&2
            return 1
        fi
    fi

    return 1
}

gsettings_array_add() {
    local current="$1"
    local item="$2"
    if [[ "$current" == *"'${item}'"* ]]; then
        printf '%s\n' "$current"
        return 0
    fi
    if [[ "$current" == "@as []" || "$current" == "[]" ]]; then
        printf "['%s']\n" "$item"
    else
        printf "%s, '%s']\n" "${current%]}" "$item"
    fi
}

gsettings_array_remove() {
    local current="$1"
    local item="$2"
    if [[ "$current" != *"'${item}'"* ]]; then
        printf '%s\n' "$current"
        return 0
    fi
    local inner="${current#\[}"
    inner="${inner%\]}"
    local result=()
    local entry trimmed
    local IFS=','
    for entry in $inner; do
        trimmed="${entry#"${entry%%[![:space:]]*}"}"
        trimmed="${trimmed%"${trimmed##*[![:space:]]}"}"
        if [[ -n "$trimmed" && "$trimmed" != "'${item}'" ]]; then
            result+=("$trimmed")
        fi
    done
    if [[ ${#result[@]} -eq 0 ]]; then
        printf '@as []\n'
    else
        local joined=""
        for entry in "${result[@]}"; do
            if [[ -n "$joined" ]]; then
                joined+=", ${entry}"
            else
                joined="${entry}"
            fi
        done
        printf '[%s]\n' "$joined"
    fi
}

enable_gnome_extension() {
    local auto_enabled=false
    if command -v gsettings >/dev/null 2>&1 && gsettings writable org.gnome.shell enabled-extensions >/dev/null 2>&1; then
        local enabled disabled updated
        enabled="$(gsettings get org.gnome.shell enabled-extensions 2>/dev/null || true)"
        if [[ -n "$enabled" ]]; then
            updated="$(gsettings_array_add "$enabled" "${EXT_UUID}")"
            if [[ "$updated" != "$enabled" ]]; then
                gsettings set org.gnome.shell enabled-extensions "$updated" 2>/dev/null || true
            fi
            auto_enabled=true
        fi
        disabled="$(gsettings get org.gnome.shell disabled-extensions 2>/dev/null || true)"
        if [[ -n "$disabled" && "$disabled" == *"'${EXT_UUID}'"* ]]; then
            updated="$(gsettings_array_remove "$disabled" "${EXT_UUID}")"
            gsettings set org.gnome.shell disabled-extensions "$updated" 2>/dev/null || true
        fi
        if [[ "$(gsettings get org.gnome.shell disable-user-extensions 2>/dev/null || true)" == "true" ]]; then
            echo "[pinshot] 提示: GNOME 用户扩展总开关已关闭（disable-user-extensions=true），请在「扩展」应用中打开"
        fi
    fi

    if command -v gnome-extensions >/dev/null 2>&1; then
        if gnome-extensions list 2>/dev/null | grep -qx "${EXT_UUID}"; then
            gnome-extensions enable "${EXT_UUID}" 2>/dev/null || true
            echo "[pinshot] 已启用 GNOME 扩展: ${EXT_UUID}（如更新了扩展代码，请注销并重新登录生效）"
        elif [[ "$auto_enabled" == true ]]; then
            echo "[pinshot] 已将 ${EXT_UUID} 加入开机自启扩展列表，注销并重新登录 GNOME 后将自动启用"
        else
            echo "[pinshot] 提示: 请注销并重新登录 GNOME，再执行: gnome-extensions enable ${EXT_UUID}"
        fi
    else
        echo "[pinshot] 提示: 未检测到 gnome-extensions 命令，请确认当前处于 GNOME Shell 桌面环境"
    fi
}

check_gnome_shell_version() {
    if ! command -v gnome-shell >/dev/null 2>&1; then
        return 0
    fi
    local raw_ver major
    raw_ver="$(gnome-shell --version 2>/dev/null || true)"
    major="$(printf '%s\n' "$raw_ver" | sed -n 's/.*GNOME Shell \([0-9][0-9]*\).*/\1/p')"
    if [[ -n "$major" && -f "${EXT_DEST_DIR}/metadata.json" ]]; then
        if ! grep -q "\"${major}\"" "${EXT_DEST_DIR}/metadata.json"; then
            echo "[pinshot] 提示: 当前为 GNOME Shell ${major}，扩展 metadata.json 暂未声明该版本；如需兼容测试可运行:"
            echo "          gsettings set org.gnome.shell disable-extension-version-validation true"
        fi
    fi
}

post_install_checks() {
    enable_gnome_extension
    check_gnome_shell_version

    if [[ -d "/usr/share/gnome-shell/extensions/${EXT_UUID}" ]]; then
        echo "[pinshot] 提示: 检测到系统级扩展 /usr/share/gnome-shell/extensions/${EXT_UUID}，用户级安装会优先覆盖它"
    fi

    if [[ ":${PATH}:" != *":${BIN_DIR}:"* ]]; then
        echo "[pinshot] 提示: 当前终端 PATH 未包含 ${BIN_DIR}，可在 ~/.bashrc 加入: export PATH=\"\$HOME/.local/bin:\$PATH\""
    fi

    local deps_ok=true
    if ! ensure_runtime_deps; then
        deps_ok=false
    fi

    if ! command -v satty >/dev/null 2>&1; then
        echo "[pinshot] 可选提示: 未检测到 satty（标注功能依赖），如需截图/贴图标注可按需安装"
    fi
    if ! command -v notify-send >/dev/null 2>&1; then
        echo "[pinshot] 可选提示: 未检测到 notify-send（libnotify），如需桌面通知可按需安装"
    fi

    if [[ "$deps_ok" == true ]]; then
        echo "[pinshot] 安装完成"
    else
        echo "[pinshot] 文件已安装，但缺少必需运行依赖；请先执行上面的安装命令补齐后再使用"
    fi
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
    if [[ "${EUID}" -eq 0 ]]; then
        echo "[pinshot] 请勿使用 root / sudo 运行此安装脚本；缺失系统依赖时脚本会按需调用 sudo" >&2
        exit 1
    fi

    parse_args "$@"

    local repo_root=""
    if [[ "$FROM_RELEASE" == false ]] && repo_root="$(detect_local_repo)"; then
        install_from_source "$repo_root"
    else
        install_from_release
    fi
}

main "$@"
