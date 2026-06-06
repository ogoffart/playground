//! The Dioxus UI for git-review.
//!
//! One root component (`app`) wiring together: a resizable left side panel (commit list on top,
//! file tree on the bottom, each draggable), a toolbar, and the scrollable diff body. All UI state
//! lives in Dioxus signals; the heavy `git2`/`syntect` work is funnelled through `recompute`, which
//! rebuilds a plain `RenderDiff` whenever the selection or settings change.

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::rc::Rc;

use dioxus::prelude::*;
use git2::Oid;

use crate::git::CommitInfo;
use crate::model::{file_icon, RenderDiff};
use crate::{Engine, EngineCtx};

const CSS: &str = include_str!("style.css");

/// What the main view is currently displaying.
#[derive(Clone, Copy, PartialEq)]
enum Showing {
    Working,
    Commit(Oid),
    Range(Oid, Oid),
}

#[derive(Clone, Copy, PartialEq)]
struct Settings {
    word_wrap: bool,
    show_space: bool,
    font_size: f32,
    line_numbers: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            word_wrap: false,
            show_space: true,
            font_size: 12.0,
            line_numbers: true,
        }
    }
}

/// A `Clone + PartialEq` commit row for the list (the raw `CommitInfo` is neither).
#[derive(Clone, PartialEq)]
struct Row {
    oid: Option<Oid>,
    short: String,
    author: String,
    date: String,
    title: String,
}

impl From<&CommitInfo> for Row {
    fn from(c: &CommitInfo) -> Self {
        Row {
            oid: c.oid,
            short: c.short.clone(),
            author: c.author.clone(),
            date: c.date.clone(),
            title: c.title.clone(),
        }
    }
}

// ===== file tree model =====

/// A node in the hierarchical file tree built from the diff's file paths.
#[derive(Clone, PartialEq)]
enum TreeNode {
    /// A directory; `name` is the (possibly chain-collapsed, e.g. `a/b`) label, `path` is the full
    /// directory path used as the toggle key.
    Dir {
        name: String,
        path: String,
        depth: usize,
        children: Vec<TreeNode>,
    },
    /// A file leaf; `idx` indexes into `RenderDiff::files` (for scroll-to).
    File {
        name: String,
        idx: usize,
        icon: &'static str,
        added: u32,
        removed: u32,
        depth: usize,
    },
}

/// Intermediate mutable tree node used while grouping paths.
struct Builder {
    dirs: BTreeMap<String, Builder>,
    files: Vec<(String, usize, u32, u32)>, // (name, idx, added, removed)
}

impl Builder {
    fn new() -> Self {
        Builder {
            dirs: BTreeMap::new(),
            files: Vec::new(),
        }
    }

    fn insert(&mut self, parts: &[&str], idx: usize, added: u32, removed: u32) {
        match parts {
            [name] => self.files.push((name.to_string(), idx, added, removed)),
            [head, rest @ ..] => {
                self.dirs
                    .entry(head.to_string())
                    .or_insert_with(Builder::new)
                    .insert(rest, idx, added, removed);
            }
            [] => {}
        }
    }

    /// Turn this builder into render `TreeNode`s, collapsing single-child directory chains
    /// GitHub-style (`a/b/c` when each level has exactly one dir child and no files).
    fn finish(self, prefix: &str, name: String, depth: usize) -> Vec<TreeNode> {
        // Build dir children first, applying chain collapse.
        let mut nodes = Vec::new();
        for (dname, child) in self.dirs {
            let full = if prefix.is_empty() {
                dname.clone()
            } else {
                format!("{prefix}/{dname}")
            };
            nodes.extend(collapse_dir(dname, child, &full, depth));
        }
        // Then files at this level, sorted by name.
        let mut files = self.files;
        files.sort_by(|a, b| a.0.cmp(&b.0));
        for (fname, idx, added, removed) in files {
            nodes.push(TreeNode::File {
                icon: file_icon(&fname),
                name: fname,
                idx,
                added,
                removed,
                depth,
            });
        }
        let _ = name;
        nodes
    }
}

