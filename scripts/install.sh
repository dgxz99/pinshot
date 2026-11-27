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
SKIP_DEPS=false
RELEASE_VERSION="${PINSHOT_VERSION:-}"
ARCHIVE_FILE=""
TEMP_DIR=""
PKG_MGR=""
ELEVATE=""
APT_UPDATED=false

cleanup() { [[ -n "$TEMP_DIR" && -d "$TEMP_DIR" ]] && rm -rf "$TEMP_DIR" || true; }
trap cleanup EXIT

usage() {
    cat <<EOF
用法:
  预编译安装: curl -fsSL https://raw.githubusercontent.com/${REPO}/main/scripts/install.sh | bash
  指定版本:   curl -fsSL https://raw.githubusercontent.com/${REPO}/main/scripts/install.sh | PINSHOT_VERSION=v0.1.0 bash
  源码安装:   ./scripts/install.sh [--symlink] [--skip-build]

选项:
  --version VER   安装指定 Release 版本（如 v0.1.0）
  --archive FILE  从本地 Release 压缩包安装
  --symlink       软链接本地扩展目录（开发调试用）
  --skip-build    跳过 cargo build --release（源码模式）
  --no-deps       跳过系统依赖自动安装
  -h, --help      显示帮助
EOF
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --symlink) USE_SYMLINK=true; shift ;;
            --skip-build) SKIP_BUILD=true; shift ;;
            --no-deps) SKIP_DEPS=true; shift ;;
            --version) RELEASE_VERSION="${2:?缺少 --version 参数值}"; shift 2 ;;
            --archive) ARCHIVE_FILE="${2:?缺少 --archive 参数值}"; shift 2 ;;
            -h|--help) usage; exit 0 ;;
            *) echo "未知参数: $1" >&2; exit 1 ;;
        esac
    done
}

detect_local_repo() {
    local src="${BASH_SOURCE[0]:-}"
    [[ -n "$src" && -f "$src" ]] || return 1
    local root
    root="$(cd -- "$(dirname -- "$src")/.." && pwd)"
    [[ -f "${root}/Cargo.toml" && -f "${root}/extensions/${EXT_UUID}/metadata.json" ]] && printf '%s\n' "$root"
}

verify_binary() {
    local bin="$1"
    [[ -x "$bin" ]] || { echo "[pinshot] 未找到可执行文件: ${bin}" >&2; exit 1; }
    for sub in longshot gif capture-fullscreen; do
        if ! "$bin" "$sub" --help >/dev/null 2>&1; then
            echo "[pinshot] 二进制自检失败，请检查 GStreamer 运行库或重新编译" >&2
            exit 1
        fi
    done
}

install_files() {
    local bin_src="$1" ext_src="$2" icon_src="$3" desktop_src="$4"
    verify_binary "$bin_src"

    mkdir -p "${BIN_DIR}" "${EXT_PARENT_DIR}" "${APPS_DIR}" "${ICON_DIR}"
    install -m 0755 "$bin_src" "${BIN_DIR}/pinshot"

    rm -rf "${EXT_DEST_DIR}"
    if [[ "$USE_SYMLINK" == true ]]; then
        ln -s "$ext_src" "${EXT_DEST_DIR}"
    else
        cp -r "$ext_src" "${EXT_DEST_DIR}"
    fi

    install -m 0644 "$icon_src" "${ICON_DEST}"
    sed "s|^Exec=pinshot |Exec=${BIN_DIR}/pinshot |" "$desktop_src" > "${DESKTOP_DEST}"
    chmod 0644 "${DESKTOP_DEST}"

    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "${APPS_DIR}" >/dev/null 2>&1 || true
    command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -f -t "${DATA_DIR}/icons/hicolor" >/dev/null 2>&1 || true
    echo "[pinshot] 已安装程序、扩展与应用入口到 ~/.local"
}

detect_pkg_mgr() {
    if [[ -n "$PKG_MGR" ]]; then return 0; fi
    for mgr in apt-get pacman dnf zypper; do
        if command -v "$mgr" >/dev/null 2>&1; then
            PKG_MGR="${mgr%-get}"
            return 0
        fi
    done
}

has_gst_plugin() {
    local name="$1"; shift
    if command -v gst-inspect-1.0 >/dev/null 2>&1 && gst-inspect-1.0 "$name" >/dev/null 2>&1; then
        return 0
    fi
    local so
    for so in "$@"; do
        compgen -G "/usr/lib*/gstreamer-1.0/${so}" >/dev/null 2>&1 ||
            compgen -G "/usr/lib/*/gstreamer-1.0/${so}" >/dev/null 2>&1 && return 0
    done
    return 1
}

