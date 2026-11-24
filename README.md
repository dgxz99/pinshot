# Pinshot

Pinshot 是一款面向 Wayland 的截图、滚动长截图和贴图工具。它可以把截图、剪贴板中的图片或文字贴在桌面上，方便对照文档、代码和聊天内容。贴图由 GNOME Shell 扩展绘制，不是普通窗口，不会出现在 `Alt+Tab` 列表里。

目前支持 **GNOME Shell 51（Wayland）**，已在 Arch Linux / GNOME Shell 51.1 上验证。

## 安装

先准备以下工具：

| 工具 | 用途 |
| --- | --- |
| GNOME Shell 51、`gnome-extensions` | 运行和管理扩展 |
| Rust / Cargo、`glib-compile-schemas` | 构建 CLI 和编译扩展设置，开发环境使用 Rust 1.100 |
| `wl-paste`、`wl-copy`（`wl-clipboard`） | 读取和写入 Wayland 剪贴板 |
| `pango-view`（Pango） | 将剪贴板文字渲染为贴图 |
| GNOME ScreenCast Portal、PipeWire、GStreamer | 滚动长截图；构建需要 `pkg-config` 和 GStreamer Core、App、Video 开发库，运行需要 `pipewiresrc`、`videorate`、`videoconvert` 插件 |
| `satty` | 可选，用于标注；已验证 Satty 0.22.0 |
| `notify-send`（libnotify） | 可选，用于 Pinshot 桌面通知 |

Arch Linux 上，长截图所需的 Portal 和 GStreamer 组件可通过 `xdg-desktop-portal-gnome`、`pipewire`、`gstreamer`、`gst-plugins-base`、`gst-plugins-base-libs`、`gst-plugin-pipewire` 与 `pkgconf` 软件包提供。

在仓库根目录执行：

```bash
./scripts/install.sh
```

脚本把 CLI 安装到 `~/.local/bin/pinshot`，扩展安装到 `~/.local/share/gnome-shell/extensions/pinshot@dgxz99`。不需要用 `sudo` 运行脚本。首次安装或更新扩展代码后，**注销并重新登录 GNOME**，然后启用并检查：

```bash
gnome-extensions enable pinshot@dgxz99
~/.local/bin/pinshot ping
```

返回 `gnome-shell-51` 表示扩展已在线。下文假设 `~/.local/bin` 已加入 `PATH`；如果 `pinshot` 找不到，就使用 `~/.local/bin/pinshot`。

开发时可用 `./scripts/install.sh --symlink`，将扩展目录链接到当前源码。更新 Rust 代码后仍需重装 CLI。`--skip-build` 只安装已经构建好的 `target/release/pinshot`，不会编译新代码。

## 快速上手

| 场景 | 默认快捷键 | 等效命令 |
| --- | --- | --- |
| 截图并贴到桌面 | `Super+Shift+P` | `pinshot capture-pin` |
| 滚动长截图并贴图 | `Super+Shift+L` | `pinshot longshot` |
| 把剪贴板图片或文字贴到桌面 | `Super+Shift+V` | `pinshot pin-clipboard` |
| 解除全部贴图的点击穿透 | `Super+Shift+U` | `pinshot unghost-all` |
| 截图后进入 Satty 标注 | 未绑定 | `pinshot capture-edit` |
| 显示或隐藏全部贴图 | 未绑定 | `pinshot toggle-hide` |
| 恢复最近关闭的一张贴图 | 未绑定 | `pinshot restore-last` |

- **截图贴图**：在冻结的桌面画面上拖出选框。松手后，拖四角或四边中点可缩放，拖框内可移动。选框旁的工具条提供「复制」(`Ctrl+C`，不贴图、不保存)、「贴图」(`P`，复制并贴图) 和「编辑」(`E`，打开 Satty)。从 `Super+Shift+P` 进入时，`Enter` 默认贴图；按 `Esc` 或右键可取消。不弹共享授权；窗口或整屏截图仍可用 GNOME 自带的 `Print`。
- **剪贴板贴图**：先在原应用用 `Ctrl+C` 复制文字或图片，再触发贴图。例如复制一段代码，就能把它贴在编辑器旁边。Pinshot 不会接管其他应用的复制快捷键。Pinshot 复制的截图再次贴出时会保留原始大小（大图仍会适屏）。如果用其他工具截的图在高分屏上贴出偏大，可到 Pinshot 首选项「截图 → 剪贴板图片」开启「外部图片按屏幕像素 1:1 显示」。
- **截图后标注**：运行 `pinshot capture-edit`，框选后按 `Enter` 默认打开 Satty，也可以点工具条上的其他动作。进入 Satty 后，按 `Enter` 保存并复制标注结果；`Esc` 退出且不改变剪贴板。已有贴图则用右键菜单里的「标注编辑 (satty)」，保存后会更新那张贴图。

