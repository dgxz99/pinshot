<div align="center">
  <img src="extensions/pinshot@dgxz99/icons/pinshot.svg" width="112" height="112" alt="Pinshot logo" />
  <h1>Pinshot</h1>
  <p><strong>Take screenshots, pin references, and record GIFs on GNOME Wayland.</strong></p>
  <p><strong>English</strong> · <a href="README.zh-CN.md">Simplified Chinese</a></p>
  <p>
    <a href="#installation"><img src="https://img.shields.io/badge/GNOME%20Shell-50%20%7C%2051-4A86CF?style=flat-square&amp;logo=gnome&amp;logoColor=white" alt="GNOME Shell 50 / 51" /></a>
    <a href="#installation"><img src="https://img.shields.io/badge/Wayland-F29B26?style=flat-square&amp;logo=wayland&amp;logoColor=white" alt="Wayland" /></a>
    <a href="Cargo.toml"><img src="https://img.shields.io/badge/Rust-B7410E?style=flat-square&amp;logo=rust&amp;logoColor=white" alt="Rust" /></a>
    <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-2C8A5B?style=flat-square&amp;logo=opensourceinitiative&amp;logoColor=white" alt="MIT License" /></a>
  </p>
  <p><a href="#features">Features</a> · <a href="#installation">Installation</a> · <a href="#quick-start">Quick start</a> · <a href="#roadmap">Roadmap</a> · <a href="https://github.com/dgxz99/pinshot/issues">Report an issue</a></p>
</div>

Pin a screenshot, an image, or a copied snippet next to your work so you can refer to it without switching windows. Pins stay above ordinary windows and follow you across workspaces. They’re drawn by GNOME Shell, so they don’t show up in `Alt+Tab`.

**Supported desktop:** GNOME Shell 50 / 51 on Wayland.

![Pinshot’s adjustable capture area and action toolbar on GNOME Wayland](docs/images/region-capture.png)

## Features

