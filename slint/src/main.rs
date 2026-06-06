//! git-review — a GitHub-style code review desktop app, built with Slint.
//!
//! Usage: `git-review-slint [path-to-repo]` (defaults to the current directory or
//! `$GIT_REVIEW_REPO`).

mod git;
mod highlight;

use std::cell::RefCell;
use std::rc::Rc;

use git2::Oid;
use slint::{Color, ModelRc, SharedString, VecModel};

use git::{ChangeKind, CommitInfo, DiffSet, LineKind, Repo};
use highlight::Highlighter;

slint::include_modules!();

#[derive(Clone, Copy, PartialEq)]
enum Showing {
    Working,
    Commit(Oid),
    Range(Oid, Oid),
}

struct Settings {
    word_wrap: bool,
    show_space: bool,
    line_numbers: bool,
    font: f32,
}

struct HeaderPos {
    y: f32,
    path: String,
    added: i32,
    removed: i32,
}

struct State {
    showing: Showing,
    from: Option<Oid>,
    to: Option<Oid>,
    settings: Settings,
    commits: Vec<CommitInfo>,
    headers: Vec<HeaderPos>,
    // The flattened file tree for the current diff (built in `recompute`). The UI repeats this
    // model directly; `toggle-node` flips `expanded`/`visible` and we re-push it.
    tree: Vec<TreeNode>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("GIT_REVIEW_REPO").ok())
        .unwrap_or_else(|| ".".to_string());

    let repo = match Repo::open(&path) {
        Ok(r) => Rc::new(r),
        Err(e) => {
            eprintln!("Could not open a git repository at '{path}': {e}");
            std::process::exit(1);
        }
    };
    let hl = Rc::new(Highlighter::new());

    let commits = repo.commits(500).unwrap_or_default();
    let showing = commits
        .iter()
        .find_map(|c| c.oid.map(Showing::Commit))
        .unwrap_or(Showing::Working);

    let state = Rc::new(RefCell::new(State {
        showing,
        from: None,
        to: None,
        settings: Settings {
            word_wrap: false,
            show_space: true,
            line_numbers: true,
            font: 12.0,
        },
        commits,
        headers: Vec::new(),
        tree: Vec::new(),
    }));

    let app = AppWindow::new()?;
    app.set_repo_name(repo.workdir_name().into());

    // --- wire callbacks ---
    macro_rules! refresh {
        () => {
            refresh_commits(&app, &state);
            recompute(&app, &repo, &hl, &state);
        };
    }

    {
        let app_w = app.as_weak();
        let state = state.clone();
        let repo = repo.clone();
        let hl = hl.clone();
        app.on_select_commit(move |i| {
            let app = app_w.unwrap();
            {
                let mut st = state.borrow_mut();
                let oid = st.commits.get(i as usize).and_then(|c| c.oid);
                st.showing = match oid {
                    Some(o) => Showing::Commit(o),
                    None => Showing::Working,
                };
            }
            refresh_commits(&app, &state);
            recompute(&app, &repo, &hl, &state);
        });
    }
    bind_endpoint(&app, &state, &repo, &hl, true);
    bind_endpoint(&app, &state, &repo, &hl, false);

    {
        let app_w = app.as_weak();
        let state = state.clone();
        app.on_select_file(move |i| {
            let app = app_w.unwrap();
            let y = state
                .borrow()
                .headers
                .get(i as usize)
                .map(|h| h.y)
                .unwrap_or(0.0);
            app.invoke_scroll_diff_to(y);
        });
    }

    {
        // Toggle a folder node: flip `expanded` and recompute every node's `visible` flag
        // (a node is visible iff all its ancestor folders are expanded), then re-push the model.
        let app_w = app.as_weak();
        let state = state.clone();
        app.on_toggle_node(move |i| {
            let app = app_w.unwrap();
            {
                let mut st = state.borrow_mut();
                if let Some(n) = st.tree.get_mut(i as usize) {
                    if n.is_dir {
                        n.expanded = !n.expanded;
                    }
                }
                recompute_visibility(&mut st.tree);
            }
            push_tree(&app, &state);
        });
    }

    bind_toggle(&app, &state, &repo, &hl, Toggle::Wrap);
    bind_toggle(&app, &state, &repo, &hl, Toggle::Space);
    bind_toggle(&app, &state, &repo, &hl, Toggle::LineNumbers);
    bind_toggle(&app, &state, &repo, &hl, Toggle::FontInc);
    bind_toggle(&app, &state, &repo, &hl, Toggle::FontDec);

    {
        let app_w = app.as_weak();
        let state = state.clone();
        app.on_scrolled(move |y| {
            let app = app_w.unwrap();
            update_sticky(&app, &state, y);
        });
    }

    refresh!();
    app.run()?;
    Ok(())
}