/// Produce a single Dir node for `dname` (full path `full`), collapsing chains.
fn collapse_dir(mut dname: String, mut child: Builder, full: &str, depth: usize) -> Vec<TreeNode> {
    let mut full = full.to_string();
    // While this dir has exactly one subdir and no files, fold it into the label.
    while child.files.is_empty() && child.dirs.len() == 1 {
        let (sub_name, sub_child) = child.dirs.into_iter().next().unwrap();
        dname = format!("{dname}/{sub_name}");
        full = format!("{full}/{sub_name}");
        child = sub_child;
    }
    let children = child.finish(&full, dname.clone(), depth + 1);
    vec![TreeNode::Dir {
        name: dname,
        path: full,
        depth,
        children,
    }]
}

fn build_tree(files: &[crate::model::RFile]) -> Vec<TreeNode> {
    let mut root = Builder::new();
    for (idx, f) in files.iter().enumerate() {
        let parts: Vec<&str> = f.path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            continue;
        }
        root.insert(&parts, idx, f.added, f.removed);
    }
    root.finish("", String::new(), 0)
}

fn recompute(engine: &Engine, showing: Showing, settings: Settings) -> RenderDiff {
    let ignore_ws = !settings.show_space;
    let res = match showing {
        Showing::Working => engine.repo.working_tree(ignore_ws),
        Showing::Commit(oid) => engine.repo.show(oid, ignore_ws),
        Showing::Range(a, b) => engine.repo.range(a, b, ignore_ws),
    };
    match res {
        Ok(set) => RenderDiff::build(set, &engine.hl),
        Err(e) => RenderDiff::error(e.message().to_string()),
    }
}

