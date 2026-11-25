# Pinshot 架构

Pinshot 由 GNOME Shell 扩展和按需运行的 Rust CLI 组成。贴图不是普通窗口，由 Shell 绘制；Rust 负责图像和文件，不需要额外的后台守护进程。

## 组件

| 模块 | 职责 |
| --- | --- |
| [GNOME Shell 扩展](../extensions/pinshot@dgxz99/) | 全局快捷键、首选项、冻结画面框选、采集控制条和贴图。`pinManager.js` 将贴图容器挂在 Shell 的 `uiGroup`；`pinActor.js` 管理单张贴图的纹理、交互和销毁 |
| [Rust CLI](../crates/pinshot-cli/) | 命令入口和输出动作分派，协调扩展与 Core。贴图菜单上的复制、标注、保存等操作通过异步子进程调用 CLI |
| [Rust Core](../crates/pinshot-core/) | 剪贴板读写、Pango 文字卡片、Satty 标注、缓存与导出，以及 PipeWire 采帧、滚动拼接和 GIF 编码 |

CLI 和扩展通过会话 D-Bus 通信。服务名为 `io.github.dgxz99.Pinshot.Frontend`，对象路径为 `/io/github/dgxz99/Pinshot/Frontend`，接口为 `io.github.dgxz99.Pinshot.Frontend1`。接口定义见 [`dbusService.js`](../extensions/pinshot@dgxz99/dbusService.js)，Rust 客户端见 [`dbus_client.rs`](../crates/pinshot-core/src/dbus_client.rs)。截图和剪贴板 PNG 通过文件描述符传输，不把图像数据直接放进 D-Bus 消息体。

## 两种采集方式

- **普通截图**由 [`screenshot.js`](../extensions/pinshot@dgxz99/screenshot.js) 直接捕获 Shell 舞台的冻结画面。框选后再裁剪为 PNG，保留原始缩放倍率。不使用屏幕共享，也不接管 GNOME 原生截图。
- **长截图和 GIF**由 Core 的 [`longshot`](../crates/pinshot-core/src/longshot/) 模块通过 GNOME ScreenCast Portal 获取单显示器的 PipeWire 流，用 GStreamer 采集。选区保持为桌面逻辑坐标，按实际帧尺寸换算像素边界；跨出授权显示器的选区被拒绝。两者共用授权和独占租约，工具条位于采集区之外。
- **长截图**隐藏光标，按固定轴将新帧与已拼接内容匹配，只补入有可靠重叠的新增区域。**GIF**录入光标，按单调时钟采样并逐帧编码至匿名临时文件。两种采集在结束或取消时都关闭共享会话；记住的是授权令牌，而不是后台连接。

## 数据边界

- **设置**：快捷键和导出选项由扩展的 GSettings 管理；缓存、贴图和文字卡片参数由 Core 从 `config.toml` 读取。
- **缓存与导出**：贴图原图、缩略图、文字原文和历史索引在用户缓存目录内；已导出的 PNG、GIF 不受历史清理影响。Portal 的恢复令牌保存在缓存目录中，文件权限为 `0600`。
- **显示倍率**：Pinshot 复制 PNG 时用私有辅助块 `psCL` 保存原始倍率，不重采样。重新贴出时，扩展使用原始像素尺寸和倍率计算逻辑显示大小，再按工作区限制大图的初始尺寸。