#[derive(Clone, Copy)]
enum Toggle {
    Wrap,
    Space,
    LineNumbers,
    FontInc,
    FontDec,
}

fn bind_endpoint(
    app: &AppWindow,
    state: &Rc<RefCell<State>>,
    repo: &Rc<Repo>,
    hl: &Rc<Highlighter>,
    is_from: bool,
) {
    let app_w = app.as_weak();
    let state = state.clone();
    let repo = repo.clone();
    let hl = hl.clone();
    let handler = move |i: i32| {
        let app = app_w.unwrap();
        {
            let mut st = state.borrow_mut();
            let oid = st.commits.get(i as usize).and_then(|c| c.oid);
            if let Some(o) = oid {
                if is_from {
                    st.from = Some(o);
                } else {
                    st.to = Some(o);
                }
                if let (Some(a), Some(b)) = (st.from, st.to) {
                    st.showing = Showing::Range(a, b);
                }
            }
        }
        refresh_commits(&app, &state);
        recompute(&app, &repo, &hl, &state);
    };
    if is_from {
        app.on_set_from(handler);
    } else {
        app.on_set_to(handler);
    }
}

fn bind_toggle(
    app: &AppWindow,
    state: &Rc<RefCell<State>>,
    repo: &Rc<Repo>,
    hl: &Rc<Highlighter>,
    which: Toggle,
) {
    let app_w = app.as_weak();
    let state = state.clone();
    let repo = repo.clone();
    let hl = hl.clone();
    let handler = move || {
        let app = app_w.unwrap();
        let mut recompute_needed = false;
        {
            let mut st = state.borrow_mut();
            match which {
                Toggle::Wrap => st.settings.word_wrap = !st.settings.word_wrap,
                Toggle::Space => {
                    st.settings.show_space = !st.settings.show_space;
                    recompute_needed = true;
                }
                Toggle::LineNumbers => st.settings.line_numbers = !st.settings.line_numbers,
                Toggle::FontInc => st.settings.font = (st.settings.font + 1.0).min(28.0),
                Toggle::FontDec => st.settings.font = (st.settings.font - 1.0).max(8.0),
            }
        }
        let _ = recompute_needed;
        recompute(&app, &repo, &hl, &state);
    };
    match which {
        Toggle::Wrap => app.on_toggle_wrap(handler),
        Toggle::Space => app.on_toggle_space(handler),
        Toggle::LineNumbers => app.on_toggle_line_numbers(handler),
        Toggle::FontInc => app.on_font_inc(handler),
        Toggle::FontDec => app.on_font_dec(handler),
    }
}

fn refresh_commits(app: &AppWindow, state: &Rc<RefCell<State>>) {
    let st = state.borrow();
    let rows: Vec<CommitRow> = st
        .commits
        .iter()
        .map(|c| {
            let current = match (st.showing, c.oid) {
                (Showing::Working, _) if c.is_working_tree() => true,
                (Showing::Commit(o), Some(oid)) => o == oid,
                _ => false,
            };
            CommitRow {
                short: c.short.clone().into(),
                date: c.date.clone().into(),
                author: c.author.clone().into(),
                title: c.title.clone().into(),
                current,
                is_from: c.oid.is_some() && c.oid == st.from,
                is_to: c.oid.is_some() && c.oid == st.to,
                working: c.is_working_tree(),
            }
        })
        .collect();
    app.set_commits(ModelRc::new(VecModel::from(rows)));
}