| Feature | What it does |
| --- | --- |
| Region & fullscreen capture | Freeze the desktop and adjust the selection with eight handles, or grab the active monitor fullscreen. Copy, save, pin in place, or annotate. |
| Desktop pins | Keep screenshots, clipboard images, and text cards on top. Region screenshots pin 1:1 where you selected them; move, resize, change opacity, collapse, or let clicks pass through. |
| Scrolling capture | Capture a page or scrollable panel vertically or horizontally. Extend either end of the chosen axis without repeating overlapping content. |
| GIF recording | Choose a frame rate before recording, include the pointer, and stop to save a looping GIF. |
| Top-bar menu & launcher | GNOME top-bar icon (color or monochrome) with quick actions and a shortcut toggle, plus an application launcher for Preferences and quick captures. |
| Annotation | Optional: open a selection or edit an existing pin in [Satty](https://github.com/Satty-org/Satty). |

## Installation

### Arch Linux (AUR)

```bash
yay -S pinshot-bin   # Prebuilt binary
# or
yay -S pinshot       # Build from source
```

### Prebuilt install script (Linux `x86_64`)

```bash
curl -fsSL https://raw.githubusercontent.com/dgxz99/pinshot/main/scripts/install.sh | bash
```

- Specific version: `curl -fsSL https://raw.githubusercontent.com/dgxz99/pinshot/main/scripts/install.sh | PINSHOT_VERSION=v0.1.0 bash`

### Build from source

Requires Git, Rust and Cargo (tested with Rust 1.100), a C toolchain, `pkg-config`, `glib-compile-schemas`, and GStreamer Core/App/Video development files in addition to the runtime dependencies:

```bash
sudo pacman -S --needed git base-devel glib2 wl-clipboard pango xdg-desktop-portal-gnome pipewire wireplumber gstreamer gst-plugins-base gst-plugins-base-libs gst-plugin-pipewire
```

```bash
git clone https://github.com/dgxz99/pinshot.git
cd pinshot
./scripts/install.sh
```

For development, `--symlink` links the extension directory to the checkout; `--skip-build` installs an existing `target/release/pinshot` without rebuilding.

### Dependencies

| Dependency | Used for |
| --- | --- |
| GNOME Shell 50 / 51, `gnome-extensions` | Running and managing the extension |
| `wl-clipboard`, `pango-view` (Pango) | Clipboard access and text cards |
| GNOME ScreenCast Portal, PipeWire, GStreamer (`pipewiresrc`, `videorate`, `videoconvert`) | Scrolling capture and GIF recording |
| `satty`, `notify-send` (`libnotify`) | Optional: annotation and desktop notifications |

### Enable the extension

GNOME Wayland only loads newly installed or updated extensions when a session starts, so **log out and back in** after installing:

- **Script / source install:** the script pre-enables the extension automatically, so it activates as soon as you log back in.
- **AUR install:** system packages don’t modify per-user GNOME settings; after logging back in, enable it once (or turn it on in the Extensions app):

```bash
gnome-extensions enable pinshot@dgxz99
```

Verify that the extension is online (returns `gnome-shell-50` or `gnome-shell-51`):

```bash
pinshot ping
```

> For script installs, if `pinshot` isn’t found in your terminal, add `~/.local/bin` to `PATH`:
> - **Bash / Zsh:** add `export PATH="$HOME/.local/bin:$PATH"` to `~/.bashrc` or `~/.zshrc`
> - **Fish:** run `fish_add_path ~/.local/bin`

## Quick start

Trigger actions via global shortcuts, the GNOME top-bar icon, the Pinshot app launcher (click for Preferences, right-click for quick actions), or the CLI:

| Action | Default shortcut | CLI |
| --- | --- | --- |
| Capture a region | `Super+Shift+P` | `pinshot capture` |
| Capture active monitor fullscreen | Not bound | `pinshot capture-fullscreen` |
| Start a vertical scrolling capture | `Super+Shift+L` | `pinshot longshot` |
| Pin an image or text from the clipboard | `Super+Shift+V` | `pinshot pin-clipboard` |
| Make all pins respond to clicks again | `Super+Shift+U` | `pinshot unghost-all` |
| Record a GIF | Not bound | `pinshot gif` |
| Capture and annotate | Not bound | `pinshot capture-edit` |
| Show or hide all pins | Not bound | `pinshot toggle-hide` |
| Restore the most recently closed pin | Not bound | `pinshot restore-last` |

`Super` is usually the Windows key. Open preferences from the top-bar menu, the Pinshot app icon, or `gnome-extensions prefs pinshot@dgxz99`. The preferences UI currently uses Simplified Chinese.

### Capture, copy, or pin

1. Press `Super+Shift+P`, drag out a region, and release. Move the selection by dragging inside it, or resize it with the handles.
2. The toolbar goes from left to right: **Scrolling capture → GIF → Annotate → Pin → Save → Cancel → Copy**. `Enter` copies only; `Esc` or right-click cancels. Save exports a PNG without changing the clipboard. Pin keeps the selection in place at 1:1 size and copies it. Use `pinshot capture-pin` if you want `Enter` to pin instead.
3. For fullscreen capture of the active monitor, use the top-bar menu, `pinshot capture-fullscreen`, or bind a shortcut in preferences. It flashes the monitor briefly, copies the PNG to the clipboard, and respects auto-save and notification settings.
4. In the selection, `Ctrl+C` copies, `P` pins, and `E` opens Satty. These three bindings are configurable independently of global shortcuts: for example, set `Ctrl+T` to pin the current selection without changing what `Ctrl+T` does outside it. In Satty, `Enter` saves and copies the annotated capture; `Esc` leaves the clipboard unchanged.

### Work with pins

Copy an image or some text in any app and press `Super+Shift+V`. Pinshot doesn’t take over the app’s `Ctrl+C`. A screenshot copied by Pinshot keeps its original display size when pinned again, subject to fit-to-screen for large images. For external images on scaled displays, the Screenshots page offers a device-pixel 1:1 option.

| Action | Gesture |
| --- | --- |
| Select and bring forward / move | Left-click / drag |
| Close and keep in history | Double-click or middle-click |
| Zoom / change opacity | Scroll / `Ctrl`+scroll |
| Copy, annotate, save, or reset the size | Right-click for the menu |
| Collapse or expand a pin | Right-click menu or `Shift+R` |
| Enable click-through | Right-click menu or `T` |

When a pin receives keyboard input, `Ctrl+C` copies it, `E` opens Satty, `0` resets its size, and `Esc` closes it. If another window is receiving the keys, use the pin’s right-click menu instead. Once click-through is enabled, use `Super+Shift+U`, the top-bar menu, or `pinshot unghost-all` to regain mouse control. By default, pins on a display temporarily hide when an app goes fullscreen there.

### Capture scrolling content

1. Select the area in a normal capture, then choose vertical or horizontal from the leftmost toolbar button. You can also start from the top-bar menu or run `pinshot longshot` / `pinshot longshot --horizontal`.
2. Choose the monitor in GNOME’s screen-sharing dialog. A normal capture reuses the selection after authorization; the dedicated command asks you to select the area on the shared monitor. The whole selection must fit on that monitor, with room outside it for the controls.
3. Scroll along the selected axis in either direction. If Pinshot asks you to scroll back, return to where stitching last succeeded, then continue with smaller steps. For something like a scrollable code block, select the content and leave unrelated sidebars or overlays out of the frame.
4. Choose Annotate, Pin, Save, or Copy to finish, or click × to discard. The controls and capture border stay out of the output. Capture finishes automatically when the scrolling axis reaches 24,000 pixels or the memory limit is reached.

GNOME can remember the shared monitor. To choose a different one, use `pinshot longshot --choose-screen`. GIF recording shares this authorization and cannot run at the same time as scrolling capture. Pinshot closes the screen-sharing session when it’s done. Stopping sharing from GNOME cancels the capture in progress.

### Record a GIF

1. Choose the camera icon in a normal capture, click **录制 GIF** in the top-bar menu, or run `pinshot gif` and confirm a region. The blue outline and `00:00` timer mean it’s ready, not recording.
2. Choose **5, 10, 16, 24, or 30 FPS** (default: 10). Click the red circle to start the screen-sharing flow. If you need a different monitor, use `pinshot gif --choose-screen`.
3. When the outline turns red and the timer runs, click the square stop button to save, or × to discard. Saving shows the full file path in a desktop notification when `notify-send` is available. It does not copy the GIF to the clipboard or add it to pin history.

GIFs loop and include the pointer, but not audio. The chosen FPS is a sampling ceiling, not a promise that the screen will update that often. Large selections are scaled proportionally to a longest edge of 1,280 pixels and a total area of at most 1,280 × 720 pixels. There is no fixed time limit; at 64 MiB Pinshot saves the portion already recorded.

## Saving and settings

Open extension preferences (**快捷键 / Shortcuts**, **截图 / Screenshots**, **界面 / Appearance**):

| Setting | Behavior |
| --- | --- |
| Save directory | Shared by Pinshot PNGs and GIFs. Use an absolute path or `~/`, then click Apply. A blank field uses `general.save_dir` in `config.toml` (default `~/Pictures/Pinshots`). |
| Filename prefix | Defaults to `pinshot`. Filenames include the local save time, such as `pinshot-20261121-214530.png`, with `-1`, `-2` suffixes on collision. |
| Automatically save to disk | Off by default. When enabled, also exports pinned, annotated, and fullscreen captures. Explicit Save and GIF stopping always write to disk. |
| Screenshot completion notifications | Off by default. Controls success notifications for screenshot flows. |
| Top-bar icon & style | Show or hide the GNOME top-bar icon and switch between the **color** logo (default) and **monochrome** symbolic icon. Applies immediately. |

### Cache and history

`pinshot status` reports the frontend status, cache and configuration paths, and pin counts. Cache files live under `~/.cache/pinshot/` (or `$XDG_CACHE_HOME/pinshot/`). The default limits are 200 MB and 14 days, evicting inactive pins first. Pins aren’t reopened on login unless `restore_on_startup` is enabled; use `pinshot restore-last` to recover a recent pin.

```bash
pinshot clear-history       # Clear inactive history, keep active pins
pinshot clear-history --all # Also close active pins and delete their cached files
```

Clearing history is irreversible, but does not delete exported PNG or GIF files.

<details>
<summary>Core configuration (<code>~/.config/pinshot/config.toml</code>)</summary>

`pinshot status` creates the file with defaults at `~/.config/pinshot/config.toml` (or `$XDG_CONFIG_HOME/pinshot/config.toml`):

```toml
[general]
save_dir = "~/Pictures/Pinshots"
restore_on_startup = false
notify_on_save = true             # Notifications for explicit PNG saves

[cache]
max_size_mb = 200
max_age_days = 14

[pin]
default_opacity = 1.0             # 0.15..=1.0
min_scale = 0.2
max_scale = 5.0
zoom_step = 0.1
opacity_step = 0.08
mini_size_px = 64
min_visible_edge_px = 32
max_texture_edge_px = 4096
hide_on_fullscreen = true

[text_card]
theme = "auto"                    # auto, light, or dark
max_width_px = 680
max_lines = 80
prose_font = "Sans 12"
code_font = "Monospace 11"
padding_px = 16
```

Text-card and cache settings apply to new items immediately. The extension reads `[pin]` settings on startup; log out and back in after changing them.

</details>

## Roadmap

- **Broader GNOME support:** Test and support additional GNOME Shell versions (such as Ubuntu LTS and other GNOME Wayland distributions).
- **Other Wayland compositors:** Explore standalone layer-shell frontends (GTK4 or Qt6) for compositors like **niri**, reusing the Rust core.

## Troubleshooting and uninstalling

- **The extension is offline:** Verify you’re in a GNOME Shell 50 / 51 Wayland session and check `gnome-extensions info pinshot@dgxz99`. If GNOME is still running older extension code, log out and back in.
- **A shortcut does nothing:** Try the corresponding CLI command first, then check whether global shortcuts are paused in the top-bar menu or conflict with a system shortcut.
- **Need logs?** Run `journalctl --user -b -o cat | grep -i pinshot`. When [reporting an issue](https://github.com/dgxz99/pinshot/issues), include your GNOME Shell version, whether the session is Wayland, and steps to reproduce it.

### Uninstall

- **Script install** (removes CLI, extension, and launcher; set `PINSHOT_PURGE=1` for a full uninstall including config, cache, and GSettings):
  ```bash
  curl -fsSL https://raw.githubusercontent.com/dgxz99/pinshot/main/scripts/uninstall.sh | bash
  # Full uninstall (keeps exported PNG/GIF files):
  curl -fsSL https://raw.githubusercontent.com/dgxz99/pinshot/main/scripts/uninstall.sh | PINSHOT_PURGE=1 bash
  ```
  If you cloned the repository, you can run `./scripts/uninstall.sh` or `./scripts/uninstall.sh --purge` directly.
- **AUR install:**
  ```bash
  sudo pacman -Rns pinshot-bin   # or pinshot
  # Optional: remove user config, cache, and GSettings
  rm -rf ~/.config/pinshot ~/.cache/pinshot && dconf reset -f /io/github/dgxz99/pinshot/
  ```

## License

Pinshot is available under the [MIT License](LICENSE).
