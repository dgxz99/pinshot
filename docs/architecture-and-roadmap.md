# `pinshot` 架构设计与开发规划文档

> 面向 Wayland 桌面的轻量级截图、真·非窗口贴图、剪贴板文字卡片与滚动长图工具（一期聚焦 GNOME 51，架构原生解耦以平滑扩展至 Niri + DMS）。

---

## 1. 项目概述与核心指标

### 1.1 目标与定位
- **项目与 CLI 名称**：`pinshot`
- **GNOME Shell 扩展 UUID**：`pinshot@dgxz99`
- **核心设计原则**：
  - **零常驻开销**：Core 采用纯 Rust 单二进制、短命按需执行模型，闲置常驻内存 **0 MB**，常规触发瞬时峰值 **5–15 MB**，启动耗时 **< 15 ms**。
  - **真·非窗口贴图**：贴图不属于常规应用窗口，彻底不出现在任务栏（如 `aztaskbar`）、Alt-Tab（如 `CoverflowAltTab`）、Overview、窗口圆角扩展（`rounded-window-corners`）及工作区切换特效（`desktop-cube`）中，始终置顶且鼠标操作不抢占当前工作窗口焦点。
  - **跨桌面解耦**：90% 业务逻辑沉淀在跨桌面的 Rust Core 中；桌面合成器差异收敛在极薄的可插拔前端中，通过统一 D-Bus 协议通信。

### 1.2 运行环境与外部依赖（一期 GNOME 51 / Arch Linux）
| 组件 | 用途 | 备注 |
|---|---|---|
| `rust` / `cargo` | 构建 Core 二进制 | 编译期依赖 |
| `glib-compile-schemas` | 为 GNOME 扩展编译首选项 schema | 安装时由脚本调用 |
| `xdg-desktop-portal` + `xdg-desktop-portal-gnome` | 长图帧流（`ScreenCast`） | 截图由 GNOME 扩展直接复用原生界面 |
| `wl-clipboard`（`wl-paste` / `wl-copy`） | 剪贴板读取与写入 | GNOME / Niri 通用 |
| `pango`（`pango-view`） | 剪贴板纯文本/代码卡片高质量渲染 | 短命子进程，中文排版稳定 |
| `satty` | 图片标注与回流编辑 | 复用现有原生标注工具 |
| `gstreamer` + `pipewire`（`pipewiresrc`） | 长图录制时拉取 ScreenCast 视频流帧 | 仅长图录制期间按需启动 |
| GNOME Shell 51（GJS） | 一期非窗口贴图浮层与快捷键前端 | 极薄 UI 适配层 |

---

## 2. 总体分层架构

### 2.1 分层总览

```text
+-----------------------------------------------------------------------+
|                       用户触发入口（全局快捷键 / CLI）                    |
|  capture-pin | capture-edit | pin-clipboard | longshot | toggle-hide  |
|               unghost-all | restore-last | clear-history              |
+-----------------------------------+-----------------------------------+
                                    | 启动短命进程（干完即退，闲置 0 MB）
                                    v
+-----------------------------------------------------------------------+
|                     pinshot Core（纯 Rust 单二进制）                    |
|                                                                       |
|  +-------------+  +--------------+  +---------------+  +-----------+  |
|  |   capture   |  |  clipboard   |  |  text_render  |  |  annotate |  |
|  | 前端抓屏     |  | 多MIME读写    |  | Pango代码/卡片 |  | satty回流  |  |
|  +-------------+  +--------------+  +---------------+  +-----------+  |
|  +-------------+  +--------------+  +---------------+  +-----------+  |
|  |  longshot   |  |    store     |  |    config     |  | ocr/hooks |  |
|  | 帧流实时拼接 |  | 缓存双阈值淘汰 |  |  config.toml  |  | 二期扩展层 |  |
|  +-------------+  +--------------+  +---------------+  +-----------+  |
+-------------------+-------------------------------+-------------------+
                    | D-Bus 调用前端展示接口          ^ 前端按需拉起
                    | (Session Bus)                 | `pinshot internal ...`
                    v                               | （短命回调，不常驻）
+-----------------------------------------------------------------------+
|              统一前端协议：io.github.dgxz99.Pinshot.Frontend            |
+-----------------------------------+-----------------------------------+
                                    |
             +----------------------+----------------------+
             |                                             |
             v                                             v
+---------------------------------------+   +---------------------------+
| 一期：GNOME 51 极薄 GJS 扩展           |   | 未来：Niri + DMS 前端      |
| (`pinshot@dgxz99`)                    |   | (`crates/pinshot-niri`)   |
| - uiGroup 顶层真·非窗口贴图渲染         |   | - gtk4-rs +               |
| - 拖拽 / 滚轮缩放 / 透明度 / 迷你态     |   |   gtk4-layer-shell        |
| - 点击穿透 / 单击选中键盘控制           |   |   (Overlay 层非窗口)       |
| - 长图选区框选与悬浮控制条              |   | - DMS 快捷键             |
| - 自定义全局快捷键                    |   | - 无活跃贴图自动退出       |
+---------------------------------------+   +---------------------------+
```