`Super` 通常是键盘上的 Windows 键。到 **GNOME「扩展」→ Pinshot → 齿轮 →「快捷键」**，可以更改、禁用或恢复默认绑定。也可用 `gnome-extensions prefs pinshot@dgxz99` 打开该面板。若已在 GNOME 系统设置中为同一个命令绑了键，可以继续用原来的绑定，不必再为同一动作配置扩展快捷键。

**快捷键 → 框选内快捷键**可单独设置复制、贴图和编辑。例如把框选内的「贴图」改为 `Ctrl+T` 后，在选框里会贴出当前选区；退出框选后，`Ctrl+T` 仍按原来的全局设置工作。`Enter` 和 `Esc` 保留给确认和取消。

### 滚动长截图

1. 打开要截取的页面并滚到起点，按 `Super+Shift+L`。
2. GNOME 弹出共享对话框时，选择页面所在的显示器。想下次跳过此步骤，就在系统对话框中勾选记住选择，再点「分享」。
3. 拖动鼠标框住正文，避开固定页眉、侧栏和滚动条。调整选框后点「开始录制」或按 `Enter`；框选阶段按 `Esc` 可取消。
4. 看到控制条提示「可以滚动」后，在正文里慢慢向下滚。若提示回滚，先回到上一段已经接上的位置，再慢一点向下滚。
5. 点「完成」复制并贴出长图；点「取消」不保存、不改剪贴板。录制期间已有贴图会暂时隐藏，结束后恢复。若启用了全屏避让，退出全屏后才能看到所在屏幕上的贴图。

想换屏时，运行 `pinshot longshot --choose-screen`，或在「快捷键」里为「滚动长截图：重新选择屏幕」设置按键。显示器不再可用或授权被撤销时，GNOME 会重新询问。记住选择不会让 Pinshot 在后台继续录屏。

长截图一次只录制一个显示器。静止和回滚不会重复拼接。框选后蓝色边框固定在屏幕上，不挡滚动；边框和控制条都不会被截进长图。达到 24,000 像素高或内存上限时会自动完成；从 GNOME 停止共享则会取消本次录制。

### 操作贴图

| 操作 | 如何触发 |
| --- | --- |
| 选中并置顶 / 移动 | 左键单击 / 左键拖动 |
| 关闭并保留历史 | 左键双击或单击中键 |
| 缩放 / 调不透明度 | 滚轮 / `Ctrl+滚轮` |
| 复制、标注、保存、恢复原尺寸 | 右键打开操作菜单 |
| 收起或展开迷你态 | 右键菜单，或贴图收到键盘事件时按 `Shift+R` |
| 开启点击穿透 | 右键菜单，或贴图收到键盘事件时按 `T` |

贴图收到键盘事件时，还可用 `Ctrl+C` 复制图片、`E` 标注、`0` 恢复 100%、`Esc` 关闭。如果按键仍进入后方的应用，使用贴图右键菜单即可。穿透后贴图不再接收点击，按 `Super+Shift+U` 或运行 `pinshot unghost-all` 恢复。

## 截图保存与通知

打开扩展首选项的 **「截图」** 页：

- **自动保存到目录**：默认关闭。截图贴图仍保留在历史缓存，但不会额外导出 PNG；需要长期保留时，可右键「保存原图」。开启后，截图贴图和滚动长截图会自动导出原图，截图后标注只导出确认后的标注图。
- **保存目录**：可填绝对路径或 `~/` 开头的路径。留空使用配置文件中的 `general.save_dir`，默认是 `~/Pictures/Pinshots`。
- **文件名前缀**：默认 `pinshot`。自动保存和手动「保存原图」都会加上保存时的本地时间，如 `pinshot-20261121-214530.png`。同一秒重名会追加 `-1`、`-2`，不会覆盖已有文件。
- **显示截图完成通知**：默认关闭，只控制从 Pinshot 发起的截图完成通知；不影响手动保存提示，也不影响直接按 `Print` 时 GNOME 自己的保存或通知。

修改目录或文件名前缀后，点击输入框旁的应用按钮保存设置。从旧版更新时，原先的默认目录值会按新默认值使用；自定义目录不变，旧图不会被移动。

## 缓存、配置和历史

