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

`+ toolkit libraries` is the actual set of shared libraries the binary loads, **excluding** the
guaranteed-present Linux desktop base (glibc, X11/xcb, GL/EGL/Vulkan/drm, wayland, xkbcommon,
fontconfig/freetype, alsa/pulse, dbus/systemd, zlib) — measured by `ldd`-closure. "Total bundle"
is what a fully self-contained download (AppImage-style) would actually contain; the gzip column
is that bundle compressed. UI LOC excludes the shared 463-line `git2`+`syntect` data layer.
Rows sorted by total bundle.

| Framework | Stripped binary | + Toolkit libraries (bundled) | = Total bundle | Bundle gzip | UI LOC |
|---|--:|--:|--:|--:|--:|
| **makepad** | 6.8 MiB | — self-contained | 6.8 MiB | 3.3 MiB | 1236 |
| **egui** | 9.1 MiB | — self-contained | 9.1 MiB | 4.2 MiB | 991 |
| **iced** | 9.9 MiB | — self-contained | 9.9 MiB | 4.7 MiB | 1139 |
| **gpui** | 10.5 MiB | — self-contained | 10.5 MiB | 4.9 MiB | 1336 |
| **slint** | 10.6 MiB | — self-contained | 10.6 MiB | 4.9 MiB | 1274 |
| **floem** | 11.3 MiB | — self-contained | 11.3 MiB | 5.3 MiB | 1239 |
| **freya** | 23.3 MiB | — self-contained (bundles Skia) | 23.3 MiB | 10.4 MiB | 1404 |
| **xilem** | 27.4 MiB | — self-contained (bundles Vello) | 27.4 MiB | 9.6 MiB | 1185 |
| **gtk4** | 2.5 MiB | +28.8 MiB · GTK 4 + GtkSourceView (35 libs) | **31.4 MiB** | 12.4 MiB | 1321 |
| **qt-widgets** | 2.5 MiB | +66.0 MiB · Qt 6 Widgets (24 libs) | **68.6 MiB** | 27.9 MiB | 1225 |
| **qt-cxx** | 3.2 MiB | +77.9 MiB · Qt 6 Quick/QML — no QtWidgets (53 libs) | **81.2 MiB** | 32.7 MiB | 1074 |
| **qt-qmetaobject** | 2.5 MiB | +85.1 MiB · Qt 6 Quick/QML — incl. QtWidgets (54 libs) | **87.7 MiB** | 35.5 MiB | 952 |
| **tauri** | 5.2 MiB | +196.9 MiB · WebKitGTK + GTK (80 libs) | **202.2 MiB** | 75.6 MiB | 1329 |
| **dioxus** | 6.0 MiB | +196.9 MiB · WebKitGTK + GTK (81 libs) | **203.0 MiB** | 75.6 MiB | 1385 |

> **The headline:** "smallest binary" and "smallest download" are almost opposite rankings. The
> Qt/GTK/web apps have the tiniest binaries (2.5–6 MiB) but pull in 29–197 MiB of toolkit
> libraries; the self-contained Rust-renderer apps statically link everything (egui's whole
> renderer, freya's copy of Skia, xilem's Vello) so the binary *is* the bundle.
>
> **In practice you often don't ship the toolkit:** webkit/GTK/Qt are frequently already on the
> user's machine (or installed via the package manager), in which case tauri/dioxus/gtk4/qt ship
> just their 2.5–6 MiB binary. The `+ toolkit` column is the **worst-case fully-bundled** cost
> (e.g. a self-contained AppImage). The self-contained Rust apps always pay their full binary but
> depend on nothing beyond the base desktop. Sizes are this Linux build; Windows/macOS differ
> (e.g. tauri uses the OS WebView ≈ 0 extra; Qt/GTK would still be bundled).

### Strengths, limitations & score

