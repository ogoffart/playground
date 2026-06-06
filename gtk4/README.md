# git-review · GTK4

A GitHub-style git code-review desktop app, built with **gtk4-rs** (plain GTK4, no relm4).
One of several framework implementations of the shared [`SPEC.md`](../SPEC.md).

## Run

```sh
cargo run --release -- /path/to/repo
# or
GIT_REVIEW_REPO=/path/to/repo cargo run --release
# defaults to "." when no path is given
```

Generate a demo repo to point it at:

```sh
../tools/make-sample-repo.sh /tmp/git-review-sample
cargo run -- /tmp/git-review-sample
```

Requires system `gtk4` (>= 4.10) and its pkg-config files. (GtkSourceView was available but
not needed — see below.)

### Headless smoke test

```sh
export DISPLAY=:82 GDK_BACKEND=x11 GSK_RENDERER=cairo
Xvfb :82 -screen 0 1280x860x24 &
./target/debug/git-review-gtk4 /tmp/git-review-sample &
import -window root screenshot.png
```

## Data layer

`src/git.rs` and `src/highlight.rs` are copied **verbatim** from the egui reference app — they are
framework-agnostic (`git2` + `syntect`/`two-face`). Only `main.rs` and `app.rs` are GTK-specific.

## How GTK4 handled the tricky parts (for the comparison)

- **Layout — full-width toolbar over GtkPaned splits.** The window's child is a vertical `Box`:
  a single full-width toolbar (styled `Box` with a bottom border and inset toggle buttons) pinned
  at the very top, then an outer horizontal `Paned` (side panel | main) wrapping an inner vertical
  `Paned` (commit list | file tree). `GtkPaned` gives native draggable handles, `set_position`,
  and per-child `resize`/`shrink` flags — the cleanest split implementation of any framework here,
  no manual drag math.

- **Real file tree — GtkTreeListModel + GtkListView.** The flat `FileDiff` list is folded into a
  real hierarchy (`build_tree`) and exposed as `FileNode` GObjects through a `TreeListModel` driven
  by a `GtkListView` whose factory rows carry a `GtkTreeExpander` (disclosure triangle) + icon +
  name + `+a −r` counts. Directories expand by default; single-child directory *chains* are
  collapsed GitHub-style (`a/b` shown as one node) while distinct subtrees branch. Clicking a leaf
  scrolls the diff to that file; clicking a folder toggles its expansion (`connect_activate`).

- **System light/dark theme.** At startup `load_css` resolves the desktop scheme via GTK's
  `gtk-application-prefer-dark-theme` setting (which GTK populates from the freedesktop
  `color-scheme` portal / `prefers-color-scheme`) and locks the matching GitHub palette (light or
  dark). The CSS provider and all `TextTag` colours are generated from that palette, so chrome and
  diff colours both track the OS. Headless (no preference) defaults to light.

- **Diff rendering — TextView + TextTags, not a grid of widgets.** Each file's unified diff is a
  single `TextView`/`TextBuffer`. Full-width green/red line backgrounds use
  `TextTag::paragraph_background` (which tints the whole logical line, gutter included), syntax
  colours from the `Highlighter` spans become foreground `TextTag`s (cached per RGB/bold/italic),
  and the line-number/sign gutter is rendered as plain leading text in a monospace buffer. Font
  size (`A-`/`A+`) is a whole-buffer `set_size_points` tag; word-wrap flips
  `TextView::set_wrap_mode`. This keeps text selectable and fast.

- **Sticky headers — approximated with a GtkOverlay.** GTK4 has no built-in sticky/pinned section
  header for free-form content (GtkListView sections exist but didn't fit a mixed
  header+TextView layout). Instead each file header is a real widget anchored in the scrolling
  `Box`; a floating label in a `GtkOverlay` over the `ScrolledWindow` is updated on the
  vadjustment's `value-changed` signal to show whichever file currently owns the top of the
  viewport. Functionally sticky, though it's a separate floating bar rather than the header widget
  itself freezing in place.

- **Scroll-to-file — compute_bounds + vadjustment.** Clicking a file in the tree calls
  `widget.compute_bounds(&diff_box)` to get the header's Y offset within the scrolled child, then
  sets the `ScrolledWindow` vadjustment value (clamped to `upper - page_size`). Same mechanism
  drives the sticky-header tracking.

- **State & callbacks — `Rc<RefCell<State>>`.** Without an Elm-style runtime, shared mutable state
  lives in `Rc<RefCell<State>>` and widget handles in an `Rc<Ui>`; a single `refresh()` rebuilds
  the diff body, file tree, and commit-row highlight whenever the diff or a toolbar setting
  changes. Toggling "show space changes" recomputes the diff via `DiffOptions::ignore_whitespace`.

## Known approximations

- Sticky header is an overlay bar, not the in-flow header widget freezing (see above).
- Syntax highlighting is per-line/stateless (inherited from the shared `highlight.rs`), so
  multi-line constructs like block comments aren't carried across lines — identical to every other
  app in this repo.