### 2.2 零常驻执行模型
本系统**不引入常驻后台守护进程**：
1. **正向调用（Core → 前端）**：用户通过快捷键执行 `pinshot <subcommand>`，Rust Core 短暂启动完成抓屏/渲染/落盘/索引更新后，通过 Session D-Bus 通知前端创建或更新贴图，随即**立即退出释放全部内存**。
2. **反向回调（前端 → Core）**：当用户在贴图右键菜单点击「复制图片」「复制原文」「进入 `satty` 标注」「保存」等需要重 I/O 或系统交互的动作时，前端通过异步子进程调用 `pinshot internal <action> <id>`，同样干完即退。
3. **长图录制期**：仅在用户处于滚动长图录制状态期间，`pinshot longshot` 保持运行以消费 PipeWire 视频帧并实时拼接；用户点击「完成」或「取消」后，结果落盘并推送贴图，进程立即退出归还全部帧缓冲内存。

---

## 3. 统一前后端 IPC / D-Bus 协议

前端在 Session Bus 上注册固定服务名与对象路径，确保 Core 完全不感知底层是 GNOME Shell 还是未来的 Niri `layer-shell`：

- **Bus Name**：`io.github.dgxz99.Pinshot.Frontend`
- **Object Path**：`/io/github/dgxz99/Pinshot/Frontend`
- **Interface**：`io.github.dgxz99.Pinshot.Frontend1`

### 3.1 前端暴露给 Core 的方法

| 方法 | 参数 | 返回值 | 说明 |
|---|---|---|---|
| `Ping()` | 无 | `s`（前端标识，如 `"gnome-shell-51"`） | 探测前端在线与版本 |
| `CreatePin(payload_json)` | `s`（JSON：`id`, `image_path`, `thumb_path`, `width`, `height`, `source_scale`, `kind`, `has_text`, `title_preview`） | `b` | 创建并弹出贴图；`source_scale` 用于分数缩放 1:1 逻辑尺寸还原 |
| `UpdatePin(id, payload_json)` | `s`, `s` | `b` | 标注回流等场景原地替换贴图内容并恢复显示 |
| `SetPinTemporaryHidden(id, hidden)` | `s`, `b` | `b` | 进入 `satty` 标注时临时隐藏原贴图，取消时无损恢复 |
| `ClosePin(id)` | `s` | `b` | 关闭指定贴图并释放纹理 |
| `ToggleHideAll()` | 无 | `b`（当前是否处于隐藏态） | 一键隐藏 / 恢复所有贴图 |
| `SetSystemHidden(hidden)` | `b` | `b` | 长图录制等系统流程临时避让隐藏旧贴图，结束后还原原显隐态 |
| `UnghostAll()` | 无 | `u`（解除数量） | 一键解除全部贴图的点击穿透 |
| `ListActivePins()` | 无 | `s`（JSON 数组） | 查询当前活跃贴图 ID、位置、缩放与状态 |
| `CaptureScreenshot(output_fd)` | `h`（Unix 文件描述符） | `d`（纹理缩放倍率；`0` 表示取消） | 显示原生截图界面，把用户确认后的 PNG 写入调用方的临时文件 |
| `GetScreenshotSettings()` | 无 | `b`, `s`, `s`, `b` | 读取 Pinshot 自动导出、目录、文件名前缀、完成通知设置 |

### 3.2 Core 提供给前端的内部子命令（`pinshot internal ...`）
前端通过短命子命令将图像处理、保存和历史写入交给 Core：
- `pinshot internal copy-image <id>`
- `pinshot internal copy-text <id>`
- `pinshot internal save-image <id>`
- `pinshot internal edit-satty <id>`
- `pinshot internal mark-closed <id>`（更新历史索引为已关闭，供 `restore-last` 捞回）
- `pinshot internal get-pin-config`
- `pinshot internal restore-startup`

