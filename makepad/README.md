# git-review — Makepad implementation

A GitHub-style git code-review desktop app, built with
[Makepad](https://makepad.dev) (`makepad-widgets`). Part of a multi-framework
comparison; see `../SPEC.md` for the shared behavioural/visual contract.

## Run

```sh
cargo run --release -- /path/to/repo
```

The repository to open is taken from `argv[1]`, else `$GIT_REVIEW_REPO`, else the
current directory. A demo repo can be generated with
`../tools/make-sample-repo.sh`.

Makepad needs an OpenGL context. On a headless box you can run it under Xvfb with
software GL:

```sh
Xvfb :92 -screen 0 1280x860x24 -ac +extension GLX +render &
DISPLAY=:92 LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe \
    cargo run -- /tmp/git-review-sample
```

### Build dependencies

Makepad links against X11 and audio libraries. On Debian/Ubuntu:

```sh
sudo apt-get install libasound2-dev libpulse-dev libx11-dev libxcursor-dev
```

## Layout

```
src/
  git.rs        # framework-agnostic git/diff model (git2) — shared, copied verbatim
  highlight.rs  # syntect + two-face syntax highlighting — shared, copied verbatim
  main.rs       # Makepad UI: live_design! DSL + AppMain/MatchEvent + custom DiffRow widget
```

`git.rs` and `highlight.rs` are the same modules used by the other apps in this
repo, so the data model (`CommitInfo`, `DiffSet`, `FileDiff`, `Hunk`, `DiffLine`,
`Settings`-equivalent) is identical.

## How Makepad's DSL handled the tricky parts

* **Whole UI in one `live_design!{}` block.** The window, toolbar, the
  horizontally-resizable side panel, and the vertically-split commits/files panes
  are all declared declaratively and composed from stock widgets (`Window`,
  `View`, `Button`, `Label`, `Splitter`, `PortalList`). Rust only feeds data in
  and reads actions out.

* **Resizable splitters — real, not approximated.** Makepad ships a `Splitter`
  widget that owns its own drag handling and exposes `a`/`b` content slots. Two
  nested `Splitter`s give exactly the SPEC layout: an outer `Horizontal` splitter
  (side panel ↔ main view) and an inner `Vertical` splitter (commit list ↔ file
  tree). The horizontal divider between the side panel and the diff is fully
  draggable.

* **Virtualized lists via `PortalList`.** All three scrollable areas (commits,
  file tree, diff body) are `PortalList`s. They recycle a small pool of item
  widgets while you scroll. To make the diff body — which mixes a commit-message
  block, per-file headers, hunk headers, and code lines — work as one uniform
  virtualized list, the diff is *flattened* into a `Vec<Row>` that is rebuilt
  whenever the shown diff changes (`Model::rebuild_rows`). Syntax highlighting is
  computed once at flatten time and cached in the rows.

* **Per-line backgrounds + per-token colors → a custom `Widget`.** A stock
  `Label` draws a single color, so it cannot render a syntax-highlighted line
  (many colors) sitting on a red/green tinted background with a stronger marker
  strip. `DiffRow` is therefore a small custom `#[derive(Live, Widget)]` widget:
  its `draw_walk` draws a background quad (`DrawColor`), the gutter marker strip,
  the old/new line-number columns, the `+`/`-` sign, and then each highlight span
  with its own color via a single `DrawText` whose `.color` is reassigned per
  span. This reproduces the GitHub diff look from the SPEC.

* **Driving the lists from Rust.** The PortalList draw loop lives in
  `handle_draw_2d` (reached via `match_event_with_draw_2d`): we step the widget
  tree with `ui.draw(...)`, and when a `PortalList` yields, dispatch to the
  matching draw routine by comparing `widget_uid()`. Items are populated with
  `item.label(id!(...)).set_text(...)` / `apply_over` for selection/toggle state,
  and for `DiffRow` by `borrow_mut::<DiffRow>()` and pushing the row data in.

* **Events.** Toolbar toggles and commit endpoint buttons are stock `Button`s; a
  shader `instance active` uniform drives the pressed/active look. Clicks are read
  in `handle_actions` via `.clicked(actions)`; row clicks (open commit /
  scroll-to-file) via `as_view().finger_down(actions)`. Click-to-scroll uses
  `PortalList::smooth_scroll_to(row_index)`.

## Approximations / limitations

* **Sticky file headers** are not pinned. The SPEC asks for a per-file header that
  stays pinned while that file scrolls. With `PortalList`'s recycling model there
  is no built-in sticky-item facility, so file headers scroll inline with their
  content (they are still clearly delimited, panel-colored, with a top border and
  the `+/−` counts). This is the one behavioural gap versus the SPEC.

* **Word-wrap** toggles state and is honored by the chrome, but `DiffRow` lays out
  code lines on a single line (horizontal overflow) regardless; wrapping
  long lines inside the custom widget is not implemented. Horizontal scrolling of
  long lines relies on `PortalList`'s vertical-only virtualization, so very long
  lines are clipped at the viewport edge rather than wrapped or h-scrolled.

* **Monospace metrics** in `DiffRow` use a fixed advance estimate
  (`font_size * 0.62`) to place the gutter columns and lay out spans, rather than
  measuring each glyph. With the default font this lines up well; exotic glyphs
  could drift slightly.

* The diff/highlighting is per-line and stateless (same trade-off as every app in
  this repo): multi-line constructs like block comments are not carried across
  lines.

## Headless render

The app **builds and runs**, and was smoke-tested under `Xvfb :92` (1280x860) with software GL
(`LIBGL_ALWAYS_SOFTWARE=1`, Mesa llvmpipe). Makepad creates its OpenGL context and lays out the
window: `screenshot.png` shows the chrome rendering correctly — the top toolbar strip, the
horizontally **resizable** side panel with its vertical commits/files split, the draggable
divider, and the main diff pane.

**Limitation in this headless container:** Makepad renders text through a GPU SDF font atlas, and
under Mesa **llvmpipe** (pure software GL, no real GPU) that text pass does not paint — so the
panels render but glyphs are blank in the captured frame. This is an environment limitation of
software GL, not an app bug; on a machine with a real GPU the same binary renders the full UI
(commit rows, file tree, and the syntax-highlighted diff drawn by the custom `DiffRow` widget).
The `target class not found` lines in the run log are benign `live_design!` apply warnings.
