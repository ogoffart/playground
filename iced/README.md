# git-review · iced

A GitHub-style git code-review desktop app built with [iced](https://iced.rs) 0.14.
This is the iced entry in the multi-framework comparison; it shares the framework-agnostic
data layer (`src/git.rs`, `src/highlight.rs`) verbatim with the other apps and only
re-implements the UI using iced's Elm architecture (`Message` / `update` / `view`).

## Run

```sh
cargo run --release -- /path/to/repo
```

The repository path comes from `argv[1]`, then `$GIT_REVIEW_REPO`, defaulting to `.`.

```sh
# point it at the generated sample repo
cargo run -- /tmp/git-review-sample
```

The window opens as `git-review · iced`: a resizable side panel (commit list over file
tree) and a main view (toolbar + scrollable diff).

## How iced handled the tricky parts

- **Resizable split** — `pane_grid` is a perfect fit. The whole layout is one `PaneGrid`
  built from a nested `Configuration`: a vertical split (side | main) whose left side is
  itself a horizontal split (commits / files). Dragging either divider emits
  `pane_grid::ResizeEvent`, and `state.resize(split, ratio)` in `update` persists it. No
  manual hit-testing or drag math needed.

- **Rich-text syntax highlighting** — `syntect` yields coloured spans per line; each becomes
  an `iced::widget::span(text).font(MONO).size(fs).color(...)`, collected into a
  `rich_text([...])`. iced lays out the multi-colour line in one text widget, and switching
  the span `wrapping` between `Word` and `None` implements the word-wrap toggle for free.

- **Scroll-to-file** — the diff body lives in a `scrollable` with a stable `Id`. Clicking a
  file dispatches `operation::scroll_to(id, AbsoluteOffset { y })`, which produces a `Task`
  that the runtime applies to that scrollable. Target offsets come from a `file_tops` map we
  compute by mirroring the row layout math (font size × line height, header/gap heights).

- **Sticky header (approximated)** — iced 0.14 has no native sticky/position-pinned widget.
  We approximate it: the `scrollable`'s `on_scroll` reports the current `Viewport` offset,
  we use it (against the same `file_tops` estimates) to find the topmost visible file, and
  render a copy of that file's header in a `stack![scroll, header]` overlay pinned to the top
  of the viewport with a small drop shadow. Because it derives from estimated row heights it
  can drift by a row on very long files, but it tracks the right file in normal scrolling.

- **GitHub-light palette** — built on `Theme::Light` with explicit per-widget `container` /
  `button` style closures using the exact hex values from the spec (added `#e6ffec`, removed
  `#ffebe9`, markers `#abf2bc` / `#ff8182`, accent `#0969da`, etc.). Tooltips on the toolbar
  buttons use iced's `tooltip` widget.

## Limitations / notes

- The sticky header and scroll-to offsets are layout *estimates*, not measured geometry
  (iced does not expose per-widget measured positions to the update loop), so they assume a
  uniform monospace row height. Good enough for review, but not pixel-exact on wrapped lines.
- Highlighting is per-line and stateless (shared decision across all apps), so multi-line
  constructs such as block comments are not carried across lines.
