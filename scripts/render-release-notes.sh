#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 3 || $# -gt 4 ]]; then
    echo "用法: $0 <tag> <dist_dir> <output_md> [fallback_notes_file]" >&2
    exit 1
fi

TAG="$1"
DIST_DIR="$2"
OUTPUT_MD="$3"
FALLBACK_NOTES="${4:-}"

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
CUSTOM_NOTES="${REPO_ROOT}/docs/releases/${TAG}.md"
SUMS_FILE="${DIST_DIR}/SHA256SUMS"
COMMIT_SHA="${GITHUB_SHA:-$(git -C "${REPO_ROOT}" rev-parse HEAD 2>/dev/null || true)}"

{
    if [[ -f "${CUSTOM_NOTES}" ]]; then
        cat "${CUSTOM_NOTES}"
    elif [[ -n "${FALLBACK_NOTES}" && -f "${FALLBACK_NOTES}" ]]; then
        cat "${FALLBACK_NOTES}"
    else
        printf '## Pinshot %s\n' "${TAG}"
    fi

    printf '\n'
    if [[ -n "${COMMIT_SHA}" ]]; then
        printf 'Source / 源码: `%s`\n\n' "${COMMIT_SHA}"
    fi

    if [[ -f "${SUMS_FILE}" ]]; then
        cat <<'EOF'
<details>
<summary>Files and SHA-256 / 文件列表与 SHA-256</summary>

```text
EOF
        cat "${SUMS_FILE}"
        cat <<'EOF'
```

</details>
EOF
    fi
} > "${OUTPUT_MD}"
