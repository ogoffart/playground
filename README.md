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

## Comparison

All 14 are implemented, polished to the same [`SPEC.md`](./SPEC.md) (full-width top toolbar, real
hierarchical file tree, per-file +/− counts, system light/dark theme), and built with a
size-optimized release profile (`opt-level="z"`, `lto`, `codegen-units=1`, `panic="abort"`,
`strip`). Each was build-verified and smoke-tested headless; 12 have a fresh border-free
`screenshot.png`. Every app shares the same 463-line `git2`+`syntect` data layer (`git.rs` +
`highlight.rs`); the LOC column below is the **framework-specific UI code on top of that**.

### Size, code & build

| Framework | Release binary | Compressed (gzip) | UI LOC | Runtime deps (not in binary) |
|---|--:|--:|--:|---|
| **qt-qmetaobject** | 2.5 MiB | 1.55 MiB | 952 | Qt 6 + QtQuick QML modules |
| **qt-widgets** | 2.5 MiB | 1.56 MiB | 1225 | Qt 6 |
| **gtk4** | 2.5 MiB | 1.56 MiB | 1321 | GTK 4 + GtkSourceView 5 |
| **qt-cxx** | 3.2 MiB | 1.72 MiB | 1074 | Qt 6 + QtQuick QML modules |
| **tauri** | 5.2 MiB | 2.58 MiB | 1329 | webkit2gtk-4.1 + system webview |
| **dioxus** | 6.0 MiB | 2.61 MiB | 1385 | webkit2gtk-4.1 |
| **makepad** | 6.8 MiB | 3.26 MiB | 1236 | GL driver only (self-contained) |
| **egui** | 9.1 MiB | 4.24 MiB | 991 | GL driver only (self-contained) |
| **iced** | 9.9 MiB | 4.65 MiB | 1139 | GPU/GL driver (self-contained) |
| **gpui** | 10.5 MiB | 4.93 MiB | 1336 | Vulkan (self-contained) |
| **slint** | 10.6 MiB | 4.89 MiB | 1274 | GPU/GL driver (self-contained) |
| **floem** | 11.3 MiB | 5.25 MiB | 1239 | GPU/GL driver (self-contained) |
| **freya** | 23.3 MiB | 10.4 MiB | 1404 | GPU/GL driver (self-contained, bundles Skia) |
| **xilem** | 27.4 MiB | 9.6 MiB | 1185 | Vulkan (self-contained, bundles Vello) |

> **Reading the sizes fairly:** the small binaries (Qt, GTK, web) are *not* actually smaller to
> ship — they dynamically link a large toolkit the user must already have (or you bundle it:
> Qt 6 ≈ 30–80 MB, GTK 4 ≈ 40 MB, a webview is usually system-provided). The self-contained
> Rust-renderer apps statically link everything (font shaping, GPU abstraction, and for freya a
> whole copy of Skia, for xilem all of Vello), so their binary *is* the shippable artifact. So
> "smallest binary" and "smallest real download" are nearly opposite rankings.

### Spec coverage, ergonomics & score

