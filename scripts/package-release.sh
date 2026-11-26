#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"

EXT_UUID="pinshot@dgxz99"
APP_ID="io.github.dgxz99.pinshot"
OUT_DIR="${REPO_ROOT}/dist"
SKIP_BUILD=false
CHECK_VERSION=false
VERSION=""

usage() {
    cat <<EOF
用法: $0 [版本号，如 v0.1.0] [--out-dir DIR] [--skip-build] [--check-version]

选项:
  --out-dir DIR    输出目录（默认: ${REPO_ROOT}/dist）
  --skip-build     跳过 cargo build --release，直接打包已有二进制
  --check-version  校验传入版本号与 Cargo.toml 的 workspace.package.version 一致
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --out-dir)
            OUT_DIR="$2"
            shift 2
            ;;
        --skip-build)
            SKIP_BUILD=true
            shift
            ;;
        --check-version)
            CHECK_VERSION=true
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        -*)
            echo "未知参数: $1" >&2
            exit 1
            ;;
        *)
            if [[ -n "$VERSION" ]]; then
                echo "多余参数: $1" >&2
                exit 1
            fi
            VERSION="$1"
            shift
            ;;
    esac
done

CARGO_VERSION="$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version *= *"\([^"]*\)"/\1/p' "${REPO_ROOT}/Cargo.toml" | head -n 1)"
if [[ -z "$CARGO_VERSION" ]]; then
    echo "[pinshot] 无法从 Cargo.toml 解析版本号" >&2
    exit 1
fi

if [[ -z "$VERSION" ]]; then
    VERSION="v${CARGO_VERSION}"
fi

if [[ "$CHECK_VERSION" == true && "${VERSION#v}" != "$CARGO_VERSION" ]]; then
    echo "[pinshot] 版本不匹配: tag=${VERSION}，Cargo.toml=${CARGO_VERSION}" >&2
    exit 1
fi

ARCH="$(uname -m)"
if [[ "$SKIP_BUILD" == false ]]; then
    echo "[pinshot] 正在编译 release 二进制 (${VERSION}, ${ARCH})..."
    cargo build --release --locked --manifest-path "${REPO_ROOT}/Cargo.toml" -p pinshot-cli --features longshot
fi

BIN_SRC="${REPO_ROOT}/target/release/pinshot"
if [[ ! -x "$BIN_SRC" ]]; then
    echo "[pinshot] 未找到可执行文件: ${BIN_SRC}" >&2
    exit 1
fi

if ! "${BIN_SRC}" --version >/dev/null 2>&1 ||
   ! "${BIN_SRC}" capture-fullscreen --help >/dev/null 2>&1 ||
   ! "${BIN_SRC}" longshot --help >/dev/null 2>&1 ||
   ! "${BIN_SRC}" gif --help >/dev/null 2>&1 ||
   ! "${BIN_SRC}" internal probe-longshot --help >/dev/null 2>&1; then
    echo "[pinshot] Release 二进制子命令自检失败" >&2
    exit 1
fi

if ! command -v glib-compile-schemas >/dev/null 2>&1; then
    echo "[pinshot] 未找到 glib-compile-schemas，请先安装 GLib 工具" >&2
    exit 1
fi

if command -v desktop-file-validate >/dev/null 2>&1; then
    desktop-file-validate "${REPO_ROOT}/data/${APP_ID}.desktop"
fi

STAGE_PARENT="$(mktemp -d)"
trap 'rm -rf "${STAGE_PARENT}"' EXIT

BUNDLE_NAME="pinshot-${VERSION}-linux-${ARCH}"
STAGE_DIR="${STAGE_PARENT}/${BUNDLE_NAME}"

mkdir -p \
    "${STAGE_DIR}/bin" \
    "${STAGE_DIR}/share/applications" \
    "${STAGE_DIR}/share/gnome-shell/extensions" \
    "${STAGE_DIR}/share/icons/hicolor/scalable/apps"

install -m 0755 "${BIN_SRC}" "${STAGE_DIR}/bin/pinshot"
if command -v strip >/dev/null 2>&1; then
    strip --strip-unneeded "${STAGE_DIR}/bin/pinshot" || true
fi

cp -r "${REPO_ROOT}/extensions/${EXT_UUID}" "${STAGE_DIR}/share/gnome-shell/extensions/${EXT_UUID}"
glib-compile-schemas --strict "${STAGE_DIR}/share/gnome-shell/extensions/${EXT_UUID}/schemas"

install -m 0644 "${REPO_ROOT}/data/${APP_ID}.desktop" "${STAGE_DIR}/share/applications/${APP_ID}.desktop"
install -m 0644 "${REPO_ROOT}/extensions/${EXT_UUID}/icons/pinshot.svg" \
    "${STAGE_DIR}/share/icons/hicolor/scalable/apps/${APP_ID}.svg"
install -m 0644 "${REPO_ROOT}/LICENSE" "${STAGE_DIR}/LICENSE"
install -m 0644 "${REPO_ROOT}/README.md" "${STAGE_DIR}/README.md"

mkdir -p "${OUT_DIR}"
ARCHIVE_PATH="${OUT_DIR}/${BUNDLE_NAME}.tar.gz"
tar -C "${STAGE_PARENT}" -czf "${ARCHIVE_PATH}" "${BUNDLE_NAME}"

(
    cd "${OUT_DIR}"
    sha256sum "${BUNDLE_NAME}.tar.gz" > SHA256SUMS
)

echo "[pinshot] 已生成发布包: ${ARCHIVE_PATH}"
echo "[pinshot] 已生成校验文件: ${OUT_DIR}/SHA256SUMS"
