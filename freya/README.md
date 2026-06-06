# git-review · Freya

A GitHub-style git code-review desktop app, built with [Freya](https://freyaui.dev) — the
cross-platform Rust GUI library powered by Skia. This is the Freya entry in a multi-framework
comparison; it implements the shared contract in [`../SPEC.md`](../SPEC.md).

## Running

```sh
cargo run --release -- /path/to/some/git/repo
```

The repository to review is taken from, in order:

1. `argv[1]`
2. the `GIT_REVIEW_REPO` environment variable
3. the current working directory

A sample repo can be generated with `../tools/make-sample-repo.sh` (the screenshot uses
`/tmp/git-review-sample`).

> Skia is compiled from source by `freya-skia-bindings`, so the **first** build is slow
> (several minutes). Subsequent builds are fast.

## What it does

- **Full-width toolbar pinned at the very top** of the window, spanning above both the side panel
  and the diff. The root is a column — `[ toolbar ]` then `[ side | main ]`. The bar has a distinct
  panel background and bottom border with slightly inset icon buttons. It shows the
  `git show <sha>` / `git diff <from> <to>` summary with aggregate `+added −removed` counts, plus
  five tool buttons (word wrap, show space changes, `A-`, `A+`, line numbers), each with a hover
  **tooltip** and a pressed/active look for toggles.
- **Resizable left side panel** (commit list on top, file tree below) split from the **main diff
  view** — both the panel/main divider and the commits/files divider are draggable
  (`ResizableContainer` / `ResizablePanel`).
- **Commit list**: a synthetic "Uncommitted changes (working tree)" row first, then commits from
  `HEAD`. Each row is two lines — endpoint buttons `◀`/`▶`, short sha, date, author on line 1;
  the title on line 2 — with click-to-open and a highlight on the active row.
- **Real hierarchical file tree**: directory components become collapsible **folder nodes**
  (folder icon + name + `▾`/`▸` disclosure), indented by depth and expanded by default; common
  single-child directory chains are collapsed GitHub-style into one node. File **leaves** show a
  type icon, the file **name** (not the whole path), and `+added −removed` counts. Clicking a
  folder toggles its expansion (tracked in a `HashSet` signal); clicking a file scrolls the diff
  view toward that file's section.
- **Diff view** (scrollable): commit message block for a single commit, then per-file headers
  (icon, path, change kind, counts), a toggleable old/new line-number gutter, a `+`/`-`/` `
  change-marker column (stronger green/red), syntax-highlighted line text, and red/green line
  backgrounds.
- **System light/dark theme**: at startup the desktop colour scheme is detected with the
  [`dark-light`](https://crates.io/crates/dark-light) crate, and the matching **GitHub palette**
  (light or dark, both defined in `main.rs`) is selected. `GIT_REVIEW_THEME=dark|light` overrides
  detection; an unspecified scheme (e.g. headless) falls back to **light** per the spec. The
  syntect syntax theme tracks the scheme too (`InspiredGitHub` for light, `base16-ocean.dark` for
  dark).
- Honors all toolbar settings: word wrap, line numbers, font size, and ignore-whitespace.

The framework-agnostic model is reused verbatim from the reference implementation: `git.rs`
(libgit2 via `git2`) and `highlight.rs` (`syntect` + `two-face`). `model.rs` flattens a `DiffSet`
plus per-line highlighting into plain `Clone`-able structs that live in Freya `State`s; `main.rs`
renders them with Freya's builder API.

## Freya version & API note

Built against **`freya = "0.4.0-rc.22"`** (latest crates.io release at time of writing).

Freya **0.3** used a Dioxus **RSX macro**; the 0.4 release line replaced that with a **builder /
extension-trait API** — `rect()`, `label()`, `paragraph()`, `ScrollView::new()`,
`ResizableContainer::new()`, chained with `.width(Size::px(..))`, `.background((r,g,b))`,
`.on_press(..)`, `.child(..)`, etc. State is `use_state` / `use_memo` (Dioxus-signals-style,
`State<T>` is `Copy`). The app root implements the `App` trait; the engine (`Repo` + `Highlighter`)
is shared via `provide_context` / `consume_context`. This implementation matches the 0.4 builder
API — it does **not** use RSX.

## How Freya handled the tricky parts

- **Two resizable splits** — handled natively by `ResizableContainer` + `ResizablePanel`
  (horizontal for panel↔main, vertical for commits↔files). No manual mouse-drag bookkeeping was
  needed.
- **Syntax-highlighted lines** — syntect spans map cleanly to Freya `paragraph().span(Span::new(..)
  .color(..).font_weight(..).font_slant(..))`, one `paragraph` per diff line.
- **Tooltips** — `TooltipContainer::new(Tooltip::new(text)).child(button)` gives the toolbar
  buttons hover tooltips for free.
- **Right-aligned row content (the `+a −r` counts)** — the file-tree rows and per-file headers use
  a horizontal `rect` with `main_align(SpaceBetween)` and two natural-width groups (left content /
  right counts). An earlier attempt using a `Size::fill()` spacer child swallowed all the
  horizontal space and pushed the counts off-screen; `SpaceBetween` over two intrinsically-sized
  groups is the reliable idiom.

## Limitations / approximations

- **Sticky per-file headers** — Freya has no built-in CSS-`position: sticky` equivalent, so the
  per-file headers are ordinary headers that scroll with their section (visually distinct, with a
  panel background and border, but not pinned). This matches the SPEC's "approximate + note"
  fallback.
- **Click-to-scroll to a file** — Freya has no `scrollIntoView`-by-id. The file tree scrolls the
  diff view via a `ScrollController` to an **estimated** pixel offset accumulated from a fixed
  per-line height, so it lands *near* (not pixel-exact on) the target file's section.
- **Horizontal scrolling of long lines** — when word-wrap is **off**, each file's code body sits in
  its own horizontal `ScrollView` so long lines pan sideways while the file header and counts stay
  pinned to the viewport. When word-wrap is **on**, lines fill the viewport width and wrap.
- **Per-line highlighting** is stateless (a fresh highlighter per line), so constructs spanning
  multiple lines (e.g. block comments) aren't carried across — an intentional, shared
  simplification across every app in this comparison.

## Headless render / screenshot

`./screenshot.png` was captured running the binary against `/tmp/git-review-sample` under a
headless X server:

```sh
Xvfb :93 -screen 0 1280x800x24 &
DISPLAY=:93 LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe \
    VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
    ./target/release/git-review-freya /tmp/git-review-sample &
DISPLAY=:93 import -window root screenshot.png
```

Freya's Skia/winit renderer runs fine headless with the Mesa software GL / lavapipe Vulkan stack —
no GPU is required. The default window is exactly **1280×800** (matching the Xvfb screen, so the
capture is border-free) and renders the full UI. With no desktop colour-scheme preference the app
defaults to the **light** palette.

## Release build (size-optimised)

`[profile.release]` is tuned for a small binary — `opt-level = "z"`, `lto = true`,
`codegen-units = 1`, `panic = "abort"`, `strip = true`. The Skia bindings are compiled from source,
so the first release build is slow; the result is a single self-contained executable.