pub fn app() -> Element {
    let engine: Rc<Engine> = use_context::<EngineCtx>().0;

    // Commit list (built once).
    let rows: Vec<Row> = use_hook(|| {
        engine
            .repo
            .commits(500)
            .unwrap_or_default()
            .iter()
            .map(Row::from)
            .collect()
    });

    // Selection / settings / panel geometry state.
    let initial_showing = rows
        .iter()
        .find_map(|r| r.oid.map(Showing::Commit))
        .unwrap_or(Showing::Working);

    let mut showing = use_signal(|| initial_showing);
    let mut from = use_signal(|| None::<Oid>);
    let mut to = use_signal(|| None::<Oid>);
    let mut settings = use_signal(Settings::default);

    // Side-panel geometry, in pixels. Both dividers are draggable (see DragKind).
    let mut side_w = use_signal(|| 320.0_f64);
    let mut commits_h = use_signal(|| 380.0_f64);
    let drag = use_signal(|| None::<DragKind>);

    // Collapsed folder paths in the file tree (expanded by default => absent means expanded).
    let collapsed = use_signal(HashSet::<String>::new);

    // The rendered diff. Recomputed via an effect whenever the selection or whitespace setting
    // changes (font / wrap / line-numbers are pure CSS and don't need a rebuild).
    let mut diff = use_signal(RenderDiff::empty);
    {
        let engine = engine.clone();
        use_effect(move || {
            let s = *showing.read();
            let st = *settings.read();
            // Only the whitespace flag affects the model; read it so the effect tracks it.
            let _ = st.show_space;
            let rd = recompute(&engine, s, st);
            diff.set(rd);
        });
    }

    let st = *settings.read();
    let d = diff.read();
    let tree = build_tree(&d.files);

    // ----- toolbar summary pieces -----
    let summary = d.summary.clone();
    let total_added = d.added;
    let total_removed = d.removed;

    // ----- helpers to mutate selection -----
    let mut open_row = move |oid: Option<Oid>| {
        let next = match oid {
            Some(o) => Showing::Commit(o),
            None => Showing::Working,
        };
        if *showing.read() != next {
            showing.set(next);
        }
    };
    let mut set_from = move |oid: Oid| {
        from.set(Some(oid));
        if let (Some(a), Some(b)) = (*from.read(), *to.read()) {
            showing.set(Showing::Range(a, b));
        }
    };
    let mut set_to = move |oid: Oid| {
        to.set(Some(oid));
        if let (Some(a), Some(b)) = (*from.read(), *to.read()) {
            showing.set(Showing::Range(a, b));
        }
    };

    // Current selection for highlighting the active row.
    let cur = *showing.read();
    let cur_from = *from.read();
    let cur_to = *to.read();

    let font_px = st.font_size;
    // Gutter width scales with font; line-numbers toggle hides the numeric columns.
    let wrap_class = if st.word_wrap { "wrap" } else { "nowrap" };
    let ln_class = if st.line_numbers { "ln" } else { "no-ln" };

    rsx! {
        style { "{CSS}" }
        // Inline custom properties drive font-size + the resizable column/row tracks.
        div {
            class: "root",
            style: "--side-w: {side_w}px; --commits-h: {commits_h}px; --diff-font: {font_px}px;",

            // ===== TOOLBAR (full width, pinned at the very top) =====
            div { class: "toolbar",
                div { class: "summary",
                    span { class: "summary-text", "{summary}" }
                    span { class: "add", "+{total_added}" }
                    span { class: "del", "−{total_removed}" }
                }
                div { class: "tools",
                    ToolButton {
                        label: "⤶", active: st.word_wrap,
                        tip: if st.word_wrap { "Word wrap: on" } else { "Word wrap: off" },
                        onclick: move |_| { let mut s = settings.write(); s.word_wrap = !s.word_wrap; },
                    }
                    ToolButton {
                        label: "␣", active: st.show_space,
                        tip: if st.show_space { "Showing space changes" } else { "Ignoring whitespace-only changes" },
                        onclick: move |_| { let mut s = settings.write(); s.show_space = !s.show_space; },
                    }
                    ToolButton {
                        label: "A-", active: false, tip: "Decrease font size",
                        onclick: move |_| { let mut s = settings.write(); s.font_size = (s.font_size - 1.0).max(8.0); },
                    }
                    ToolButton {
                        label: "A+", active: false, tip: "Increase font size",
                        onclick: move |_| { let mut s = settings.write(); s.font_size = (s.font_size + 1.0).min(28.0); },
                    }
                    ToolButton {
                        label: "#", active: st.line_numbers,
                        tip: if st.line_numbers { "Line numbers: on" } else { "Line numbers: off" },
                        onclick: move |_| { let mut s = settings.write(); s.line_numbers = !s.line_numbers; },
                    }
                }
            }

            // ===== BODY: [ side panel | main view ] =====
            div { class: "body",

            // ===== LEFT SIDE PANEL =====
            aside {
                class: "side",
                // ----- commit list -----
                section {
                    class: "commits",
                    div { class: "section-head", "COMMITS · {engine.repo_name}" }
                    div { class: "scroll",
                        for r in rows.iter().cloned() {
                            CommitRow {
                                row: r.clone(),
                                is_current: row_is_current(cur, &r),
                                is_from: r.oid.is_some() && r.oid == cur_from,
                                is_to: r.oid.is_some() && r.oid == cur_to,
                                on_open: move |oid| open_row(oid),
                                on_from: move |oid| set_from(oid),
                                on_to: move |oid| set_to(oid),
                            }
                        }
                    }
                }
                // horizontal divider between commit list and file tree
                div {
                    class: "vdivider",
                    onmousedown: move |_| start_drag(drag, DragKind::Vert),
                }
                // ----- file tree (real hierarchical tree) -----
                section {
                    class: "files",
                    div { class: "section-head", "FILES ({d.files.len()})" }
                    div { class: "scroll",
                        for node in tree.iter().cloned() {
                            TreeNodeView { node, collapsed }
                        }
                    }
                }
            }

            // vertical divider between side panel and main view
            div {
                class: "hdivider",
                onmousedown: move |_| start_drag(drag, DragKind::Horiz),
            }

            // ===== MAIN VIEW =====
            main {
                class: "main",
                // ----- diff body -----
                div { class: "diff {wrap_class} {ln_class}",
                    if let Some(err) = &d.error {
                        div { class: "error", "Error: {err}" }
                    } else {
                        if let Some(msg) = &d.message {
                            div { class: "commit-msg",
                                div { class: "cm-title", "{msg.title}" }
                                div { class: "cm-meta", "{msg.author}  ·  {msg.date}  ·  {msg.short}" }
                                if !msg.body.is_empty() {
                                    pre { class: "cm-body", "{msg.body}" }
                                }
                            }
                        }
                        for (i, f) in d.files.iter().enumerate() {
                            FileSection { idx: i, file: f.clone() }
                        }
                        if d.files.is_empty() {
                            div { class: "empty", "No changes to show." }
                        }
                    }
                }
            }
            } // end .body

            // ===== DRAG OVERLAY =====
            // While a divider is held, a full-window transparent layer captures mousemove/up so the
            // drag keeps working even when the cursor leaves the thin divider.
            if drag.read().is_some() {
                div {
                    class: "drag-overlay",
                    style: match *drag.read() {
                        Some(DragKind::Horiz) => "cursor: col-resize;",
                        Some(DragKind::Vert) => "cursor: row-resize;",
                        None => "",
                    },
                    onmousemove: move |e| {
                        let p = e.client_coordinates();
                        match *drag.read() {
                            Some(DragKind::Horiz) => {
                                let w = p.x.clamp(180.0, 640.0);
                                side_w.set(w);
                            }
                            Some(DragKind::Vert) => {
                                // y is window-relative; subtract the full-width toolbar height that
                                // now sits above the side panel.
                                let h = (p.y - 38.0).clamp(120.0, 1000.0);
                                commits_h.set(h);
                            }
                            None => {}
                        }
                    },
                    onmouseup: move |_| end_drag(drag),
                    onmouseleave: move |_| end_drag(drag),
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum DragKind {
    Horiz, // side-panel width
    Vert,  // commit-list height
}

fn start_drag(mut drag: Signal<Option<DragKind>>, kind: DragKind) {
    drag.set(Some(kind));
}
fn end_drag(mut drag: Signal<Option<DragKind>>) {
    if drag.read().is_some() {
        drag.set(None);
    }
}

fn row_is_current(cur: Showing, r: &Row) -> bool {
    match (cur, r.oid) {
        (Showing::Working, None) => true,
        (Showing::Commit(o), Some(oid)) => o == oid,
        _ => false,
    }
}

// ---- components ----

#[component]
fn CommitRow(
    row: Row,
    is_current: bool,
    is_from: bool,
    is_to: bool,
    on_open: EventHandler<Option<Oid>>,
    on_from: EventHandler<Oid>,
    on_to: EventHandler<Oid>,
) -> Element {
    let cls = if is_current { "commit-row current" } else { "commit-row" };
    let oid = row.oid;
    rsx! {
        div { class: "{cls}",
            div { class: "cr-line1",
                if let Some(o) = oid {
                    button {
                        class: if is_from { "ep ep-on" } else { "ep" },
                        title: "Compare from this commit",
                        onclick: move |e| { e.stop_propagation(); on_from.call(o); },
                        "◀"
                    }
                    button {
                        class: if is_to { "ep ep-on" } else { "ep" },
                        title: "Compare to this commit",
                        onclick: move |e| { e.stop_propagation(); on_to.call(o); },
                        "▶"
                    }
                } else {
                    span { class: "ep-spacer" }
                }
                span { class: "sha", "{row.short}" }
                span { class: "cdate", "{row.date}" }
                span { class: "cauthor", "{row.author}" }
            }
            div {
                class: "cr-title",
                onclick: move |_| on_open.call(oid),
                "{row.title}"
            }
        }
    }
}

/// Recursively renders one file-tree node (directory or file leaf). `collapsed` is the shared set
/// of collapsed directory paths.
#[component]
fn TreeNodeView(node: TreeNode, collapsed: Signal<HashSet<String>>) -> Element {
    match node {
        TreeNode::Dir {
            name,
            path,
            depth,
            children,
        } => {
            let is_collapsed = collapsed.read().contains(&path);
            let indent = format!("padding-left: {}px;", 8 + depth * 14);
            let arrow = if is_collapsed { "▸" } else { "▾" };
            let toggle_path = path.clone();
            rsx! {
                div {
                    class: "tree-row tree-dir",
                    style: "{indent}",
                    onclick: move |_| {
                        let mut set = collapsed.write();
                        if !set.remove(&toggle_path) {
                            set.insert(toggle_path.clone());
                        }
                    },
                    span { class: "tri", "{arrow}" }
                    span { class: "ficon", "📁" }
                    span { class: "fpath", "{name}" }
                }
                if !is_collapsed {
                    for child in children.iter().cloned() {
                        TreeNodeView { node: child, collapsed }
                    }
                }
            }
        }
        TreeNode::File {
            name,
            idx,
            icon,
            added,
            removed,
            depth,
        } => {
            // file leaves align past the disclosure-triangle column of their parent dir.
            let indent = format!("padding-left: {}px;", 8 + depth * 14 + 14);
            rsx! {
                div {
                    class: "tree-row file-row",
                    style: "{indent}",
                    onclick: move |_| {
                        let js = format!(
                            "var e=document.getElementById('file-{idx}'); if(e) e.scrollIntoView({{behavior:'smooth',block:'start'}});"
                        );
                        let _ = document::eval(&js);
                    },
                    span { class: "ficon", "{icon}" }
                    span { class: "fpath", "{name}" }
                    span { class: "fcount",
                        span { class: "add", "+{added}" }
                        span { class: "del", "−{removed}" }
                    }
                }
            }
        }
    }
}

#[component]
fn ToolButton(label: &'static str, active: bool, tip: String, onclick: EventHandler<MouseEvent>) -> Element {
    let cls = if active { "tool active" } else { "tool" };
    rsx! {
        button { class: "{cls}", title: "{tip}", onclick: move |e| onclick.call(e), "{label}" }
    }
}

#[component]
fn FileSection(idx: usize, file: crate::model::RFile) -> Element {
    rsx! {
        section { class: "file", id: "file-{idx}",
            // sticky per-file header
            div { class: "file-head",
                span { class: "ficon", "{file.icon}" }
                span { class: "fh-path", "{file.header_label}" }
                span { class: "fh-kind", "[{file.kind_letter}]" }
                span { class: "fh-count",
                    span { class: "add", "+{file.added}" }
                    span { class: "del", "−{file.removed}" }
                }
            }
            if file.binary {
                div { class: "binary", "Binary file not shown" }
            } else {
                div { class: "code",
                    for (li, line) in file.lines.iter().enumerate() {
                        DiffRow { key: "{li}", line: line.clone() }
                    }
                }
            }
        }
    }
}

#[component]
fn DiffRow(line: crate::model::RLine) -> Element {
    use crate::git::LineKind;
    let (row_cls, sign) = if line.is_hunk_header {
        ("drow hunk", ' ')
    } else {
        match line.kind {
            LineKind::Added => ("drow add", '+'),
            LineKind::Removed => ("drow del", '-'),
            LineKind::Context => ("drow ctx", ' '),
        }
    };
    let old = line.old_no.map(|n| n.to_string()).unwrap_or_default();
    let new = line.new_no.map(|n| n.to_string()).unwrap_or_default();
    let sign_str = if sign == ' ' { String::new() } else { sign.to_string() };

    rsx! {
        div { class: "{row_cls}",
            if !line.is_hunk_header {
                span { class: "gutter old", "{old}" }
                span { class: "gutter new", "{new}" }
                span { class: "sign", "{sign_str}" }
            }
            span { class: "ltext",
                if line.spans.is_empty() {
                    "\u{00a0}"
                } else {
                    for sp in line.spans.iter() {
                        span {
                            style: span_style(sp),
                            "{sp.text}"
                        }
                    }
                }
            }
        }
    }
}

fn span_style(sp: &crate::model::RSpan) -> String {
    let mut s = format!("color:{};", sp.color);
    if sp.bold {
        s.push_str("font-weight:600;");
    }
    if sp.italic {
        s.push_str("font-style:italic;");
    }
    s
}
