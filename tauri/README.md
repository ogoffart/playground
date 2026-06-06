# git-review — Tauri

A GitHub-style git code-review desktop app, built with **Tauri 2** (Rust backend +
plain HTML/CSS/JS frontend, no JS framework, no bundler). Part of the multi-framework
comparison in this repo; it implements the shared contract in [`../SPEC.md`](../SPEC.md).

## Architecture

- **Backend** (`src-tauri/src/`): `git.rs` and `highlight.rs` are copied verbatim from the
  egui reference app (the framework-agnostic git/diff/syntect model). `lib.rs` wraps that
  model in serde-serializable JSON structs and exposes three `#[tauri::command]`s:
  - `repo_name() -> String`
  - `commits() -> Vec<CommitJson>`
  - `diff(mode, a, b, ignoreWs) -> DiffJson` — `mode` is `"working"`, `"commit"` (uses `a`),
    or `"range"` (uses `a` + `b`). Syntax highlighting is done in Rust (syntect spans →
    `{text, color, bold, italic}` arrays) so the frontend is a dumb renderer, consistent
    with the other native apps.
- **Frontend** (`ui/`): hand-written `index.html` + `style.css` + `app.js`. Tauri serves it
  via `frontendDist: "../ui"`. `withGlobalTauri: true` exposes `window.__TAURI__.core.invoke`
  so no import/build step is needed.
- The repo path comes from `argv[1]`, then `$GIT_REVIEW_REPO`, then the cwd — resolved in the
  Rust backend at startup.

## Build & run

No Tauri CLI or Node build step is required — it builds with plain Cargo:

```sh
cd src-tauri
cargo build                # or: cargo build --release
./target/debug/git-review-tauri /path/to/some/git/repo
```

If `argv[1]` is omitted it falls back to `$GIT_REVIEW_REPO` or the current directory.

### Headless smoke test (Xvfb + WebKitGTK)

```sh
export DISPLAY=:83 WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1 LIBGL_ALWAYS_SOFTWARE=1
Xvfb :83 -screen 0 1280x860x24 >/dev/null 2>&1 &
./src-tauri/target/debug/git-review-tauri /tmp/git-review-sample &
sleep 9
import -window root screenshot.png
```

The WebKit env vars above are needed for software rendering under Xvfb; without them the
WebView paints a blank/white window. See `screenshot.png` for a captured run.

## How Tauri handled the tricky parts (for the comparison)

- **Sticky per-file headers** — a single CSS line: `position: sticky; top: 0` on
  `.file-header`. No manual scroll math (the egui app has to overlay a floating area and track
  header offsets by hand). This is the case where the web stack is dramatically simpler.
- **IPC commands** — `#[tauri::command]` + `tauri::generate_handler!` on the Rust side,
  `invoke("diff", { mode, a, b, ignoreWs })` on the JS side. Args are auto-serialized;
  note Tauri converts the JS camelCase `ignoreWs` to the Rust snake_case `ignore_ws`.
  Shared `git2` state lives in a `Mutex<Repo>` behind `tauri::State`.
- **Resizable dividers** — a few lines of `mousedown`/`mousemove`/`mouseup` JS adjusting a
  flex `width`/`height`. Two splits: the side-panel width (horizontal) and the
  commits/files split (vertical), both clamped to sane min/max.
- **Diff backgrounds & gutter** — pure CSS. Each row is a flex line of `[old#][new#][sign][code]`;
  `.row.added`/`.row.removed` set the green/red line background and a stronger marker color on
  the sign column. Word-wrap, line-numbers, and font-size are CSS toggles (`white-space`,
  display of `.gutter-num`, a `--font-size` custom property) so they re-render instantly.
- **Highlighting** — done in Rust with syntect (per-line, stateless), shipped to the frontend
  as colored spans, identical to the egui/dioxus apps. The whitespace ("show space changes")
  toggle maps to `DiffOptions::ignore_whitespace` and re-queries the backend; the other four
  toolbar settings are frontend-only and need no round trip.

## Notes / approximations

- Highlighting is per-line and stateless (same trade-off as the other apps): constructs that
  span lines, e.g. block comments, aren't carried across lines. Acceptable for a diff viewer.
- The sticky header shows the file currently scrolled under it; because each file is its own
  scroll-sticky container, headers stack naturally at the top with no JS — slightly different
  from a single global sticky bar but visually equivalent to GitHub.