---

## 4. 仓库结构与 Rust Core 详细设计

### 4.1 仓库目录规划

```text
pinshot/
├── Cargo.toml                         # Rust workspace 配置
├── docs/
│   └── architecture-and-roadmap.md    # 本文档
├── crates/
│   ├── pinshot-core/                  # 跨桌面核心业务库
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── config.rs              # config.toml 加载与默认值
│   │       ├── store.rs               # 缓存索引、双阈值滚动清理、物理删除
│   │       ├── dbus_client.rs         # 调用前端 D-Bus 接口
│   │       ├── capture.rs             # 前端截图接收与 PNG 导出
│   │       ├── clipboard.rs           # wl-clipboard 多 MIME 读写与解析
│   │       ├── text_render.rs         # 代码检测、Pango 转义、主题感知卡片渲染
│   │       ├── annotate.rs            # satty 标注回流状态机
│   │       ├── longshot/
│   │       │   ├── mod.rs             # 长图会话编排与容错控制
│   │       │   ├── screencast.rs      # Portal ScreenCast + restore_token
│   │       │   ├── stream.rs          # GStreamer pipewiresrc 帧采集与选区裁剪
│   │       │   └── stitcher.rs        # 纯 Rust 实时帧间重叠匹配与拼接
│   │       ├── notify.rs              # 轻量桌面通知（空剪贴板/错误/保存提示）
│   │       ├── hooks.rs               # 自定义外部命令钩子（二期）
│   │       └── ocr.rs                 # AI 视觉 OCR 接口抽象（二期）
│   ├── pinshot-cli/                   # 主 CLI 入口二进制 (`pinshot`)
│   │   ├── Cargo.toml
│   │   └── src/main.rs
│   └── pinshot-niri/                  # （未来扩展预留）Niri gtk4-layer-shell 前端
├── extensions/
│   └── pinshot@dgxz99/                # 一期 GNOME Shell 51 极薄浮层前端
│       ├── metadata.json
│       ├── stylesheet.css
│       ├── extension.js               # 生命周期、D-Bus 注册、全屏与显示器监听
│       ├── dbusService.js             # Frontend1 D-Bus 接口实现
│       ├── pinManager.js              # 多贴图 Z-Order、全局显隐/穿透管理、热插拔回收
│       ├── pinActor.js                # 单贴图非窗口渲染、缩放、迷你态、防泄漏销毁
│       ├── contextMenu.js             # 贴图右键菜单
│       ├── screenshot.js              # 复用 GNOME 原生截图界面，分离截图输出
│       ├── regionSelector.js          # 长图纯滚动区域透明框选遮罩
│       └── longshotHud.js             # 长图悬浮控制条（高度展示、快滚告警、完成/取消）
└── scripts/
    ├── install.sh                     # 构建并安装到 ~/.local/bin 与扩展目录
    └── uninstall.sh                   # 彻底卸载并可选清理缓存与配置
```

### 4.2 配置文件设计（`~/.config/pinshot/config.toml`）

```toml
[general]
save_dir = "~/Pictures/Pinshots"
restore_on_startup = false
notify_on_save = true

[cache]
max_size_mb = 200          # 缓存目录最大磁盘占用（MB）
max_age_days = 14          # 缓存最长保留天数

[pin]
default_opacity = 1.0
min_scale = 0.2
max_scale = 5.0
zoom_step = 0.1
opacity_step = 0.08
mini_size_px = 64          # 迷你缩略图边长（逻辑像素）
min_visible_edge_px = 32   # 拖拽防丢至少留在屏内的像素
max_texture_edge_px = 4096 # 前端渲染最大纹理边长上限
hide_on_fullscreen = true

[text_card]
theme = "auto"             # "auto" | "dark" | "light"
max_width_px = 680
max_lines = 80             # 超长文本截断保护
prose_font = "Sans 12"
code_font = "Monospace 11"
padding_px = 16

```

