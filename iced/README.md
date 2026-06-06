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

The window opens as `git-review · iced` (default 1280×800): a full-width toolbar pinned at
the top, and below it a resizable side panel (commit list over a hierarchical file tree)
next to the main scrollable diff view.

## How iced handled the tricky parts

- **Full-width toolbar on top** — the root view is `column![toolbar, rest]`, where `rest`
  is the `PaneGrid`. The toolbar is a single full-width `container` with a panel background
  and a bottom border, holding inset (bordered, slightly padded) `button`s on the right and
  the `git show … +a −r` summary on the left, so it reads as a real toolbar above both the
  side panel and the diff.

- **Resizable split** — `pane_grid` is a perfect fit. The split below the toolbar is one
  `PaneGrid` built from a nested `Configuration`: a vertical split (side | main) whose left
  side is itself a horizontal split (commits / files). Dragging either divider emits
  `pane_grid::ResizeEvent`, and `state.resize(split, ratio)` in `update` persists it. No
  manual hit-testing or drag math needed.

- **Hierarchical file tree** — iced has no tree widget, so the flat `FileDiff.path` list is
  folded into a nested `TreeNode` structure (folders + file leaves), single-child directory
  chains are collapsed GitHub-style (`a/b/c`), and the tree is rendered recursively into a
  flat `Vec<Element>` (each row indented by `depth`) inside a `scrollable`. Folders carry a
  stable key; clicking one toggles a `collapsed: HashSet<String>` in the model, files emit
  `OpenFile(idx)` to scroll the diff. Counts (`+a −r`) appear on each leaf.

- **System light/dark theme** — at startup `dark_light::detect()` chooses the GitHub light
  or dark palette (a `Palette` struct of the exact spec hex values), the iced base `Theme`
  is set to match, and the `syntect` theme tracks it (`InspiredGitHub` vs
  `base16-ocean.dark`). Headless (no desktop preference) falls back to light.

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

- **GitHub palette** — explicit per-widget `container` / `button` style closures use the
  exact hex values from the spec for the active scheme (light: added `#e6ffec`, removed
  `#ffebe9`, accent `#0969da`; dark: added `#12261e`, removed `#25171c`, accent `#2f81f7`,
  etc.). Tooltips on the toolbar buttons use iced's `tooltip` widget.

## Limitations / notes

- The sticky header and scroll-to offsets are layout *estimates*, not measured geometry
  (iced does not expose per-widget measured positions to the update loop), so they assume a
  uniform monospace row height. Good enough for review, but not pixel-exact on wrapped lines.
- Highlighting is per-line and stateless (shared decision across all apps), so multi-line
  constructs such as block comments are not carried across lines.
