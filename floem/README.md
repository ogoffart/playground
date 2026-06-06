# git-review · Floem

A GitHub-style code-review desktop app built with [Floem](https://github.com/lapce/floem),
a fine-grained reactive Rust UI library. It implements the shared
[`SPEC.md`](../SPEC.md): a resizable side panel (commit list over file tree), a toolbar with
the summary and five tool buttons, and a scrollable, syntax-highlighted unified diff.

## Running

```sh
cargo run --release -- /path/to/git/repo
```

The repository path is taken from `argv[1]`, then `$GIT_REVIEW_REPO`, then the current
directory. A demo repo can be generated with `tools/make-sample-repo.sh` (see the workspace
root); point the app at it:

```sh
cargo run -- /tmp/git-review-sample
```

### Headless / CI

The app renders correctly under a virtual X server (wgpu falls back to the software/llvmpipe
adapter):

```sh
export DISPLAY=:85 LIBGL_ALWAYS_SOFTWARE=1
Xvfb :85 -screen 0 1280x860x24 &
./target/debug/git-review-floem /tmp/git-review-sample
```

A reference screenshot captured this way lives at [`screenshot.png`](screenshot.png).

## How Floem handled the tricky parts

- **Reactive state, rebuilt views.** All app state is held in `RwSignal`s (`src/ui.rs`,
  `AppState`). The view tree is constructed once; the dynamic regions (diff body, commit
  list, file tree) are wrapped in `dyn_container` / `dyn_stack` keyed on the signals that
  drive them, so toggling a setting or selecting a commit re-derives only that subtree.

- **Syntax highlighting via `rich_text` + `TextLayout`.** syntect spans are turned into a
  Floem `TextLayout` with per-range `Attrs` (color / bold / italic), giving multi-colored
  monospace runs inside each diff line without a custom view.

- **Resizable panels by hand.** Floem 0.2 has no built-in splitter, so the side-panel width
  and commit-list height are plain signals driven by thin drag-handle views
  (`h_splitter` / `v_splitter`) that track `PointerDown/Move/Up` and clamp the size.

- **Scroll-to-file via `ViewId`.** Each rendered file section is converted to a concrete view
  to capture its `ViewId`; those ids are published into a signal, and `Scroll::scroll_to_view`
  jumps the diff to the clicked file in the tree.

- **Diff styling with flex rows.** Each diff line is an `h_stack` of a fixed-width gutter
  (old/new line numbers + a colored `+`/`-`/` ` marker cell) and a flexible code cell, with
  the whole row tinted green/red and the marker cell a stronger shade.

## Known limitations / approximations

- **Sticky file headers are not pinned.** Floem 0.2 has no sticky-position primitive, so the
  per-file headers scroll with their content rather than staying pinned at the top of the
  viewport. The header styling (panel background, bordered, path + counts) matches the spec
  otherwise.
- **Word-wrap vs. horizontal scroll** is approximated through flex sizing (`flex_grow` when
  wrapping, natural/`min_width_full` width otherwise) rather than a dedicated wrap mode.
- A harmless `XDG_RUNTIME_DIR is invalid or not set` warning may print under bare Xvfb; it
  does not affect rendering.

## Tested

`cargo build` succeeds with no warnings, and the app renders the full UI (verified via the
headless Xvfb screenshot above) — commit list, file tree, toolbar, and a syntax-highlighted
red/green diff with gutter line numbers.
