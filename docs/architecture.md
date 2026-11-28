# Pinshot 架构

Pinshot 由 GNOME Shell 扩展和按需运行的 Rust CLI 组成。贴图不是普通窗口，由 Shell 绘制；Rust 负责图像、剪贴板和文件，不需要额外的后台守护进程。

## 组件

| 模块 | 职责 |
| --- | --- |
| [GNOME Shell 扩展](../extensions/pinshot@dgxz99/) | 全局快捷键、首选项、原生顶栏图标菜单（`panelIndicator.js`）、冻结画面框选与全屏抓取（`screenshot.js`）、采集控制条（`longshot.js`）和贴图。`pinManager.js` 将贴图容器挂在 Shell 的 `uiGroup` 并调度串行后台 OCR 队列；`pinActor.js` 与 `ocrSelection.js` 管理单张贴图的纹理、原位定位、离线划词选区和销毁 |
| [Rust CLI](../crates/pinshot-cli/) | 命令入口和输出动作分派，协调扩展与 Core。顶栏菜单与贴图右键菜单中的采集、复制、标注、保存、离线 OCR 等操作通过异步子进程调用 CLI |
| [Rust Core](../crates/pinshot-core/) | 剪贴板读写（含 `text/html` 富文本样式解析）、Pango 动态宽度文字/代码卡片、可选 Satty 标注、纯 Rust 离线 PP-OCRv5 推理（`ocr/`）、通知图标解析、缓存与导出，以及 PipeWire 采帧、滚动拼接和 GIF 编码 |
| [桌面入口与打包](../packaging/) | 轻量 `.desktop` 应用启动器（[data/io.github.dgxz99.pinshot.desktop](../data/io.github.dgxz99.pinshot.desktop)）、发布包组装脚本（[scripts/package-release.sh](../scripts/package-release.sh)）与 AUR `PKGBUILD` 配置 |

CLI 和扩展通过会话 D-Bus 通信。服务名为 `io.github.dgxz99.Pinshot.Frontend`，对象路径为 `/io/github/dgxz99/Pinshot/Frontend`，接口为 `io.github.dgxz99.Pinshot.Frontend1`。接口定义见 [`dbusService.js`](../extensions/pinshot@dgxz99/dbusService.js)，Rust 客户端见 [`dbus_client.rs`](../crates/pinshot-core/src/dbus_client.rs)。截图和剪贴板 PNG 通过文件描述符传输，不把图像数据直接放进 D-Bus 消息体。

由于 Rust Core 与 CLI 只面向 `Frontend1` 协议通信，未来支持 niri 等非 GNOME Wayland 合成器时，可以独立实现基于 GTK4 / Qt6 Layer Shell 的前端进程，复用全部核心逻辑。

## 两种采集方式

- **区域与全屏截图**由 [`screenshot.js`](../extensions/pinshot@dgxz99/screenshot.js) 直接捕获 Shell 舞台的冻结画面。区域框选裁剪为 PNG 并把原始选区坐标传回 Core，贴图时按 1:1 尺寸原位显示；全屏截图直接截取当前显示器并播放短暂闪屏反馈。不使用屏幕共享，也不接管 GNOME 原生截图。
- **长截图和 GIF**由 Core 的 [`longshot`](../crates/pinshot-core/src/longshot/) 模块通过 GNOME ScreenCast Portal 获取单显示器的 PipeWire 流，用 GStreamer 采集。选区保持为桌面逻辑坐标，按实际帧尺寸换算像素边界；跨出授权显示器的选区被拒绝。两者共用授权和独占租约，工具条位于采集区之外。
- **长截图**隐藏光标，按固定轴将新帧与已拼接内容匹配，只补入有可靠重叠的新增区域。**GIF**录入光标，按单调时钟采样并逐帧编码至匿名临时文件。两种采集在结束或取消时都关闭共享会话；记住的是授权令牌，而不是后台连接。

## 数据边界与安装路径

- **设置**：快捷键、导出选项、顶栏图标外观与贴图划词开关由扩展的 GSettings 管理；缓存、贴图、文字卡片与 OCR 参数由 Core 从 `~/.config/pinshot/config.toml` 读取。
- **缓存与导出**：贴图原图、缩略图、文字原文、OCR 结果（`ocr/<id>.json`）和历史索引在 `~/.cache/pinshot/`（或 `$XDG_CACHE_HOME/pinshot/`）内；已导出的 PNG、GIF 不受历史清理影响。Portal 的恢复令牌保存在缓存目录中，文件权限为 `0600`。
- **离线 OCR 模型**：按「自定义 `ocr.model_dir` → 用户目录 `~/.local/share/pinshot/ocr/<model_id>/` → 系统目录 `/usr/share/pinshot/ocr/<model_id>/`」顺序查找，与历史缓存隔离；`pinshot clear-history` 不影响模型文件。
- **显示倍率**：Pinshot 复制 PNG 时用私有辅助块 `psCL` 保存原始倍率，不重采样。重新贴出时，扩展使用原始像素尺寸和倍率计算逻辑显示大小，再按工作区限制大图的初始尺寸。
- **用户级与系统级安装**：脚本安装（含 `curl | bash`）写入 `~/.local/bin` 与 `~/.local/share`，AUR 等包管理器安装写入 `/usr/bin` 与 `/usr/share`。`coreRunner.js`、`notify.rs` 与 `ocr/models.rs` 均按「用户目录优先、系统目录回退」的顺序查找二进制、通知图标与 OCR 模型。