| 内容 | 默认位置 |
| --- | --- |
| 历史缓存 | `~/.cache/pinshot/`；设置了 `XDG_CACHE_HOME` 时使用 `$XDG_CACHE_HOME/pinshot/` |
| Core 配置 | `~/.config/pinshot/config.toml`；设置了 `XDG_CONFIG_HOME` 时使用 `$XDG_CONFIG_HOME/pinshot/config.toml` |
| 扩展快捷键和截图设置 | GNOME 的 Pinshot 首选项（GSettings） |
| 导出的永久截图 | 默认 `~/Pictures/Pinshots/`，可在首选项中覆盖 |

用 `pinshot status` 查看实际缓存路径、历史条数和前端状态。缓存里 `images/` 放原图，`thumbs/` 放缩略图，`texts/` 放文字贴图的原文，`index.json` 是历史索引。缓存里的文件名使用内部编号，不是导出文件名；也不要把历史缓存当作永久相册。

默认缓存策略是 **200 MB、14 天**，优先清理非活跃条目。可在 `config.toml` 的 `[cache]` 下调整 `max_size_mb` 和 `max_age_days`。默认不会在下次登录时自动重新弹出贴图，仍可运行 `pinshot restore-last` 找回最近一张。

```bash
pinshot clear-history       # 清理缓存历史，保留活跃贴图
pinshot clear-history --all # 同时关闭并删除活跃贴图的缓存，不可撤销
```

这两条命令都不会删除已经导出到保存目录的 PNG。

## 配置文件示例

运行一次 `pinshot status` 会在需要时生成 `config.toml`，并打印实际路径。**快捷键、截图自动保存、文件名前缀以及截图完成通知在扩展首选项里改**；下面是配置文件中常用的项目，可按需修改：

```toml
[general]
save_dir = "~/Pictures/Pinshots"    # 首选项未指定保存目录时使用
restore_on_startup = false         # 登录时是否恢复上次未关闭的贴图
notify_on_save = true              # 右键「保存原图」成功后的通知

[cache]
max_size_mb = 200                   # 缓存大小上限，优先清理非活跃条目
max_age_days = 14                   # 非活跃条目保留天数

[pin]
default_opacity = 1.0               # 新贴图的不透明度；1.0 为完全不透明
min_scale = 0.2                     # 最小缩放比例
max_scale = 5.0                     # 最大缩放比例
zoom_step = 0.1                     # 每格滚轮的缩放步长
opacity_step = 0.08                 # Ctrl+滚轮每格调整的不透明度
mini_size_px = 64                   # 迷你态尺寸
hide_on_fullscreen = true           # 全屏应用所在显示器临时隐藏贴图

[text_card]
theme = "auto"                      # auto、light 或 dark
max_width_px = 680                  # 文字卡片最大宽度
max_lines = 80                      # 最多渲染的行数
prose_font = "Sans 12"              # 普通文字字体
code_font = "Monospace 11"          # 代码字体
padding_px = 16                     # 卡片内边距
```

例如想让文字卡片始终用深色主题，把 `theme` 改成 `"dark"`，下一次从剪贴板创建文字贴图时生效。`[pin]` 由扩展启用时加载，修改后重新登录 GNOME 即可生效，无需重编项目。

## 项目架构

| 部分 | 负责什么 |
| --- | --- |
| GNOME Shell 扩展（`extensions/pinshot@dgxz99`） | 打开 GNOME 原生截图界面、绘制贴图，处理鼠标操作与全局快捷键，提供首选项 |
| Rust CLI（`crates/pinshot-cli`） | 提供 `pinshot` 命令，通过会话 D-Bus 与扩展通信 |
| Rust Core（`crates/pinshot-core`） | 处理图像、剪贴板、Satty 标注、缓存与 PNG 导出 |

以「截图并贴图」为例：扩展收到快捷键后启动 CLI；CLI 请求扩展打开截图界面；确认后扩展把 PNG 交给 Core，由它更新历史并请求扩展显示贴图。CLI 处理完就退出，不需要额外运行后台守护进程。

## 排查与卸载

- `pinshot ping` 失败：确认当前是 GNOME Shell **51** 的 Wayland 会话，运行 `gnome-extensions info pinshot@dgxz99` 查看状态。更新后仍加载旧代码时，注销并重新登录。
- 快捷键没反应：先用对应的 CLI 命令验证功能，再到扩展首选项检查是否禁用或与系统绑定冲突。
- 排查扩展错误：运行 `journalctl --user -b -o cat | grep -i pinshot` 查看本次启动的日志。
- 卸载运行 `./scripts/uninstall.sh`，默认保留缓存和配置；加 `--purge` 会额外删除 Pinshot 的缓存目录和 Core 配置目录。已导出的 PNG 不受影响。