### 4.3 持久化存储、滚动清理与隐私保护（`store.rs`）
- **目录与权限**：`~/.cache/pinshot/` 创建时严格设置 Unix 权限 `0700`（仅当前用户可读写）。
  - `images/<id>.png`：100% 原始无损图片（供复制、保存和 `satty` 标注使用）。
  - `thumbs/<id>.png`：前端迷你态用的轻量缩略图（如 128px 以内，几 KB）。
  - `texts/<id>.txt`：剪贴板文字转图片时同步保存的原始纯文本。
  - `index.json`：所有贴图与历史条目的元数据（原子写入：先写 `.tmp` 再 `rename`）。
  - `screencast_token`：Portal ScreenCast 复用令牌（权限 `0600`）。
- **单条索引字段**：
  `id`, `kind` (`"screenshot" | "clipboard_image" | "clipboard_text" | "longshot"`), `created_at`, `last_used_at`, `active`（上次是否处于显示态）, `image_path`, `thumb_path`, `text_path?`, `title_preview`, `width`, `height`, `source_scale`, `size_bytes`。
- **双阈值自动淘汰算法**（每次新增条目后顺带执行，无需后台定时任务）：
  1. 保护当前处于 `active` 状态的贴图不被淘汰；
  2. 先清理 `now - last_used_at > max_age_days` 的非活跃历史条目；
  3. 再统计剩余目录总字节数，若超过 `max_size_mb`，按 `last_used_at` 从旧到新依次删除对应图片、缩略图、文本及索引项，直至低于阈值；
  4. 用户主动保存到 `~/Pictures/Pinshots/` 的文件独立存在，永不参与清理。
- **隐私清理**：
  - 支持 `pinshot clear-history` 一键清空所有非活跃历史（加 `--all` 时连同活跃贴图一起关闭并清空）。
- **重启后恢复行为**：
  - 默认 `restore_on_startup = false`：重新登录后不自动弹满屏，上次关机前未关闭的贴图自动排在历史记录最顶部，运行 `pinshot restore-last` 或绑定对应快捷键即可恢复；
  - 若开启 `restore_on_startup = true`：扩展启动完成时调用 Core 恢复上次 `active = true` 的条目。

### 4.4 剪贴板处理与文字转图片（`clipboard.rs` & `text_render.rs`）
- **多 MIME 探测优先级**（调用 `wl-paste --list-types`）：
  1. `image/png` → 直接写入缓存 PNG；
  2. `image/jpeg` / `image/webp` / `image/bmp` → 读取后统一转码存为标准 PNG；
  3. `text/uri-list` → 解析 `file://` URI，若是本地存在的常见图片文件（`.png`/`.jpg`/`.jpeg`/`.webp`/`.bmp`），直接复制/转码进缓存贴出；
  4. `text/plain;charset=utf-8` / `text/plain` → 走文字卡片渲染流程；
  5. 空剪贴板或不支持类型 → 发送轻量桌面通知提示后退出。
- **文字转图片细节**：
  - **原文留存**：将原始文本完整存入 `texts/<id>.txt`，前端根据 `has_text = true` 在右键菜单显示「复制原始文本」。
  - **代码特征自动识别**：基于缩进比例（多行以 2/4 空格或 Tab 开头）、代码符号密度（`{}`、`();`、`=>`、`::`、`import`/`fn`/`def`/`const`）判断是否切换 `code_font`（等宽字体、保留空格缩进）；普通文本使用 `prose_font`。
  - **安全转义与折行**：严格转义 `&`、`<`、`>` 等字符避免 Pango 解析报错；启用 `--wrap=word-char` 强制对超长单行（URL、压缩 JSON、Token）折行，避免生成超宽画布。
  - **超长文本保护**：超过 `max_lines` 时截断渲染内容并在卡片底部追加省略提示（如 `… 已截断，共 320 行（右键可复制完整原文）`），原文文件仍完整保留。
  - **主题自适应**：`theme = "auto"` 时读取 Portal `org.freedesktop.appearance.color-scheme`（`1` = 深色，`2` = 浅色）自动选用高对比度卡片配色；同时为 `pango-view` 按当前显示器缩放倍率设置 DPI，确保高分屏下文字锐利不模糊。