| Framework | 🟢 Strengths | 🔴 Limitations | Score |
|---|---|---|:--:|
| **tauri** |🟢 Web frontend = fastest path to a pixel-perfect GitHub look (CSS `sticky`/flex/grid, collapsible tree); huge web ecosystem; thin, small Rust core; mature bundler & security model. |🔴 Relies on the platform **WebView** (webkit2gtk on Linux) — a heavy runtime dep and per-platform rendering differences; two-language + IPC boundary; not native widgets. | **9.0** |
| **qt-qmetaobject** |🟢 QML gives resizable `SplitView` and `ListView` **section** sticky headers *for free*; mature Qt Quick widgets/animation; productive `qt_property!`/`qt_method!` macros; tiny app binary. |🔴 Large Qt 6 runtime dep; QML↔Rust boundary is dynamically typed (JSON-over-property here); Qt 6.4 QML lacks colour-scheme (needed Rust-side detection); not pure Rust. | **8.5** |
| **gtk4** |🟢 The most **batteries-included** native toolkit: real `GtkPaned`, `TreeListModel` tree, `GtkSourceView`, native dark-mode & a11y; tiny app binary. |🔴 Big GTK 4 stack to ship/depend on; gtk4-rs is verbose with GObject ceremony; least "native" on Windows/macOS. | **8.0** |
| **egui** |🟢 Dead-simple immediate-mode model; very fast to iterate; resizable panels in one call; small self-contained binary; ideal for tools/overlays. |🔴 Retained niceties are DIY — sticky header and the diff rows are hand-painted with `Painter`; achieving a polished "designed" look takes manual effort; no CSS/retained tree. | **8.0** |
| **slint** |🟢 Concise declarative `.slint` markup with live preview; clean UI/logic split; built-in `Palette.color-scheme`; designed embedded→desktop; reasonable footprint. |🔴 Models are flat (tree must be flattened in Rust); `Text` has no multi-colour runs (one `Text` per span); no splitter/tree widgets out of the box. | **8.0** |
| **iced** |🟢 Clean Elm architecture with very predictable state; `pane_grid` resizing; first-class theming; rich-text spans; pure-Rust & cross-platform. |🔴 No built-in tree/sticky widgets (hand-rolled); widget geometry isn't readable in `update`, so scroll-to is estimated; more boilerplate than declarative toolkits. | **7.5** |
| **dioxus** |🟢 React-like RSX + CSS but **all-Rust**; ergonomic signals; one component model across web/desktop/mobile; sticky/theme are plain CSS. |🔴 Desktop is a webview (webkit dep) → larger binary; transitive `libxdo`/tray deps need a headless workaround; 0.7 API churn. | **7.5** |
| **floem** |🟢 Fine-grained Leptos-style reactivity (minimal manual diffing); Taffy flex + virtualized lists; editor-grade (powers Lapce); self-contained. |🔴 Pre-1.0, smaller ecosystem; no sticky primitive; closure-based styling is verbose when threading a runtime palette; no tree widget. | **7.5** |
| **qt-cxx** |🟢 KDAB's modern, **safe CXX** bridge; declarative `#[qproperty]`/`#[qinvokable]`; full Qt Quick power; best Rust↔C++ interop for large/mixed codebases. |🔴 Heavy, finicky build (CXX codegen + Qt C++ compile; ~6-min LTO); `Pin`/`rust_mut` borrow ergonomics; large Qt 6 dep. | **7.0** |
| **qt-widgets** |🟢 Classic rock-solid QtWidgets (`QSplitter`, `QTreeWidget`, `QToolBar`, `QMainWindow`) — top toolbar/tree/splitters essentially free; extremely predictable; tiny app binary. |🔴 No maintained Qt 6 *Widgets* Rust binding → needs a C++ shim + C-ABI glue; rich text via HTML in `QLabel`/`QTextEdit`; large Qt 6 dep; not pure Rust. | **7.0** |
| **freya** |🟢 Clean builder API; `ResizableContainer` + tooltips built-in; `paragraph().span()` consumes syntect spans directly; crisp Skia text; Dioxus-style reactivity. |🔴 No sticky / scroll-into-view (pixel-offset approximations); young ecosystem; **statically bundles Skia** → very large binary & long build. | **7.0** |
| **gpui** |🟢 Built for a real editor (Zed): fast, tailwind-ish `div()` styling, virtualization, ergonomic entity/listener model. |🔴 Not really published/semver-stable (git dep); few batteries (no splitter/tooltip/sticky widgets — all hand-rolled); Vulkan-only on Linux (no headless capture here); large binary. | **6.5** |
| **makepad** |🟢 GPU/shader `live_design!` DSL with live styling; real `Splitter` + virtualized `PortalList`; very high performance ceiling; smallest of the GPU-rendered apps. |🔴 Steepest learning curve & sparse docs; anything bespoke needs custom drawing (a whole custom `DiffRow`); text didn't paint under software GL here; word-wrap unimplemented. | **5.5** |
| **xilem** |🟢 Linebender's next-gen reactive architecture (xilem_core + Masonry + Vello GPU); promising structure & performance; active development. |🔴 **Alpha**: sharp API edges (`Style`-vs-inherent method ordering, `Send+Sync` state ⇒ `Repo` behind a `Mutex`), thin widget set (no tree/sticky/scroll-to-child, single-style labels), Vulkan-only (flaky headless capture), large binary. | **5.5** |

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