# 将逻辑依赖名映射为当前发行版包名
pkg_name_for() {
    case "${PKG_MGR}:$1" in
        *:wl-clipboard) echo "wl-clipboard" ;;
        apt:pango)      echo "pango1.0-tools" ;;
        zypper:pango)   echo "pango-tools" ;;
        *:pango)        echo "pango" ;;
        apt:pipewire)   echo "gstreamer1.0-pipewire" ;;
        pacman:pipewire) echo "gst-plugin-pipewire" ;;
        dnf:pipewire)   echo "pipewire-gstreamer" ;;
        zypper:pipewire) echo "gstreamer-plugin-pipewire" ;;
        apt:gst-base)   echo "gstreamer1.0-plugins-base" ;;
        pacman:gst-base) echo "gst-plugins-base" ;;
        dnf:gst-base)   echo "gstreamer1-plugins-base" ;;
        zypper:gst-base) echo "gstreamer-plugins-base" ;;
        *:satty)        echo "satty" ;;
    esac
}

detect_elevate() {
    if [[ -n "$ELEVATE" ]]; then return 0; fi
    if command -v sudo >/dev/null 2>&1; then
        ELEVATE="sudo"
    elif command -v doas >/dev/null 2>&1; then
        ELEVATE="doas"
    fi
}

format_pkg_cmd() {
    local prefix="${ELEVATE:+${ELEVATE} }"
    case "$PKG_MGR" in
        apt)    echo "${prefix}apt-get update && ${prefix}apt-get install -y $*" ;;
        pacman) echo "${prefix}pacman -S --needed $*" ;;
        dnf)    echo "${prefix}dnf install -y $*" ;;
        zypper) echo "${prefix}zypper install -y $*" ;;
    esac
}

request_elevate() {
    detect_elevate
    if [[ -z "$ELEVATE" ]]; then
        echo "[pinshot] 未找到 sudo 或 doas，请切换为 root 后手动安装依赖" >&2
        return 1
    fi
    if [[ "$ELEVATE" == "sudo" ]]; then
        sudo -n true 2>/dev/null && return 0
        if [[ -r /dev/tty && -w /dev/tty ]]; then sudo -v < /dev/tty; else sudo -v; fi
    else
        if [[ -r /dev/tty && -w /dev/tty ]]; then doas true < /dev/tty; else doas true; fi
    fi
}

install_pkgs_quiet() {
    local log_file ok=true
    log_file="$(mktemp)"
    case "$PKG_MGR" in
        apt)
            if [[ "$APT_UPDATED" == false ]]; then
                "$ELEVATE" env DEBIAN_FRONTEND=noninteractive apt-get update >"$log_file" 2>&1 && APT_UPDATED=true || ok=false
            fi
            [[ "$ok" == true ]] && { "$ELEVATE" env DEBIAN_FRONTEND=noninteractive apt-get install -y "$@" >>"$log_file" 2>&1 || ok=false; }
            ;;
        pacman) "$ELEVATE" pacman -S --needed --noconfirm "$@" >"$log_file" 2>&1 || ok=false ;;
        dnf)    "$ELEVATE" dnf install -y "$@" >"$log_file" 2>&1 || ok=false ;;
        zypper) "$ELEVATE" zypper --non-interactive install "$@" >"$log_file" 2>&1 || ok=false ;;
        *)      ok=false ;;
    esac

    if [[ "$ok" == true ]]; then
        rm -f "$log_file"
        return 0
    fi
    echo "[pinshot] 安装依赖失败，错误输出如下：" >&2
    tail -n 20 "$log_file" >&2 || true
    rm -f "$log_file"
    return 1
}

pkg_exists_in_repo() {
    case "$PKG_MGR" in
        apt)    apt-cache show "$1" >/dev/null 2>&1 ;;
        pacman) pacman -Si "$1" >/dev/null 2>&1 ;;
        dnf)    dnf info "$1" >/dev/null 2>&1 ;;
        zypper) zypper info "$1" >/dev/null 2>&1 ;;
        *)      return 1 ;;
    esac
}