### 4.5 `satty` 标注回流状态机（`annotate.rs`）
```text
贴图右键/按 E 触发 `pinshot internal edit-satty <id>`
  │
  ├─ 1. 调前端 `SetPinTemporaryHidden(id, true)`（原贴图避让，不遮挡画布）
  ├─ 2. 准备临时输出路径 `<tmp_out.png>`
  ├─ 3. 启动 `satty -f <src.png> -o <tmp_out.png> --early-exit all --actions-on-enter save-to-file exit`
  └─ 4. 等待 satty 退出：
        ├─ `<tmp_out.png>` 存在且非空（用户确认保存）：
        │     ├─ 原旧图自动降级保留在历史记录中（防后悔）
        │     ├─ 生成新缓存图与缩略图，更新该贴图绑定图片
        │     └─ 调前端 `UpdatePin(id, new_payload)` 原地更新并显示
        └─ `<tmp_out.png>` 不存在（用户按 Esc 取消）：
              └─ 调前端 `SetPinTemporaryHidden(id, false)` 无损恢复原贴图
```
对于 `pinshot capture-edit`：直接抓屏后调起 `satty`，标注完成后将最终图复制到剪贴板。若开启自动保存，只导出标注后的图。

### 4.6 滚动长图引擎规划（`longshot/`）
滚动长图完整流程分为 4 个清晰阶段：
1. **启动与避让**：
   - 调用前端 `SetSystemHidden(true)` 临时隐藏屏幕上已有贴图，避免遮挡滚动内容；
   - 通过 `ashpd` 请求 `org.freedesktop.portal.ScreenCast` 会话（默认尝试携带 `persist_mode = 2` 与上次缓存的 `restore_token` 以减少重复授权弹窗，失败则平滑回退标准系统选择框）。
2. **锁定纯滚动内容区**：
   - 调用前端 `SelectRegion()`，在屏幕上弹出全屏半透明框选遮罩，用户框出需要拼接的**纯滚动内容矩形**（天然排除浏览器吸顶栏、左侧菜单和聊天输入框）；
   - 启动 GStreamer `pipewiresrc fd=<fd> path=<node_id> ! videoconvert ! video/x-raw,format=RGBA ! appsink` 按目标区域裁剪取帧（限频 ~15–20 fps，大幅降低 CPU 占用）。
3. **纯 Rust 实时重叠拼接算法（`stitcher.rs`）**：
   - 保留当前拼接底部窗口的**灰度水平投影/下采样特征条带**（左右各裁掉少量边缘像素如 4%–6%，避免右侧滚动条滑块干扰匹配）；
   - **静止帧去重**：与上一帧全区平均绝对差（MAD）低于极小阈值时直接丢弃；
   - **纵向位移搜索**：在合理的向下滚动位移范围内（`dy ∈ [8px, 0.85 * frame_height]`）做粗搜（降采样 4x SAD）+ 精搜（原始分辨率验证归一化匹配度 NCC/MAD）；
   - **回滚忽略与快滚告警**：
     - 检测到向上回滚（内容已存在于历史尾部区域）→ 忽略该帧不重复拼接；
     - 最佳匹配置信度低于阈值（滚动过快导致与上一帧无重叠）→ 不强行拼接，通知前端悬浮控制条高亮提示**「滚动过快，请微调回滚」**；
     - 匹配成功 → 将新露出底部增量行追加到拼接画布，并节流推送 `UpdateLongshotHud` 刷新已拼接像素高度（达到 `max_height_px` 时自动安全封顶提示）。
4. **收尾闭环**：
   - 监听前端 `LongshotAction("finish" | "cancel")` 信号；
   - 若完成：停止流管线 → 保存 100% 原图到缓存 → 根据配置写入剪贴板 → 恢复先前隐藏的旧贴图（`SetSystemHidden(false)`）→ 将新长图按适配屏幕高度的比例推给前端展示（可右键保存或进 `satty` 标注）；
   - 若取消：停止流管线 → 清理临时缓冲 → 恢复旧贴图 → 退出释放全部内存。

---

## 5. GNOME 51 极薄 GJS 前端详细设计（`pinshot@dgxz99`）

### 5.1 「真·非窗口」承载与全屏避让
- **渲染层级**：每个贴图是一个独立 `St.Widget`（内嵌图像绘制与极细选中边框），统一挂载在一个顶层容器并加入 `Main.layoutManager.uiGroup`（位于普通窗口层之上、锁屏与屏保之下，不创建任何 `Meta.Window`）。
- **天然隔离**：因为不是窗口，天然不进入 `aztaskbar`、`CoverflowAltTab`、Overview，不受 `rounded-window-corners` 修饰，且在 `desktop-cube` 切工作区时稳稳悬浮在视口上层。
- **全屏自动避让**：监听 `Main.layoutManager` 的 `monitors-changed` 与各显示器的全屏状态变化（`monitor.inFullscreen`）。当某个显示器进入全屏应用时，自动隐藏位于该显示器上的贴图，退出全屏立即恢复（若用户手动按 `toggle-hide` 可覆盖）。