fn recompute(app: &AppWindow, repo: &Rc<Repo>, hl: &Rc<Highlighter>, state: &Rc<RefCell<State>>) {
    let (showing, ignore_ws, font, line_numbers, word_wrap, show_space) = {
        let st = state.borrow();
        (
            st.showing,
            !st.settings.show_space,
            st.settings.font,
            st.settings.line_numbers,
            st.settings.word_wrap,
            st.settings.show_space,
        )
    };

    let diff: DiffSet = match showing {
        Showing::Working => repo.working_tree(ignore_ws),
        Showing::Commit(o) => repo.show(o, ignore_ws),
        Showing::Range(a, b) => repo.range(a, b, ignore_ws),
    }
    .unwrap_or_else(|_| DiffSet {
        message: None,
        files: Vec::new(),
        added: 0,
        removed: 0,
        summary: "error".into(),
    });

    app.set_summary(diff.summary.clone().into());
    app.set_total_added(diff.added as i32);
    app.set_total_removed(diff.removed as i32);
    app.set_file_count(diff.files.len() as i32);
    app.set_line_numbers(line_numbers);
    app.set_wrap_lines(word_wrap);
    app.set_show_space(show_space);
    app.set_diff_font(font);

    // file tree — build a real hierarchical, flattened tree (folders + leaves)
    let tree = build_tree(&diff.files);
    state.borrow_mut().tree = tree;
    push_tree(app, state);

    // diff items + header offsets
    let line_h = font * 1.5;
    let header_h = 30.0_f32;
    let mut items: Vec<DiffItem> = Vec::new();
    let mut headers: Vec<HeaderPos> = Vec::new();
    let mut y = 0.0f32;

    if let Some(msg) = &diff.message {
        let body_lines = if msg.body.is_empty() {
            0
        } else {
            msg.body.lines().count()
        };
        let h = 24.0 + 24.0 + 18.0 + body_lines as f32 * (font * 1.4) + 24.0;
        items.push(DiffItem {
            row_h: h,
            ..blank_item(
                2,
                &msg.title,
                &format!("{} · {} · {}", msg.author, msg.date, msg.short),
                &msg.body,
            )
        });
        y += h;
    }

    for f in &diff.files {
        headers.push(HeaderPos {
            y,
            path: f.path.clone(),
            added: f.added as i32,
            removed: f.removed as i32,
        });
        items.push(DiffItem {
            row_h: header_h,
            ..header_item(f)
        });
        y += header_h;

        if f.binary {
            items.push(DiffItem {
                row_h: line_h,
                ..code_item(
                    LineKind::Context,
                    "",
                    "",
                    "Binary file not shown",
                    vec![Span {
                        text: "Binary file not shown".into(),
                        col: Color::from_rgb_u8(0x65, 0x6d, 0x76),
                    }],
                    0,
                )
            });
            y += line_h;
            continue;
        }

        let syntax = hl.syntax_for(&f.path);
        for hunk in &f.hunks {
            items.push(DiffItem {
                row_h: line_h,
                ..hunk_item(&hunk.header)
            });
            y += line_h;
            for line in &hunk.lines {
                let spans: Vec<Span> = hl
                    .line(syntax, &line.text)
                    .into_iter()
                    .map(|s| Span {
                        text: s.text.into(),
                        col: Color::from_rgb_u8(s.color.0, s.color.1, s.color.2),
                    })
                    .collect();
                let spans = if spans.is_empty() {
                    vec![Span {
                        text: " ".into(),
                        col: Color::from_rgb_u8(0x1f, 0x23, 0x28),
                    }]
                } else {
                    spans
                };
                let (sign, old_no, new_no, code_kind) = match line.kind {
                    LineKind::Added => ("+", String::new(), num(line.new_no), 1),
                    LineKind::Removed => ("-", num(line.old_no), String::new(), 2),
                    LineKind::Context => (" ", num(line.old_no), num(line.new_no), 0),
                };
                items.push(DiffItem {
                    row_h: line_h,
                    ..code_item(line.kind, &old_no, &new_no, sign, spans, code_kind)
                });
                y += line_h;
            }
        }
    }

    app.set_diff_items(ModelRc::new(VecModel::from(items)));
    state.borrow_mut().headers = headers;
    app.invoke_scroll_diff_to(0.0);
    update_sticky(app, state, 0.0);
}