| Framework | Notable framework-specific limitations | Ease of use (building this app) | Score |
|---|---|---|:--:|
| **tauri** | per-file sticky is per-section CSS (visually GitHub-identical) | Easiest to nail the GitHub look — plain HTML/CSS/JS over a thin Rust IPC layer; `position:sticky`, flexbox, collapsible tree all trivial. Needs a JS/webview mental model. | **9.0** |
| **qt-qmetaobject** | syntect theme stays light in dark mode (chrome switches) | Very productive: `qt_property!`/`qt_method!` macros + model-as-JSON; QML gives `SplitView` resize and `ListView` **section** sticky headers for free. Qt 6 dependency. | **8.5** |
| **gtk4** | sticky is an overlay label, not an in-flow pinned header | The most "batteries included": real `GtkPaned`, `TreeListModel` tree, `GtkSourceView`. gtk4-rs is mature but verbose, and the build needs system GTK. | **8.0** |
| **egui** | sticky header emulated as a redrawn overlay | Immediate-mode is quick to reason about; resizable panels are one call. The diff rows are hand-painted (`Painter`), which is flexible but lower-level than a retained tree. | **8.0** |
| **slint** | flat model ⇒ tree flattened in Rust; syntect light in dark | Clean declarative `.slint` markup; the `Theme` global + `Palette.color-scheme` made dark mode tidy. No splitter/tree widgets, so those are hand-rolled. | **8.0** |
| **iced** | no native sticky/tree widgets (overlay + manual tree) | Elm architecture is predictable and `pane_grid` handles resizing; rich-text spans map cleanly. Some boilerplate and estimated scroll geometry. | **7.5** |
| **dioxus** | syntect light in dark mode; needs a `libxdo` link stub headless | RSX + CSS felt like comfortable web dev, all in Rust, with signals; the recursive tree was natural. Transitive `libxdo`/tray deps are an annoyance. | **7.5** |
| **floem** | no sticky primitive (headers scroll inline) | Fine-grained signals made the collapsible tree and theme refactor pleasant; closure-based styling means threading a palette through many closures. | **7.5** |
| **qt-cxx** | syntect light in dark; long, finicky build | `#[cxx_qt::bridge]` is clean once set up; reused the qmetaobject QML verbatim. The CXX/Pin borrow dance and ~6-min LTO builds are the cost. | **7.0** |
| **qt-widgets** | sticky not pinned (stacked `QTextEdit`s); rich text via HTML | Predictable QtWidgets via a C++ shim + C-ABI; top toolbar is free in `QMainWindow`. Colored text/counts need HTML, and you maintain the FFI/JSON boundary. | **7.0** |
| **freya** | no sticky / scroll-into-view (pixel-offset approximations); 23 MB binary | Nice builder API — `ResizableContainer` and tooltips are free, `paragraph().span()` eats syntect spans directly. Skia-from-source is a heavy build & binary. | **7.0** |
| **gpui** | sticky not expressible in one `uniform_list`; **no headless screenshot** (Vulkan swapchain) | Ergonomic tailwind-ish `div()` once you accept the retained model; but few batteries (no splitter/tooltip/sticky widgets) and a git dependency. | **6.5** |
| **makepad** | word-wrap clips; sticky not pinned; **text doesn't paint under software GL** | Powerful shader/`live_design!` DSL with real `Splitter`+`PortalList`, but the steepest learning curve and the most custom drawing (a whole custom `DiffRow` widget). | **5.5** |
| **xilem** | alpha API sharp edges; one label per span; no scroll-to-child; flaky headless capture | Promising reactive design, but it's alpha: `Style`-vs-inherent method ordering, `Send+Sync` state (Repo behind a `Mutex`), and a thin widget set mean more friction today. | **5.5** |

*Scores are a subjective blend of spec completeness, how GitHub-like the result looks, ergonomics,
maturity, and shippability for **this** app — not a general verdict on the frameworks.*

### Headless-render caveats (this container only)

- **makepad** builds and lays out (toolbar, splitters, tree all visible) but its GPU SDF text pass
  doesn't paint under Mesa llvmpipe — glyphs are blank in the capture; fine on a real GPU.
- **gpui** and **xilem** render through Vulkan swapchains (Blade / Vello); under software Vulkan
  (lavapipe) the presented frames aren't reliably grabbable by X11 tools, so their screenshots may
  be absent or not reflect the latest UI. Both build and run correctly.

### Other Rust UI frameworks not included

Beyond the 14 above, the remaining notable Rust GUI options are either superseded or niche:
**Druid** (deprecated, succeeded by Xilem), **Vizia**, **Cushy**, **Ribir**, and **Bevy UI**
(game-engine UI). Any of these could be added following the same `SPEC.md` contract.
