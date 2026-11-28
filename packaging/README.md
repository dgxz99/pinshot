# Pinshot 发布与 AUR 打包指南

本文档记录维护者发布 GitHub Release、推送 AUR 包以及配置自定义安装短链的标准流程。

## 1. 本地试打包与验证

在打 tag 之前，可先在本地跑通与 CI 完全一致的发布包组装和安装测试：

```bash
# 生成 dist/pinshot-v0.1.0-linux-x86_64.tar.gz 与 dist/SHA256SUMS
./scripts/package-release.sh v0.1.0 --check-version

# 使用刚生成的本地压缩包测试预编译安装路径
./scripts/install.sh --archive dist/pinshot-v0.1.0-linux-x86_64.tar.gz
```

也可以在 GitHub 仓库的 **Actions → Release → Run workflow** 手动试跑构建，下载 Artifact 验证 `ubuntu-24.04` 基线产物，不会对外发布正式 Release。

## 2. 发布正式 GitHub Release

1. 确认 [Cargo.toml](../Cargo.toml) 中 `[workspace.package].version` 已更新为目标版本（如 `0.1.1`）。
2. 新建对应版本的发布说明 `docs/releases/v0.1.1.md`（参考 [docs/releases/v0.1.0.md](../docs/releases/v0.1.0.md)），并推送到 `main` 等 CI 全部通过。若未创建该文件，Release 流水线会自动回退使用 GitHub 自动生成的变更日志。
3. 创建并推送对应 tag：

```bash
git tag v0.1.1
git push origin v0.1.1
```

[`.github/workflows/release.yml`](../.github/workflows/release.yml) 会自动校验版本号、运行测试、编译包含 `longshot,ocr` 特性的二进制、拉取经 SHA-256 校验的默认离线 OCR 模型（`ppocrv5-mobile`）打进通用 `.tar.gz` 与 Arch `.pkg.tar.zst`，在发布正文底部自动附加折叠的 SHA-256 校验块，并发布对应 GitHub Release（附带 `.tar.gz`、`.pkg.tar.zst` 与 `SHA256SUMS`）。

> 已发布的 tag 和压缩包请勿覆盖重打，以免破坏 AUR 用户的本地缓存校验；如有修复请递增补丁版本号（如 `v0.1.2`）。

## 3. 发布与更新 AUR（`pinshot` / `pinshot-bin`）

本目录提供两个 AUR 包模板：
- `packaging/aur/pinshot/`：从 GitHub tag 源码离线锁定编译
- `packaging/aur/pinshot-bin/`：直接拉取 GitHub Release 预编译包

### 自动发布（推荐）

[`.github/workflows/release.yml`](../.github/workflows/release.yml) 内置了 `publish-aur` 任务。只需在 GitHub 仓库的 **Settings → Secrets and variables → Actions** 配置一次密钥：

- `AUR_SSH_PRIVATE_KEY`（必填）：已添加到 AUR 账号的 SSH 私钥
- `AUR_USERNAME`（可选，默认 `daguo`）：AUR 提交作者名
- `AUR_EMAIL`（可选，默认 `dgxz99@users.noreply.github.com`）：AUR 提交作者邮箱

配置完成后：
- 以后每次推送 `v*` tag，流水线发布 GitHub Release 后会自动计算真实 `sha256sums`、生成 `.SRCINFO` 并推送 `pinshot` 与 `pinshot-bin` 到 AUR（未配置 `AUR_SSH_PRIVATE_KEY` 时自动跳过，不影响 GitHub Release）。
- 如需为已发布的版本补推 AUR，可在 **Actions → Release → Run workflow** 填入版本号（如 `v0.1.1`）并勾选 `publish_aur`。

### 手动发布 / 更新（备选）

首次推送（以 `pinshot-bin` 为例，`pinshot` 流程完全一致）：

```bash
# 1. 克隆空 AUR 仓库（AUR 要求使用 master 分支）
git clone ssh://aur@aur.archlinux.org/pinshot-bin.git /tmp/aur-pinshot-bin
cd /tmp/aur-pinshot-bin

# 2. 拷贝模板（按需更新第 1 行 # Maintainer 邮箱）
cp /path/to/pinshot/packaging/aur/pinshot-bin/* .

# 3. 自动下载上游压缩包写入真实 sha256sums，并生成 .SRCINFO
updpkgsums
makepkg --printsrcinfo > .SRCINFO

# 4. 本地试构建安装验证后清理临时产物
makepkg -si
rm -rf src pkg *.tar.gz *.pkg.tar.*

# 5. 提交并推送
git add PKGBUILD pinshot-bin.install .SRCINFO
git commit -m "Initial release v0.1.0"
git push -u origin master
```

后续更新原则：
- **仅调整打包本身**（依赖、安装路径、`.install` 提示）：保持 `pkgver` 不变，递增 `pkgrel`（如 `1` → `2`），重新生成 `.SRCINFO` 后推送。
- **发布新版本代码**：先发新 GitHub tag（如 `v0.1.1`），再把 `pkgver` 改为 `0.1.1`、`pkgrel` 重置为 `1`，运行 `updpkgsums && makepkg --printsrcinfo > .SRCINFO` 后推送。

## 4. 可选：为 `curl | bash` 配置自定义短域名

默认一键安装直接使用 GitHub Raw 链接：

```bash
curl -fsSL https://raw.githubusercontent.com/dgxz99/pinshot/main/scripts/install.sh | bash
```

如需更短的自定义域名（以 Cloudflare 托管 `example.com` 为例）：
1. 在 DNS 新增记录（如 `get.example.com` → `A` `192.0.2.1`，开启橙色 Proxy）。
2. 在 **Rules → Redirect Rules** 添加 302 重定向规则：将 `https://get.example.com/pinshot` 跳转到 `https://raw.githubusercontent.com/dgxz99/pinshot/main/scripts/install.sh`。
3. 用户即可通过 `curl -fsSL https://get.example.com/pinshot | bash` 安装。