fn update_sticky(app: &AppWindow, state: &Rc<RefCell<State>>, y: f32) {
    let st = state.borrow();
    let cur = st.headers.iter().rev().find(|h| h.y <= y + 0.5);
    match cur {
        Some(h) => {
            app.set_sticky_path(h.path.clone().into());
            app.set_sticky_added(h.added);
            app.set_sticky_removed(h.removed);
            app.set_sticky_visible(true);
        }
        None => app.set_sticky_visible(false),
    }
}

// --- file tree --------------------------------------------------------------

/// An intermediate tree node used while building the flattened model.
enum Node {
    Dir {
        name: String,
        children: Vec<(String, Node)>, // (component, child); preserves insertion order
    },
    File {
        name: String,
        icon: String,
        added: i32,
        removed: i32,
        file_index: i32,
    },
}

/// Build a flattened `[TreeNode]` from the diff's file list:
/// directory components become collapsible folder nodes; files are leaves with `+a −r` counts.
/// Single-child directory chains are collapsed GitHub-style (`a/b/c.rs`); distinct subtrees branch.
fn build_tree(files: &[git::FileDiff]) -> Vec<TreeNode> {
    // Build an order-preserving prefix tree of directory components.
    let mut roots: Vec<(String, Node)> = Vec::new();

    for (idx, f) in files.iter().enumerate() {
        let parts: Vec<&str> = f.path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            continue;
        }
        let (dirs, file_name) = parts.split_at(parts.len() - 1);
        let leaf = Node::File {
            name: file_name[0].to_string(),
            icon: file_icon(&f.path).to_string(),
            added: f.added as i32,
            removed: f.removed as i32,
            file_index: idx as i32,
        };
        insert(&mut roots, dirs, file_name[0], leaf);
    }

    // Collapse single-child directory chains (GitHub-style: `a/b/c.rs`).
    collapse_chains(&mut roots);

    // Flatten depth-first into rows.
    let mut out: Vec<TreeNode> = Vec::new();
    flatten(&roots, 0, &mut out);
    out
}

/// Recursively insert a file leaf under the given directory path, creating dirs as needed.
fn insert(nodes: &mut Vec<(String, Node)>, dirs: &[&str], file_name: &str, leaf: Node) {
    match dirs.split_first() {
        None => {
            nodes.push((file_name.to_string(), leaf));
        }
        Some((comp, rest)) => {
            let pos = match nodes.iter().position(|(c, _)| c == comp) {
                Some(i) => i,
                None => {
                    nodes.push((
                        comp.to_string(),
                        Node::Dir {
                            name: comp.to_string(),
                            children: Vec::new(),
                        },
                    ));
                    nodes.len() - 1
                }
            };
            if let Node::Dir { children, .. } = &mut nodes[pos].1 {
                insert(children, rest, file_name, leaf);
            }
        }
    }
}

/// Collapse a folder that has exactly one child folder into a single `parent/child` node.
fn collapse_chains(nodes: &mut Vec<(String, Node)>) {
    for (_, node) in nodes.iter_mut() {
        if let Node::Dir { name, children } = node {
            // Merge while this dir has exactly one child and that child is also a dir.
            while children.len() == 1 {
                let only_is_dir = matches!(children[0].1, Node::Dir { .. });
                if !only_is_dir {
                    break;
                }
                let (_, child) = children.remove(0);
                if let Node::Dir {
                    name: cname,
                    children: cchildren,
                } = child
                {
                    *name = format!("{name}/{cname}");
                    *children = cchildren;
                }
            }
            collapse_chains(children);
        }
    }
}

fn flatten(nodes: &[(String, Node)], depth: i32, out: &mut Vec<TreeNode>) {
    for (_, node) in nodes {
        match node {
            Node::Dir { name, children } => {
                out.push(TreeNode {
                    depth,
                    is_dir: true,
                    expanded: true,
                    visible: true,
                    name: name.clone().into(),
                    icon: "📁".into(),
                    added: 0,
                    removed: 0,
                    file_index: -1,
                });
                flatten(children, depth + 1, out);
            }
            Node::File {
                name,
                icon,
                added,
                removed,
                file_index,
            } => {
                out.push(TreeNode {
                    depth,
                    is_dir: false,
                    expanded: false,
                    visible: true,
                    name: name.clone().into(),
                    icon: icon.clone().into(),
                    added: *added,
                    removed: *removed,
                    file_index: *file_index,
                });
            }
        }
    }
}