### 5.2 分数缩放 1:1 还原与防丢机制
- **1:1 视觉尺寸换算**：收到 Core 传入的物理像素 `(width, height)` 与源缩放 `source_scale` 后，初始逻辑尺寸设为：
  `logical_w = round(width / source_scale)`，`logical_h = round(height / source_scale)`。
  若单边超过当前显示器工作区的 75%（如超长图或全屏截图），等比缩放到工作区 75% 以内作为初始展示尺寸（滚轮随时可放大回 100%）。
- **跨缩放屏一致**：贴图坐标与尺寸在 Clutter 逻辑坐标系中维护，拖到不同缩放比例的显示器时视觉物理大小保持稳定。
- **初始弹出位置**：查询 `global.get_pointer()` 获取当前光标坐标与所在显示器，将贴图居中或偏移弹出在光标附近，并钳制在显示器工作区内。
- **防丢保护**：
  - 多图重叠时，鼠标按下或拖动某贴图即执行 `set_child_above_sibling(actor, null)` 置前；
  - 拖拽移动时钳制坐标，保证贴图四边至少有 `32px` 留在合并显示器边界内；
  - 监听 `monitors-changed`：拔掉外接屏后遍历所有贴图，发现落在有效显示器几何外的贴图自动拉回主屏可见区。

### 5.3 交互细节与防误触机制
| 操作 | 触发方式 | 焦点影响 |
|---|---|---|
| 移动贴图 | 左键按下拖拽（普通态与迷你态通用） | 免焦点，不抢编辑器焦点 |
| 无级放大/缩小 | 滚轮上下滚动（以鼠标指针为中心缩放） | 免焦点 |
| 调节不透明度 | `Ctrl + 滚轮` | 免焦点 |
| 迷你缩略图切换 | 右键菜单收起或展开（收缩至 `64×64px` 缩略图并记忆原位原尺寸） | 仅菜单展开期间捕获点击 |
| 快速关闭 | 双击左键或点击鼠标中键 | 免焦点 |
| 右键菜单 | 右键点击（复制图片 / 复制原文 / `satty` 标注 / 迷你态 / 点击穿透 / 恢复 100% / 保存 / 关闭） | 仅菜单展开期间捕获点击 |
| 键盘快捷键 | **单击贴图选中**（显示 1px 主题色高亮描边）后支持：`Ctrl+C` 复制、`E` 标注、`Shift+R` 收起／展开迷你态、`T` 开启穿透、`0` 复原 100%、`Esc` 关闭 | 点贴图外任意位置立即取消选中并归还焦点 |

**点击穿透（Ghost Mode）**：
- 开启穿透后设置 `actor.reactive = false` 并自动将不透明度压低（如若当前 `> 0.75` 则温和降至 `0.65`，提示处于穿透态）。
- 全局快捷键（默认 `Super+Shift+U`）和终端命令 `pinshot unghost-all` 可解除全部穿透，不依赖贴图焦点。未穿透的贴图也可从右键菜单解除全部穿透。

**扩展快捷键设置**：可在 GNOME「扩展」应用打开 Pinshot 首选项，或运行 `gnome-extensions prefs pinshot@dgxz99`。默认 `Super+Shift+P` 区域截图并贴图、`Super+Shift+V` 剪贴板贴图、`Super+Shift+U` 解除全部穿透。区域截图后标注、显示／隐藏全部贴图、恢复最近贴图默认未绑定。各项可改绑、禁用或恢复默认，不修改 GNOME 系统里的自定义绑定。贴图局部按键只有在 Shell 将键盘事件交给贴图时才生效，也可用右键菜单完成相应操作。

**截图输出设置**：Pinshot 发起的截图仍使用 GNOME 原生框选、屏幕或窗口截图界面。仅在本次 Pinshot 截图期间，将 PNG 交给 Core 处理，不触发 GNOME 的自动保存和完成通知，也不修改任何 GNOME 设置。原生 `Print` 工作流照常保存和通知。

扩展首选项的「截图」页可单独启用自动保存和完成通知，默认都关闭。截图贴图仍写入 Pinshot 历史缓存；勾选自动保存后，额外导出一份 PNG。目录和文件名也用于手动「保存原图」：目录留空沿用 `general.save_dir`（默认 `~/Pictures/Pinshots`），文件名默认为 `pinshot-YYYYMMDD-HHMMSS.png`，以实际保存时的本地时间命名；同一秒重名则追加数字序号，不覆盖旧图。