ensure_runtime_deps() {
    local missing=() keys=()
    if ! command -v wl-copy >/dev/null 2>&1 || ! command -v wl-paste >/dev/null 2>&1; then
        missing+=("wl-clipboard"); keys+=("wl-clipboard")
    fi
    if ! command -v pango-view >/dev/null 2>&1; then
        missing+=("pango-view"); keys+=("pango")
    fi
    if ! has_gst_plugin pipewiresrc libgstpipewire.so; then
        missing+=("GStreamer PipeWire"); keys+=("pipewire")
    fi
    if ! has_gst_plugin videorate libgstvideorate.so || ! has_gst_plugin videoconvert libgstvideoconvertscale.so libgstvideoconvert.so; then
        missing+=("GStreamer plugins-base"); keys+=("gst-base")
    fi
    [[ ${#keys[@]} -eq 0 ]] && return 0

    detect_pkg_mgr
    detect_elevate
    local pkgs=() k
    for k in "${keys[@]}"; do
        [[ -n "$PKG_MGR" ]] && pkgs+=("$(pkg_name_for "$k")")
    done
    local cmd_str=""
    [[ ${#pkgs[@]} -gt 0 ]] && cmd_str="$(format_pkg_cmd "${pkgs[@]}")"

    echo "[pinshot] 缺少运行依赖：${missing[*]}"
    if [[ -z "$cmd_str" ]]; then
        echo "[pinshot] 未识别包管理器，请手动安装上述运行依赖" >&2
        exit 1
    fi
    if [[ "$SKIP_DEPS" == true ]]; then
        echo "[pinshot] 请手动执行：${cmd_str}"
        return 1
    fi

    echo "[pinshot] 将执行：${cmd_str}"
    echo "[pinshot] 正在安装运行依赖..."
    if ! request_elevate || ! install_pkgs_quiet "${pkgs[@]}"; then
        echo "[pinshot] 请手动执行：${cmd_str}" >&2
        exit 1
    fi
    echo "[pinshot] 运行依赖安装完成"
}

install_satty_from_release() {
    local arch satty_tmp archive_path url satty_bin check_err=""
    arch="$(uname -m)"
    satty_tmp="$(mktemp -d)"
    archive_path="${satty_tmp}/satty.tar.gz"
    url="https://github.com/Satty-org/Satty/releases/latest/download/satty-${arch}-unknown-linux-gnu.tar.gz"

    if ! curl -fsSL "$url" -o "$archive_path" 2>/dev/null; then
        url="$(curl -fsSL "https://api.github.com/repos/Satty-org/Satty/releases/latest" 2>/dev/null \
            | grep -oE "https://[^\"]+${arch}[^\"]*linux[^\"]*\.tar\.gz" | head -n 1 || true)"
        if [[ -z "$url" ]] || ! curl -fsSL "$url" -o "$archive_path" 2>/dev/null; then
            rm -rf "$satty_tmp"
            return 1
        fi
    fi

    if ! tar -xzf "$archive_path" -C "$satty_tmp" 2>/dev/null; then
        rm -rf "$satty_tmp"
        return 1
    fi
    satty_bin="$(find "$satty_tmp" -type f -name satty | head -n 1)"
    if [[ -z "$satty_bin" ]]; then
        rm -rf "$satty_tmp"
        return 1
    fi
    mkdir -p "${BIN_DIR}"
    install -m 0755 "$satty_bin" "${BIN_DIR}/satty"
    rm -rf "$satty_tmp"

    if ! check_err="$("${BIN_DIR}/satty" --version 2>&1)"; then
        rm -f "${BIN_DIR}/satty"
        [[ -n "$check_err" ]] && echo "[pinshot] satty 运行自检失败：${check_err}" >&2
        return 1
    fi
}

offer_optional_satty() {
    if command -v satty >/dev/null 2>&1 || [[ -x "${BIN_DIR}/satty" ]]; then
        return 0
    fi
    detect_pkg_mgr
    local method="" arch
    arch="$(uname -m)"
    if pkg_exists_in_repo satty; then
        method="pkg"
    elif command -v curl >/dev/null 2>&1 && command -v tar >/dev/null 2>&1 && [[ "$arch" == "x86_64" || "$arch" == "aarch64" ]]; then
        method="release"
    fi

    if [[ "$SKIP_DEPS" == false && -r /dev/tty && -w /dev/tty && -n "$method" ]]; then
        local reply=""
        printf '[pinshot] 可选组件 satty（截图/贴图标注）未安装，是否一并安装？[y/N] ' > /dev/tty
        read -r reply < /dev/tty || reply="n"
        if [[ "$reply" =~ ^[Yy]$ ]]; then
            if [[ "$method" == "pkg" ]]; then
                echo "[pinshot] 正在安装 satty..."
                if request_elevate && install_pkgs_quiet satty; then
                    echo "[pinshot] satty 安装完成"
                else
                    echo "[pinshot] satty 安装失败，可稍后手动执行：$(format_pkg_cmd satty)" >&2
                fi
            else
                echo "[pinshot] 正在下载安装 satty 到 ${BIN_DIR}/satty..."
                if install_satty_from_release; then
                    echo "[pinshot] satty 安装完成：${BIN_DIR}/satty"
                else
                    echo "[pinshot] satty 安装失败，可前往 https://github.com/Satty-org/Satty/releases 手动安装" >&2
                fi
            fi
        fi
    else
        echo "[pinshot] 可选组件：未安装 satty（标注功能，可按需安装：https://github.com/Satty-org/Satty/releases）"
    fi
}

enable_gnome_extension() {
    local major="" version_ok=true ignore_ver="false"
    if command -v gnome-shell >/dev/null 2>&1 && [[ -f "${EXT_DEST_DIR}/metadata.json" ]]; then
        major="$(gnome-shell --version 2>/dev/null | sed -n 's/.*GNOME Shell \([0-9][0-9]*\).*/\1/p')"
        if [[ -n "$major" ]] && ! grep -q "\"${major}\"" "${EXT_DEST_DIR}/metadata.json"; then
            version_ok=false
        fi
    fi

    local auto_enabled=false
    if command -v gsettings >/dev/null 2>&1 && gsettings writable org.gnome.shell enabled-extensions >/dev/null 2>&1; then
        ignore_ver="$(gsettings get org.gnome.shell disable-extension-version-validation 2>/dev/null || echo false)"
        local enabled disabled updated
        enabled="$(gsettings get org.gnome.shell enabled-extensions 2>/dev/null || true)"
        if [[ -n "$enabled" ]]; then
            if [[ "$enabled" != *"'${EXT_UUID}'"* ]]; then
                updated="$([[ "$enabled" == "@as []" || "$enabled" == "[]" ]] && echo "['${EXT_UUID}']" || echo "${enabled%]}, '${EXT_UUID}']")"
                gsettings set org.gnome.shell enabled-extensions "$updated" 2>/dev/null || true
            fi
            auto_enabled=true
        fi
        disabled="$(gsettings get org.gnome.shell disabled-extensions 2>/dev/null || true)"
        if [[ "$disabled" == *"'${EXT_UUID}'"* ]]; then
            updated="$(printf '%s' "$disabled" | sed "s/'${EXT_UUID}', *//g; s/, *'${EXT_UUID}'//g; s/\['${EXT_UUID}'\]/@as []/")"
            gsettings set org.gnome.shell disabled-extensions "$updated" 2>/dev/null || true
        fi
        if [[ "$(gsettings get org.gnome.shell disable-user-extensions 2>/dev/null || true)" == "true" ]]; then
            echo "[pinshot] 提示：GNOME 用户扩展总开关已关闭，请在「扩展」应用中打开"
        fi
    fi

    if [[ "$version_ok" == false && "$ignore_ver" != "true" ]]; then
        echo "[pinshot] 提示：当前为 GNOME Shell ${major}，扩展暂未声明该版本；兼容测试可运行："
        echo "          gsettings set org.gnome.shell disable-extension-version-validation true"
        return 0
    fi

    if command -v gnome-extensions >/dev/null 2>&1 \
        && gnome-extensions list 2>/dev/null | grep -qx "${EXT_UUID}" \
        && gnome-extensions enable "${EXT_UUID}" 2>/dev/null; then
        echo "[pinshot] 已启用扩展"
    elif [[ "$auto_enabled" == true ]]; then
        echo "[pinshot] 已设为自启，注销并重新登录 GNOME 后自动生效"
    else
        echo "[pinshot] 提示：注销并重新登录 GNOME 后运行：gnome-extensions enable ${EXT_UUID}"
    fi
}

post_install() {
    local deps_ok=true
    ensure_runtime_deps || deps_ok=false
    offer_optional_satty
    command -v notify-send >/dev/null 2>&1 || echo "[pinshot] 可选组件：未安装 notify-send（桌面通知，可按需安装）"

    enable_gnome_extension

    [[ -d "/usr/share/gnome-shell/extensions/${EXT_UUID}" ]] &&
        echo "[pinshot] 提示：检测到系统级扩展 /usr/share/gnome-shell/extensions/${EXT_UUID}，用户级安装会优先覆盖它"
    if [[ ":${PATH}:" != *":${BIN_DIR}:"* ]]; then
        if [[ "${SHELL:-}" == */fish ]]; then
            echo "[pinshot] 提示：可将 ${BIN_DIR} 加入 PATH：fish_add_path ~/.local/bin"
        else
            echo "[pinshot] 提示：可将 ${BIN_DIR} 加入 PATH：export PATH=\"\$HOME/.local/bin:\$PATH\""
        fi
    fi

    if [[ "$deps_ok" == true ]]; then
        echo "[pinshot] 安装完成"
    else
        echo "[pinshot] 文件已安装，补齐上述运行依赖后即可使用"
    fi
}

install_from_source() {
    local root="$1"
    local ext_src="${root}/extensions/${EXT_UUID}"
    if [[ "$SKIP_BUILD" == false ]]; then
        echo "[pinshot] 正在编译 release 二进制..."
        cargo build --release --locked --manifest-path "${root}/Cargo.toml" -p pinshot-cli --features longshot
    fi
    command -v glib-compile-schemas >/dev/null 2>&1 || { echo "[pinshot] 未找到 glib-compile-schemas" >&2; exit 1; }
    glib-compile-schemas --strict "${ext_src}/schemas"
    install_files "${root}/target/release/pinshot" "$ext_src" "${ext_src}/icons/pinshot.svg" "${root}/data/${APP_ID}.desktop"
    post_install
}

install_from_release() {
    [[ "$USE_SYMLINK" == false && "$SKIP_BUILD" == false ]] || {
        echo "[pinshot] --symlink 与 --skip-build 仅适用于源码安装模式" >&2; exit 1
    }
    command -v tar >/dev/null 2>&1 || { echo "[pinshot] 未找到 tar 命令" >&2; exit 1; }

    TEMP_DIR="$(mktemp -d)"
    local archive_path="$ARCHIVE_FILE"

    if [[ -n "$archive_path" ]]; then
        [[ -f "$archive_path" ]] || { echo "[pinshot] 未找到压缩包: ${archive_path}" >&2; exit 1; }
        echo "[pinshot] 正在从本地发布包安装: ${archive_path}"
    else
        for cmd in curl sha256sum; do
            command -v "$cmd" >/dev/null 2>&1 || { echo "[pinshot] 未找到 ${cmd} 命令" >&2; exit 1; }
        done
        local arch version archive_name base_url latest_url
        arch="$(uname -m)"
        [[ "$arch" == "x86_64" ]] || { echo "[pinshot] 暂仅提供 x86_64 预编译包，请从源码构建" >&2; exit 1; }

        if [[ -n "$RELEASE_VERSION" ]]; then
            version="$RELEASE_VERSION"
        else
            echo "[pinshot] 正在获取最新版本..."
            latest_url="$(curl -fsSL -o /dev/null -w '%{url_effective}' "https://github.com/${REPO}/releases/latest")"
            version="${latest_url##*/}"
            [[ -n "$version" && "$version" != "releases" && "$version" != "latest" ]] || {
                echo "[pinshot] 无法解析最新 Release 版本" >&2; exit 1
            }
        fi
        [[ "$version" == v* ]] || version="v${version}"

        archive_name="pinshot-${version}-linux-${arch}.tar.gz"
        base_url="https://github.com/${REPO}/releases/download/${version}"
        archive_path="${TEMP_DIR}/${archive_name}"

        echo "[pinshot] 正在下载并校验 ${archive_name}..."
        curl -fsSL "${base_url}/${archive_name}" -o "${archive_path}"
        curl -fsSL "${base_url}/SHA256SUMS" -o "${TEMP_DIR}/SHA256SUMS"
        (cd "${TEMP_DIR}" && grep "  ${archive_name}$" SHA256SUMS | sha256sum -c - >/dev/null)
    fi

    local extract_dir="${TEMP_DIR}/extract"
    mkdir -p "$extract_dir"
    tar -xzf "$archive_path" -C "$extract_dir"

    local bundle_dir
    bundle_dir="$(find "$extract_dir" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
    [[ -n "$bundle_dir" && -x "${bundle_dir}/bin/pinshot" ]] || { echo "[pinshot] 发布包结构无效" >&2; exit 1; }

    install_files \
        "${bundle_dir}/bin/pinshot" \
        "${bundle_dir}/share/gnome-shell/extensions/${EXT_UUID}" \
        "${bundle_dir}/share/icons/hicolor/scalable/apps/${APP_ID}.svg" \
        "${bundle_dir}/share/applications/${APP_ID}.desktop"
    post_install
}

main() {
    if [[ "${EUID}" -eq 0 ]]; then
        echo "[pinshot] 请使用普通用户运行此脚本；安装系统依赖时会自动请求提权" >&2
        exit 1
    fi
    parse_args "$@"

    local repo_root=""
    if [[ -z "$RELEASE_VERSION" && -z "$ARCHIVE_FILE" ]] && repo_root="$(detect_local_repo)"; then
        install_from_source "$repo_root"
    else
        install_from_release
    fi
}

main "$@"
