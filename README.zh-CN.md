<div align="center">
  <img src="extensions/pinshot@dgxz99/icons/pinshot.svg" width="112" height="112" alt="Pinshot 标志" />
  <h1>Pinshot</h1>
  <p><strong>在 GNOME Wayland 上截图、贴图和录制 GIF。</strong></p>
  <p><a href="README.md">English</a> · <strong>简体中文</strong></p>
  <p>
    <a href="#安装"><img src="https://img.shields.io/badge/GNOME%20Shell-51-4A86CF?style=flat-square&amp;logo=gnome&amp;logoColor=white" alt="GNOME Shell 51" /></a>
    <a href="#安装"><img src="https://img.shields.io/badge/Wayland-F29B26?style=flat-square&amp;logo=wayland&amp;logoColor=white" alt="Wayland" /></a>
    <a href="Cargo.toml"><img src="https://img.shields.io/badge/Rust-B7410E?style=flat-square&amp;logo=rust&amp;logoColor=white" alt="Rust" /></a>
    <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-2C8A5B?style=flat-square" alt="MIT 许可证" /></a>
  </p>
  <p><a href="#功能">功能</a> · <a href="#安装">安装</a> · <a href="#快速上手">快速上手</a> · <a href="https://github.com/dgxz99/pinshot/issues">反馈问题</a></p>
</div>

把截图、图片或刚复制的代码贴在桌面上，对照参考资料时不用来回切窗口。贴图默认在普通窗口上方，切换工作区后仍可见。它由GNOME Shell直接绘制，不会出现在`Alt+Tab`列表里。

**支持环境：GNOME Shell 51（Wayland）**。已在Arch Linux / GNOME Shell 51.1上验证。

![GNOME Wayland 中 Pinshot 的可调截图选区和操作工具条](docs/images/region-capture.png)

## 功能

