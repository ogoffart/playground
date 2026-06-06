# `git-review` — shared feature & visual specification

This document is the **contract** that every framework implementation in this repo follows, so
the apps can be compared apples-to-apples. Each app lives in its own folder, is a standalone
project, and re-implements the same behaviour using its framework's idioms.

The app is a desktop tool for **reviewing code in git**, with a look familiar to anyone who uses
GitHub's "Files changed" view.

## Window layout

```
┌──────────────┬──────────────────────────────────────────────┐
│ commit list  │  toolbar:  git show a1b2c3   +120 −34   [⤶][␣][A-][A+][#] │
│  (resizable) ├──────────────────────────────────────────────┤
│              │                                              │
├──────────────┤   diff view (scrollable)                     │
│ file tree    │   ┌── src/main.rs   +12 −3 ───────────(sticky)│
│  (resizable) │   │  1  1   fn main() {                       │
│              │   │     2 +     println!("hi");               │
│              │   │  2      -   todo!();                       │
└──────────────┴──────────────────────────────────────────────┘
       ▲ the whole left side panel is horizontally resizable
```

- A left **side panel**, horizontally **resizable** against the main view.
- The side panel is split **vertically** into two **resizable** parts:
  - **top:** the commit list,
  - **bottom:** the file tree of the current diff.
- The **main view** is the diff, with a toolbar pinned at its top and a scrollable body.

## Toolbar (top of the main view)

- **Left — summary** of what is being shown:
  - single commit: `git show <short-sha>`
  - range: `git diff <short-from> <short-to>`
  - followed by aggregate counts: `+<added> −<removed>` (green / red).
- **Right — icon tool buttons** (icon + tooltip, toggles look pressed when active):
  1. **word wrap** — wrap long diff lines vs. horizontal scroll.
  2. **show space changes** — when off, whitespace-only differences are ignored.
  3. **decrease font** `A-`
  4. **increase font** `A+`
  5. **line numbers** — show/hide the old/new line-number gutter.

## Commit list (top of side panel)

A vertical list. The **first row is a synthetic placeholder** for the working tree
("Uncommitted changes"). Then every commit reachable from `HEAD`, newest first.

Each row is **two lines**:

- **line 1:** two small endpoint buttons **[ from ]** **[ to ]**, the **short sha**, the
  **date**, and the **author name** (ellipsized to fit).
- **line 2:** the commit **title** (first line of the message), ellipsized.

Behaviour:

- Clicking a row **opens that commit** (`git show`) in the main view.
- The **from** / **to** endpoint buttons select the two ends of a comparison; when both are set
  the main view shows `git diff from..to`.
- The row currently shown is **highlighted**.

## File tree (bottom of side panel)

The files present in the current diff, as a tree (directories collapsible). Each file row shows:

- a **file-type icon** (by extension),
- the **path / name**,
- the per-file **`+added −removed`** counts.

Clicking a file **scrolls the diff view** to that file's section.

## Diff view (main, scrollable)

- If a **single commit** is shown, the body **starts with the commit message** (full message,
  author, date, sha).
- Then, **for each file**:
  - a **sticky header** (stays pinned at the top while that file scrolls) showing the file path
    and its `+added −removed` counts;
  - the **unified diff**: a left **gutter** with old/new line numbers (toggleable) and a
    `+` / `-` / ` ` change marker, then the line text with **syntax highlighting**;
  - added lines have a **green** background, removed lines a **red** background, context lines
    none. The change marker column is a stronger green/red.
- Honors the toolbar settings: word-wrap, line numbers, font size, ignore-whitespace.

## Behavioural data model (names kept parallel across apps)

- `CommitInfo { oid, short_sha, author, date, title, is_working_tree }`
- `Endpoints { from: Option<Oid>, to: Option<Oid>, single: Option<Oid> }`
- `FileDiff { path, old_path, kind: Added|Deleted|Modified|Renamed, added, removed, hunks }`
- `Hunk { header, lines: Vec<DiffLine> }`
- `DiffLine { kind: Context|Added|Removed, old_no: Option<u32>, new_no: Option<u32>, spans }`
  where `spans: Vec<(Style, String)>` come from syntax highlighting.
- `Settings { word_wrap, show_space_changes, font_size, line_numbers }`

## Data sources (native apps)

- **git:** `git2` (libgit2). Commit list from a `Revwalk` over `HEAD`. A single commit's diff is
  `parent_tree → commit_tree`; a range is `from_tree → to_tree`; the working-tree row is
  `head_tree → workdir`. Counts from the diff line callbacks / `Diff::stats`. "Show space
  changes" off maps to `DiffOptions::ignore_whitespace(true)`.
- **highlighting:** `syntect` (+ `two-face`) chooses a syntax by file extension and yields styled
  spans per line. Web-backed apps (tauri, dioxus) highlight in the Rust backend.

## Repo to open

Each app takes the repository path as **argv[1]** (or `$GIT_REVIEW_REPO`), defaulting to the
current directory. Use `tools/make-sample-repo.sh` to generate a demo repo with varied file
types and changes to point the apps at.

## Visual style

Modern desktop, GitHub-familiar. Light theme by default:

- background `#ffffff`, panels `#f6f8fa`, borders `#d0d7de`, text `#1f2328`, muted `#656d76`,
  accent/links `#0969da`.
- diff added bg `#e6ffec` / marker `#abf2bc`; removed bg `#ffebe9` / marker `#ff8182`.
- monospace for diff (`ui-monospace`, system mono), system UI font for chrome.