### 5.4 前端 4 条内存防泄漏红线
1. **历史零像素常驻**：贴图关闭后只保留磁盘路径和元数据索引，GJS 中绝不缓存已关闭贴图的 `Cogl.Texture` / `GdkPixbuf`。
2. **超大图/长图纹理降采样**：前端加载显示纹理前检查图片尺寸，若任一边超过 `max_texture_edge_px`（默认 `4096px`）或超过当前显示器物理分辨率的 2 倍，使用 `GdkPixbuf.Pixbuf.new_from_file_at_scale` 按上限等比降采样生成显示纹理；磁盘保留 100% 原图供复制、标注与保存。
3. **迷你缩略图态释放大图纹理**：贴图切入迷你态时，销毁原高清图像纹理并切换加载几十 KB 的 `thumbs/<id>.png`；展开时再从磁盘毫秒级重新载入清晰纹理。
4. **严格断连销毁**：每个 `PinActor` 维护全部信号 ID 列表；关闭或扩展 `disable()` 时显式断开所有 Clutter/Stage/Monitor 信号、销毁右键菜单和子节点、清空纹理引用并置 `null`，确保长时间不重启桌面零内存泄漏。

---

## 6. 未来 Niri + DMS 前端扩展蓝图

得益于核心与前端的完全解耦，未来迁移至 **Niri + DMS（DankMaterialShell）** 时：
1. **Core 零改动复用**：`pinshot-core` 与 `pinshot-cli`（5 个主命令 + 兜底命令、前端截图协议与 Portal ScreenCast、剪贴板文字转图、长图拼接、`satty` 回流、缓存双阈值清理、配置文件、二期 OCR）100% 直接沿用。
2. **新增极薄 Niri 前端（`crates/pinshot-niri`）**：
   - 使用 `gtk4-rs` + `gtk4-layer-shell`，在 `Layer::Overlay` 创建贴图表面，天然实现「非窗口、永远置顶、不进 DMS 任务栏、跨工作区显示、精确坐标定位与点击穿透（空 input region）」。
   - 实现与 GNOME 前端完全一致的 `io.github.dgxz99.Pinshot.Frontend1` D-Bus 接口；当所有贴图关闭后进程自动退出（回到 0 MB 闲置）。
   - 支持通过 DMS 的全局快捷键调用贴图管理命令。

---

## 7. 分阶段开发路线图与验收标准

### M0：基础脚手架与基础设施
- **交付内容**：
  - 初始化 Rust Workspace（`pinshot-core`、`pinshot-cli`）与 GNOME 51 扩展骨架（`pinshot@dgxz99`）；
  - 实现 `config.rs`（`config.toml` 解析与默认生成）与 `store.rs`（`0700` 目录、原子 JSON 索引、`200 MB / 14 天` 双阈值自动淘汰、单条与全局物理删除）；
  - 定义并打通 `io.github.dgxz99.Pinshot.Frontend1` D-Bus `Ping` 双向联通；
  - 编写 `scripts/install.sh` 与 `scripts/uninstall.sh`。
- **验收标准**：
  - `pinshot --help` 正常输出子命令；扩展在 GNOME 51 成功启用且 `journalctl` 无报错；
  - 单元测试验证双阈值清理（超空间、超天数、活跃贴图豁免、隐私彻底删除）。

### M1：核心闭环（抓屏 + 剪贴板文字转图 + 真·非窗口贴图）
- **交付内容**：
  - Core：`capture.rs`（接收前端截图和导出 PNG）、`clipboard.rs`（多图片 MIME + `text/uri-list` + 纯文本探测 + 空剪贴板通知）、`text_render.rs`（代码检测、Pango 转义、折行、超长保护、主题自适应）；
  - GNOME 前端：`PinActor` 完整实现（无边框非窗口渲染、分数缩放 1:1 逻辑尺寸、左键拖动、滚轮缩放、`Ctrl+滚轮` 透明度、迷你缩略图、双击左键或中键关闭、右键菜单、4 条内存防泄漏红线）；
  - 支持通过命令恢复最近贴图。