| 功能 | 可以做什么 |
| --- | --- |
| 区域截图 | 冻结画面，拖动八个手柄调整选区，再复制、保存、贴图或标注。不接管GNOME自带的`Print`。 |
| 桌面贴图 | 把截图、剪贴板图片或文字卡片固定在桌面上，可移动、缩放、调不透明度、收起或开启点击穿透。 |
| 滚动长截图 | 纵向或横向截取网页与滚动容器，可沿所选轴向两端补图，来回滚动不重复拼接。 |
| GIF录制 | 开录前选帧率，录入鼠标指针，停止后保存为循环GIF。 |
| 标注 | 用[Satty](https://github.com/Satty-org/Satty)标注选区或编辑已有贴图。 |

## 安装

### 环境与依赖

| 依赖 | 用途 |
| --- | --- |
| GNOME Shell 51、`gnome-extensions` | 运行和管理扩展 |
| Git、Rust与Cargo（已用Rust 1.100验证）、C工具链、`glib-compile-schemas` | 构建CLI和编译扩展设置 |
| `wl-clipboard`、`pango-view`（Pango） | 读取剪贴板和生成文字卡片 |
| GNOME ScreenCast Portal、PipeWire、GStreamer | 长截图和GIF录制。构建需要`pkg-config`和GStreamer Core、App、Video开发库，运行需要`pipewiresrc`、`videorate`、`videoconvert`。 |
| `satty`、`notify-send`（libnotify） | 可选，用于标注和桌面通知 |

在Arch Linux上，如果已安装GNOME和Rust，其余必要的工具和库可通过以下软件包安装：

```bash
sudo pacman -S --needed git base-devel glib2 wl-clipboard pango xdg-desktop-portal-gnome \
  pipewire wireplumber gstreamer gst-plugins-base gst-plugins-base-libs gst-plugin-pipewire
```

需要标注或桌面通知时，再安装`satty`和`libnotify`。

### 从源码安装

```bash
git clone https://github.com/dgxz99/pinshot.git
cd pinshot
./scripts/install.sh
```

用普通用户运行安装脚本。CLI装到`~/.local/bin/pinshot`，扩展装到`~/.local/share/gnome-shell/extensions/pinshot@dgxz99`。首次安装或更新了扩展代码后，**注销并重新登录GNOME**，再启用扩展并检查连接：

```bash
gnome-extensions enable pinshot@dgxz99
~/.local/bin/pinshot ping
```

返回`gnome-shell-51`表示扩展已在线。把`~/.local/bin`加入`PATH`后，即可直接运行下文的`pinshot`命令；也可以继续用完整路径。

更新时拉取最新代码，重新运行`./scripts/install.sh`。开发时可加`--symlink`，让扩展目录链接到源码。`--skip-build`只安装已有的`target/release/pinshot`，不会重新编译。

## 快速上手

| 操作 | 默认快捷键 | 等效命令 |
| --- | --- | --- |
| 区域截图 | `Super+Shift+P` | `pinshot capture` |
| 纵向长截图 | `Super+Shift+L` | `pinshot longshot` |
| 把剪贴板图片或文字贴到桌面 | `Super+Shift+V` | `pinshot pin-clipboard` |
| 解除全部贴图的点击穿透 | `Super+Shift+U` | `pinshot unghost-all` |
| 录制GIF | 未绑定 | `pinshot gif` |
| 截图后标注 | 未绑定 | `pinshot capture-edit` |
| 显示或隐藏全部贴图 | 未绑定 | `pinshot toggle-hide` |
| 恢复最近关闭的贴图 | 未绑定 | `pinshot restore-last` |

`Super`通常是键盘上的Windows键。运行`gnome-extensions prefs pinshot@dgxz99`打开扩展首选项，在「快捷键」里修改或禁用绑定，在「框选内快捷键」里单独设置框选时的绑定。

### 截图、复制或贴图

1. 按`Super+Shift+P`拖出选区。松手后可以拖框内移动，或拉动手柄调整大小。
2. 工具条从左到右是**长截图 → GIF → 标注 → 贴图 → 保存 → 取消 → 复制**。`Enter`只复制，`Esc`或右键取消。点保存只导出PNG，不改剪贴板；点贴图会把选区贴到桌面并复制。想让`Enter`默认贴图，可以运行`pinshot capture-pin`。
3. 框选时默认`Ctrl+C`复制、`P`贴图、`E`打开Satty。这三个局部快捷键可独立设置，例如把`Ctrl+T`设为贴出当前选区，不会改变它在框选之外的行为。进入Satty后，按`Enter`保存并复制标注结果，按`Esc`退出且不改剪贴板。

### 操作贴图

在原应用复制图片或文字，再按`Super+Shift+V`。Pinshot不接管其他应用的`Ctrl+C`。经Pinshot复制的截图再次贴出时会保留原始显示大小，大图仍会适屏。高分屏下的外部图片可在「截图 → 剪贴板图片」开启按设备像素1:1显示。

| 操作 | 如何触发 |
| --- | --- |
| 选中并置顶 / 移动 | 左键单击 / 拖动 |
| 关闭并保留历史 | 左键双击或单击中键 |
| 缩放 / 调不透明度 | 滚轮 / `Ctrl`+滚轮 |
| 复制、标注、保存或恢复原尺寸 | 打开右键菜单 |
| 收起或展开迷你态 | 右键菜单或`Shift+R` |
| 开启点击穿透 | 右键菜单或`T` |

贴图收到键盘事件时，`Ctrl+C`复制、`E`打开Satty、`0`恢复原尺寸、`Esc`关闭。若按键仍发给后方窗口，请用右键菜单。开启穿透后，按`Super+Shift+U`或运行`pinshot unghost-all`可恢复鼠标操作。默认情况下，应用全屏时会暂时隐藏该显示器上的贴图。

### 滚动长截图

1. 普通截图框选后，点工具条最左侧的长截图图标，选择纵向或横向。也可单独运行`pinshot longshot`或`pinshot longshot --horizontal`。
2. 在GNOME共享对话框中选择显示器。从普通截图进入会沿用原选区；从独立入口进入则在授权屏幕上框选。整个选区必须位于该屏幕内，且框外要留出工具条的位置。
3. 沿所选轴向任一端滚动。若提示回滚，先回到上次拼接成功的位置，再小幅滚动。截取内部滚动的代码框时，尽量只框正文，避开无关侧栏或动态浮层。
4. 点标注、贴图、保存或复制结束并输出；点×放弃。边框和工具条不会进入成品。沿滚动轴达到24,000像素或触及内存上限时会自动结束采集。

可让GNOME记住共享屏幕，需要换屏时用`pinshot longshot --choose-screen`。GIF和长截图共用这个授权，但不能同时运行。采集结束后Pinshot会关闭共享会话。从GNOME停止共享会取消本次采集。

### 录制GIF

1. 点普通截图工具条上的摄像机，或执行`pinshot gif`再确认一个选区。蓝框和`00:00`表示待录，此时不会启动屏幕共享。
2. 选择**5、10、16、24或30 FPS**（默认10），再点红色圆形按钮请求屏幕共享。需要重选屏幕时，可用`pinshot gif --choose-screen`。
3. 红框和计时表示录制中。点方形停止按钮保存，点×放弃。系统装有`notify-send`时，保存成功会通知完整文件路径；不会复制到剪贴板，也不会写入贴图历史。

GIF循环播放，会录入鼠标指针，不录声音。帧率是采样上限，实际帧数取决于画面更新。大选区会等比缩小，长边不超过1,280像素，总像素数不超过1,280 × 720。没有固定时长上限，文件达到64 MiB时会保存已录部分。

## 保存与设置

打开扩展首选项的「截图」页：

| 设置 | 行为 |
| --- | --- |
| 保存目录 | PNG和GIF共用。填绝对路径或以`~/`开头的路径，再点输入框旁的应用按钮。留空时使用Core配置的`general.save_dir`，默认为`~/Pictures/Pinshots`。 |
| 文件名前缀 | 默认`pinshot`，加上本地保存时间，例如`pinshot-20261121-214530.png`。重名时加`-1`等序号，不覆盖已有文件。 |
| 自动保存到目录 | 默认关闭。开启后，贴图或在Satty里确认标注时会额外导出结果。复制仍只改剪贴板；主动保存PNG或停止GIF不受此开关影响。 |
| 显示截图完成通知 | 默认关闭。不影响GIF保存的路径通知、主动保存提示或GNOME自带的`Print`。 |

### 缓存与历史

`pinshot status`可查看扩展状态、缓存和配置路径、贴图数量。缓存默认放在`~/.cache/pinshot/`，若设置了`XDG_CACHE_HOME`则使用`$XDG_CACHE_HOME/pinshot/`。默认按200 MB、14天清理，优先清理非活跃贴图。除非启用`restore_on_startup`，否则下次登录时不会自动弹出贴图。可用`pinshot restore-last`找回最近一张仍有缓存的贴图。

```bash
pinshot clear-history       # 清理非活跃历史，保留活跃贴图
pinshot clear-history --all # 同时关闭活跃贴图，并删除其缓存
```

清理不可撤销，但不会删除已导出的PNG或GIF。

<details>
<summary>Core配置文件</summary>

运行`pinshot status`会在需要时生成配置文件。默认路径是`~/.config/pinshot/config.toml`，也可能是`$XDG_CONFIG_HOME/pinshot/config.toml`。常用配置如下：

```toml
[general]
save_dir = "~/Pictures/Pinshots"
restore_on_startup = false
notify_on_save = true             # 主动保存 PNG 成功后的通知

[cache]
max_size_mb = 200
max_age_days = 14

[text_card]
theme = "auto"                    # auto、light 或 dark
max_width_px = 680
max_lines = 80
```

文字卡片设置对新建卡片生效。`[pin]`下的配置（如`default_opacity`、`mini_size_px`和`hide_on_fullscreen`）在扩展启用时读取，修改后注销并重新登录，无需重编。

</details>

## 排查与卸载

- **扩展离线**：确认当前是GNOME Shell 51的Wayland会话，用`gnome-extensions info pinshot@dgxz99`查看状态。若GNOME仍在用旧版扩展代码，注销并重新登录。
- **快捷键没反应**：先试对应的CLI命令，再检查快捷键是否禁用或与系统按键冲突。
- **查看日志**：运行`journalctl --user -b -o cat | grep -i pinshot`。[反馈问题](https://github.com/dgxz99/pinshot/issues)时请附上GNOME Shell版本、是否为Wayland会话和复现步骤。
- **卸载**：运行`./scripts/uninstall.sh`，默认保留Core配置和缓存；加`--purge`会一并删除。已经导出的文件不受影响。

## 开源协议

Pinshot采用[MIT许可证](LICENSE)。
