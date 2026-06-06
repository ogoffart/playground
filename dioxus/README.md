# git-review · Dioxus

A GitHub-style git code-review desktop app, built with **Dioxus 0.7** (desktop / `wry` webview).
Part of a multi-framework comparison; the behaviour and visuals follow the shared
[`../SPEC.md`](../SPEC.md). The git/diff model (`src/git.rs`) and syntax highlighter
(`src/highlight.rs`) are copied verbatim from the egui reference so the apps stay apples-to-apples.

## Run

```sh
cd dioxus
cargo build
./target/debug/git-review-dioxus /path/to/repo
```

The repository path comes from `argv[1]`, else `$GIT_REVIEW_REPO`, else the current directory.
Generate the demo repo with `../tools/make-sample-repo.sh` and point the app at
`/tmp/git-review-sample`.

A plain `cargo build` + `cargo run` works — the dioxus-CLI is **not** required (`main` calls
`LaunchBuilder::desktop()...launch`).

### Headless smoke test

```sh
export DISPLAY=:84 WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1 LIBGL_ALWAYS_SOFTWARE=1
Xvfb :84 -screen 0 1280x860x24 >/dev/null 2>&1 &
./target/debug/git-review-dioxus /tmp/git-review-sample &
sleep 9 && import -window root screenshot.png
```

See `screenshot.png` for the rendered result.

## How Dioxus handled the tricky parts (for the comparison)

- **State = signals, work funnelled through one effect.** UI state (selection, `from`/`to`
  endpoints, settings, panel sizes) lives in `use_signal`. A single `use_effect` watches the
  selection + whitespace flag and rebuilds a plain, `Clone + PartialEq` `RenderDiff` (`src/model.rs`)
  off the `git2`/`syntect` engine. Font-size, word-wrap and line-numbers are pure CSS toggles, so
  they re-render instantly without recomputing the diff.

- **Non-`Clone` engine via context + `Rc`.** `git2::Repository` and the `syntect` highlighter are
  neither `Clone` nor cheap to rebuild, so they are constructed once, wrapped in `Rc<Engine>`, and
  handed to the tree through `LaunchBuilder::with_context`. Desktop is single-threaded (the wry
  event loop), so a thin `unsafe Send/Sync` newtype satisfies the bound without ever crossing
  threads.

- **Sticky headers and red/green diffs are free CSS.** The whole visual contract maps cleanly onto
  HTML/CSS in the webview: `position: sticky` gives per-file pinned headers, flexbox lays out the
  gutter/sign/code columns, and added/removed line backgrounds are just classes. Syntect spans
  become `<span style="color:rgb(...)">`. This is where the webview approach shines versus immediate
  mode.

- **Resizable splits = draggable dividers + a drag overlay.** `onmousedown` on a divider sets a
  `drag` signal; a full-window transparent overlay then captures `onmousemove`/`onmouseup` (so the
  drag survives the cursor leaving the 5px handle) and writes pixel sizes into CSS custom properties
  (`--side-w`, `--commits-h`). Mouse coordinates come from `event.client_coordinates()`.

- **Scroll-to-file via `document::eval`.** Clicking a file in the tree runs a one-line JS
  `scrollIntoView` against the file's `id`, which is simpler than threading scroll offsets through
  Rust state.

## Limitations / notes

- **libxdo stub.** `dioxus-desktop` transitively links `muda`/`tray-icon`/`global-hotkey`, which
  link `libxdo` unconditionally even though this app uses no tray, global hotkeys, or menu
  accelerators (the menubar is disabled with `Config::with_menu(None)`). The real libxdo is not
  installed in this environment, so `build.rs` compiles a tiny no-op `stubs/xdo_stub.c` into a
  `libxdo.so` and adds it to the link search path + rpath. None of those symbols are ever called at
  runtime. On a machine with `libxdo-dev` installed, the stub is harmless (or the `build.rs` can be
  dropped).

- **Per-line highlighting.** Like every app in this repo, syntect runs statelessly per line, so
  multi-line constructs (e.g. block comments) are not carried across lines — an accepted
  approximation for a diff viewer.

- **Sticky header overlap.** With many small files the only sticky header pinned at any moment is the
  current file's own (native CSS sticky), which matches GitHub's behaviour; there is no separate
  global overlay header.