- **验收标准**：
  - `pinshot capture-pin` 与 `pinshot pin-clipboard`（图片/代码/中文段落/文件管理器复制图）稳定弹出真·非窗口贴图；
  - 验证贴图不出现在 `aztaskbar`、`CoverflowAltTab`、Overview，不被 `rounded-window-corners` 影响；
  - 125%/150% 分数缩放下贴图与原画面视觉 1:1 严格对齐；
  - 连续创建并关闭 30 张大图后，GNOME Shell 内存回落正常无持续增长。

### M2：工作流精修（标注回流 + 穿透 + 防误触 + 边界保护）
- **交付内容**：
  - `annotate.rs`：`satty` 标注回流状态机（临时隐藏 → 保存覆盖并保留旧版进历史 / `Esc` 取消无损恢复）；
  - 键盘防误触（单击选中描边响应快捷键、点外部归还焦点）；
  - 点击穿透（`unghost-all` 恢复与透明度视觉提示）；
  - 全屏应用自动避让隐藏、`toggle-hide`、多图 Z-Order、拖拽 32px 钳制、外接屏热插拔回收；
  - `restore-last` 与重启后历史顶部恢复流程。
- **验收标准**：
  - 贴图进 `satty` 保存后原地刷新，取消后无损恢复；
  - 打开全屏视频时贴图自动隐藏、退出即回归；
  - 开启点击穿透后鼠标可直击底层编辑器，并通过快捷键及 CLI 均可可靠解除；
  - 模拟重启扩展/会话后，未关闭贴图安静出现在历史顶部且可一键找回。

### M3：滚动长图（`pinshot longshot`）
- **交付内容**：
  - `longshot/screencast.rs`（Portal ScreenCast + `restore_token` 复用）；
  - 前端 `regionSelector.js`（透明框选纯滚动内容区）与 `longshotHud.js`（悬浮控制条：实时高度、快滚告警、完成/取消）；
  - `longshot/stream.rs`（GStreamer `pipewiresrc` 裁剪取帧）与 `longshot/stitcher.rs`（纯 Rust 实时拼接、静止去重、忽略回滚、滚动过快提醒）；
  - 录前自动隐藏旧贴图、录完自动恢复、自动复制剪贴板并按适配高度贴图。
- **验收标准**：
  - 在浏览器长网页、终端输出、带固定顶栏/底栏的聊天窗口中手动滚动均能稳定无缝拼接；
  - 刻意快速滚动触发悬浮条告警、回滚不出现重复叠影；
  - 超过 8000px 长图贴图展示流畅且前端纹理降采样生效，复制/保存导出仍为 100% 原始清晰度。

### M4：二期能力扩展
- **交付内容**：
  - `ocr.rs`：纯 Rust AI 视觉 API 集成（OpenAI 兼容 / Gemini / 本地 Ollama），右键菜单触发 OCR 并支持复制结果；
  - `hooks.rs`：`config.toml` 自定义外部命令钩子接入右键菜单；
  - （切桌面时启动）`crates/pinshot-niri`：`gtk4-rs + gtk4-layer-shell` 前端与 DMS 快捷键接入。

---

## 8. 关键风险与应对策略

| 风险点 | 潜在表现 | 应对措施 |
|---|---|---|
| **GJS 扩展纹理/信号泄漏** | 长时间开机高频贴图后 `gnome-shell` 内存缓慢上涨 | 严格落实 4 条内存红线；为 `PinActor` 建立统一销毁清单；M1 阶段执行 30+ 次高频贴图/迷你态/关闭回归压测 |
| **长图遇到滚动条或固定悬浮按钮** | 右侧滚动条滑块随页面反向移动干扰重叠匹配 | 匹配窗口默认裁剪左右边缘 5% 区域，结合下采样条带与归一化相关性阈值过滤局部干扰 |
| **GStreamer PipeWire 流在部分环境下帧率抖动** | 帧时间戳间隔大导致拼接断层 | `appsink` 设 `drop=true`、`max-buffers=2` 防止积压延迟；匹配失败时不强行拼接并立刻向 HUD 发出回滚提示 |
| **GNOME 大版本升级（51 → 52）** | 扩展因 `shell-version` 或个别 Clutter API 变动失效 | 扩展代码严格保持极薄、仅使用稳定公开接口；所有重业务逻辑留在 Rust Core，升级时只需极小成本适配前端 |
| **热键与当前焦点冲突** | 悬停贴图时打字误触发贴图关闭 | 强制「鼠标免焦点 + 键盘必须单击选中才生效」，点出贴图区域瞬间归还焦点 |