/// Recompute each node's `visible` flag: a row is visible iff every ancestor folder is expanded.
/// Uses a stack of (depth, expanded) for the currently open ancestors.
fn recompute_visibility(tree: &mut [TreeNode]) {
    let mut stack: Vec<(i32, bool)> = Vec::new();
    for n in tree.iter_mut() {
        while let Some(&(d, _)) = stack.last() {
            if d >= n.depth {
                stack.pop();
            } else {
                break;
            }
        }
        let ancestors_open = stack.iter().all(|&(_, e)| e);
        n.visible = ancestors_open;
        if n.is_dir {
            stack.push((n.depth, n.expanded));
        }
    }
}

fn push_tree(app: &AppWindow, state: &Rc<RefCell<State>>) {
    // Only push currently-visible rows so every delegate is a constant-height row (collapsed
    // subtrees simply drop out of the model). `toggle-node` indexes into the full `state.tree`,
    // so we carry the full-tree row index in `file-index` for folders to remap clicks.
    let st = state.borrow();
    let rows: Vec<TreeNode> = st
        .tree
        .iter()
        .enumerate()
        .filter(|(_, n)| n.visible)
        .map(|(i, n)| {
            let mut n = n.clone();
            if n.is_dir {
                // For folders, stash the full-tree index so toggle-node can find the node.
                n.file_index = i as i32;
            }
            n
        })
        .collect();
    app.set_tree(ModelRc::new(VecModel::from(rows)));
}

// --- DiffItem constructors --------------------------------------------------

fn empty_item() -> DiffItem {
    DiffItem {
        kind: 0,
        code_kind: 0,
        old_no: SharedString::new(),
        new_no: SharedString::new(),
        sign: SharedString::new(),
        spans: ModelRc::new(VecModel::from(Vec::<Span>::new())),
        path: SharedString::new(),
        added: 0,
        removed: 0,
        text: SharedString::new(),
        sub: SharedString::new(),
        row_h: 0.0,
    }
}

fn header_item(f: &git::FileDiff) -> DiffItem {
    let label = match (&f.old_path, f.kind) {
        (Some(old), ChangeKind::Renamed) => format!("{old}  →  {}", f.path),
        _ => f.path.clone(),
    };
    DiffItem {
        kind: 1,
        path: label.into(),
        added: f.added as i32,
        removed: f.removed as i32,
        ..empty_item()
    }
}

fn hunk_item(header: &str) -> DiffItem {
    DiffItem {
        kind: 0,
        code_kind: 3,
        text: header.into(),
        ..empty_item()
    }
}

fn code_item(
    _kind: LineKind,
    old_no: &str,
    new_no: &str,
    sign: &str,
    spans: Vec<Span>,
    code_kind: i32,
) -> DiffItem {
    DiffItem {
        kind: 0,
        code_kind,
        old_no: old_no.into(),
        new_no: new_no.into(),
        sign: sign.into(),
        spans: ModelRc::new(VecModel::from(spans)),
        ..empty_item()
    }
}

fn blank_item(kind: i32, title: &str, sub: &str, body: &str) -> DiffItem {
    DiffItem {
        kind,
        text: title.into(),
        sub: sub.into(),
        path: body.into(),
        ..empty_item()
    }
}

fn num(n: Option<u32>) -> String {
    n.map(|v| v.to_string()).unwrap_or_default()
}

fn file_icon(path: &str) -> &'static str {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext {
        "rs" => "🦀",
        "py" => "🐍",
        "js" | "ts" | "tsx" | "jsx" => "📜",
        "md" => "📝",
        "toml" | "yaml" | "yml" | "json" | "ini" | "cfg" => "⚙",
        "png" | "jpg" | "jpeg" | "gif" | "svg" => "🖼",
        "sh" | "bash" => "💲",
        "html" | "css" => "🌐",
        _ => "📄",
    }
}
