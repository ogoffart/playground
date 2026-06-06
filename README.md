# git-review — the same code-review app in every Rust UI framework

A small but non-trivial desktop application for **reviewing code in git** (a GitHub-style
"Files changed" view), implemented **independently in every major Rust UI framework** so the
frameworks can be compared on equal footing.

Each app is a **standalone project** in its own folder. They share no code — only a common
[`SPEC.md`](./SPEC.md) describing the exact behaviour and look, and the same underlying
libraries (`git2` for git, `syntect` for highlighting).

> See [`SPEC.md`](./SPEC.md) for the full feature & visual contract every app implements.

## The app

A left **resizable** side panel (commit list on top, file tree below — also resizable) and a
main scrollable **diff view** with a toolbar. Pick a commit to `git show` it, or pick two
endpoints to `git diff` them. Sticky per-file headers, a `+`/`-` gutter, syntax highlighting,
and red/green line backgrounds. Run an app with a repo path: `<app> /path/to/repo`
(defaults to the current directory).

## Frameworks

| Folder | Framework | Style | System deps |
|---|---|---|---|
| [`egui/`](./egui) | [egui](https://github.com/emilk/egui) 0.34 | immediate mode | wgpu/GL |
| [`iced/`](./iced) | [iced](https://github.com/iced-rs/iced) 0.14 | Elm / retained | wgpu/GL |
| [`slint/`](./slint) | [Slint](https://slint.dev) 1.x | declarative markup | — |
| [`tauri/`](./tauri) | [Tauri](https://tauri.app) 2.x | web frontend + Rust | webkit2gtk, node |
| [`dioxus/`](./dioxus) | [Dioxus](https://dioxuslabs.com) 0.7 | RSX (webview) | webkit2gtk |
| [`floem/`](./floem) | [Floem](https://github.com/lapce/floem) | fine-grained reactive | wgpu/GL |
| [`xilem/`](./xilem) | [Xilem](https://github.com/linebender/xilem) | reactive (alpha) | wgpu/Vulkan |
| [`gtk4/`](./gtk4) | [gtk4-rs](https://gtk-rs.org) + Relm4 | native GTK4 | gtk4, gtksourceview-5 |
| [`makepad/`](./makepad) | [Makepad](https://makepad.dev) | shader DSL | GL |
| [`freya/`](./freya) | [Freya](https://freyaui.dev) | Skia + reactive | — |
| [`gpui/`](./gpui) | [GPUI](https://www.gpui.rs) (Zed) | GPU retained | Vulkan |
| [`qt-qmetaobject/`](./qt-qmetaobject) | [qmetaobject-rs](https://github.com/woboq/qmetaobject-rs) | Qt Quick / QML | Qt6 |
| [`qt-cxx/`](./qt-cxx) | [cxx-qt](https://github.com/KDAB/cxx-qt) (KDAB) | Qt Quick / QML | Qt6 |
| [`qt-widgets/`](./qt-widgets) | [`qt_widgets`](https://github.com/rust-qt) (ritual) | Qt Widgets | Qt6 |

Each folder has its own `README.md` with exact build/run instructions and any caveats.

## Building

The native apps build `git2`'s vendored libgit2 (needs `cmake` + a C compiler). Framework
system dependencies are listed per row above. To get a repo to point the apps at:

```sh
tools/make-sample-repo.sh            # creates /tmp/git-review-sample
egui/target/debug/git-review-egui /tmp/git-review-sample
```

## Status

Apps are landed one framework at a time, each build-verified and smoke-tested headless
(`xvfb` + a screenshot against the sample repo). See each folder's `README.md` and
`screenshot.png` for details.

All 14 are implemented and build-verified. 12 were captured rendering headless (see each folder's
`screenshot.png`); the two exceptions are noted below and are environment limitations of this
headless/no-GPU container, not app bugs.

| Framework | Build | Headless render | Notes |
|---|---|---|---|
| egui | ✅ | ✅ shot | immediate-mode; hand-laid diff rows, overlay sticky header |
| gtk4 | ✅ | ✅ shot | `GtkPaned` splits, TextView/TextTag diff, overlay sticky |
| slint | ✅ | ✅ shot | declarative markup, draggable splitters, Flickable + floating sticky |
| iced | ✅ | ✅ shot | `pane_grid` splits, `rich_text` spans, overlay sticky |
| qt-qmetaobject | ✅ | ✅ shot | QML `SplitView`, `ListView` sections = **true** sticky headers |
| tauri | ✅ | ✅ shot | web frontend, CSS `position: sticky`, collapsible file tree |
| dioxus | ✅ | ✅ shot | RSX + CSS sticky headers; libxdo link stub for headless |
| floem | ✅ | ✅ shot | reactive signals, draggable splitters, lavapipe render |
| qt-cxx | ✅ | ✅ shot | cxx-qt bridge; same QML as qmetaobject (clean A/B) |
| qt-widgets | ✅ | ✅ shot | **native QtWidgets** via a C++ shim + C-ABI (no QML/moc) |
| xilem | ✅ | ✅ shot | alpha (Masonry/Vello); one label per span; lavapipe render |
| makepad | ✅ | ⚠ layout only | `live_design!` DSL, `Splitter`+`PortalList`, custom `DiffRow`; text glyphs don't paint under software GL (llvmpipe) |
| gpui | ✅ | ⚠ no capture | Zed's framework; runs on Vulkan (lavapipe) but the swapchain present path isn't screen-capturable headless |
| freya | ✅ | ✅ shot | Skia + own reactive core; builder API; lavapipe render |

All native apps build `git2`'s vendored libgit2 (needs `cmake` + a C compiler). The three Qt apps
need Qt 6 (+ the QtQuick QML modules for the two QML ones); the GTK app needs GTK 4; the web apps
(tauri, dioxus) need webkit2gtk; the wgpu/GL apps render headless via Mesa **lavapipe** (software
Vulkan) or `LIBGL_ALWAYS_SOFTWARE=1`.

### Other Rust UI frameworks not included

Beyond the 14 above, the remaining notable Rust GUI options are either superseded or niche:
**Druid** (deprecated, succeeded by Xilem), **Vizia**, **Cushy**, **Ribir**, and **Bevy UI**
(game-engine UI). Any of these could be added following the same `SPEC.md` contract.
