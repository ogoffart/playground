//! git-review — a GitHub-style code review desktop app, built with qmetaobject-rs (Qt Quick/QML).
//!
//! The Rust `Backend` QObject exposes the model as JSON strings (parsed in QML) plus the toolbar
//! settings, and a handful of slots. The QML in `qml/main.qml` renders it with `SplitView`
//! (resizable splits) and a `ListView` whose `section` gives true sticky per-file headers.
//!
//! Usage: `git-review-qmetaobject [path-to-repo]` (defaults to cwd / `$GIT_REVIEW_REPO`).
#![allow(non_snake_case)]

mod git;
mod highlight;

use git2::Oid;
use qmetaobject::prelude::*;
use serde::Serialize;

use git::{CommitInfo, DiffSet, LineKind, Repo};
use highlight::Highlighter;

#[derive(Clone, Copy, PartialEq)]
enum Showing {
    Working,
    Commit(Oid),
    Range(Oid, Oid),
}

impl Default for Showing {
    fn default() -> Self {
        Showing::Working
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CommitJson {
    short: String,
    date: String,
    author: String,
    title: String,
    current: bool,
    is_from: bool,
    is_to: bool,
    working: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileJson {
    icon: String,
    path: String,
    added: i32,
    removed: i32,
    item_index: i32,
}

/// A flattened tree node (folders + file leaves) for the side-panel file tree.
/// The QML side renders only nodes whose `visible` is true, indented by `depth`.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TreeNodeJson {
    depth: i32,
    is_dir: bool,
    expanded: bool,
    visible: bool,
    name: String,
    icon: String,
    added: i32,
    removed: i32,
    item_index: i32,
    /// Index of this node's parent in the flat array, or -1 for roots.
    parent: i32,
}

/// Internal tree representation built from the flat `FileDiff` paths, then flattened
/// (pre-order) into `Vec<TreeNodeJson>`, GitHub-style collapsing single-child dir chains.
#[derive(Default)]
struct TreeBuilder {
    name: String,
    children: Vec<TreeBuilder>,
    // leaf-only fields
    is_leaf: bool,
    icon: String,
    added: i32,
    removed: i32,
    item_index: i32,
}

impl TreeBuilder {
    fn child_mut(&mut self, name: &str) -> &mut TreeBuilder {
        if let Some(pos) = self
            .children
            .iter()
            .position(|c| !c.is_leaf && c.name == name)
        {
            return &mut self.children[pos];
        }
        self.children.push(TreeBuilder {
            name: name.to_string(),
            ..Default::default()
        });
        self.children.last_mut().unwrap()
    }

    fn insert(&mut self, comps: &[&str], leaf: FileJson) {
        match comps {
            [] => {}
            [last] => {
                self.children.push(TreeBuilder {
                    name: (*last).to_string(),
                    is_leaf: true,
                    icon: leaf.icon,
                    added: leaf.added,
                    removed: leaf.removed,
                    item_index: leaf.item_index,
                    ..Default::default()
                });
            }
            [head, rest @ ..] => {
                self.child_mut(head).insert(rest, leaf);
            }
        }
    }

    /// Collapse single-child directory chains (`a/ -> b/ -> c.rs` becomes `a/b/`).
    fn collapse(&mut self) {
        for c in &mut self.children {
            c.collapse();
        }
        // If this dir has exactly one child and that child is a dir, fold them.
        if !self.is_leaf
            && self.children.len() == 1
            && !self.children[0].is_leaf
            && !self.name.is_empty()
        {
            let mut only = self.children.remove(0);
            self.name = format!("{}/{}", self.name, only.name);
            self.children = std::mem::take(&mut only.children);
        }
    }

    /// Sort: directories first, then files, each alphabetical.
    fn sort(&mut self) {
        self.children
            .sort_by(|a, b| (a.is_leaf, &a.name).cmp(&(b.is_leaf, &b.name)));
        for c in &mut self.children {
            c.sort();
        }
    }

    /// Pre-order flatten into the output vector. `expanded` persisted from `prev`.
    fn flatten(
        &self,
        depth: i32,
        parent: i32,
        out: &mut Vec<TreeNodeJson>,
        expanded_of: &dyn Fn(&str) -> bool,
        path_prefix: &str,
    ) {
        for c in &self.children {
            let full = if path_prefix.is_empty() {
                c.name.clone()
            } else {
                format!("{}/{}", path_prefix, c.name)
            };
            let my_index = out.len() as i32;
            if c.is_leaf {
                out.push(TreeNodeJson {
                    depth,
                    is_dir: false,
                    expanded: false,
                    visible: true, // visibility recomputed afterwards
                    name: c.name.clone(),
                    icon: c.icon.clone(),
                    added: c.added,
                    removed: c.removed,
                    item_index: c.item_index,
                    parent,
                });
            } else {
                let exp = expanded_of(&full);
                out.push(TreeNodeJson {
                    depth,
                    is_dir: true,
                    expanded: exp,
                    visible: true,
                    name: c.name.clone(),
                    icon: "📁".into(),
                    added: 0,
                    removed: 0,
                    item_index: -1,
                    parent,
                });
                c.flatten(depth + 1, my_index, out, expanded_of, &full);
            }
        }
    }
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct DiffItemJson {
    kind: i32,
    code_kind: i32,
    old_no: String,
    new_no: String,
    sign: String,
    html: String,
    file: String,
    title: String,
    sub: String,
    body: String,
}

#[derive(QObject, Default)]
struct Backend {
    base: qt_base_class!(trait QObject),

    commitsJson: qt_property!(QString; NOTIFY updated),
    filesJson: qt_property!(QString; NOTIFY updated),
    treeJson: qt_property!(QString; NOTIFY updated),
    diffJson: qt_property!(QString; NOTIFY updated),
    dark: qt_property!(bool; NOTIFY updated),
    summary: qt_property!(QString; NOTIFY updated),
    totalAdded: qt_property!(i32; NOTIFY updated),
    totalRemoved: qt_property!(i32; NOTIFY updated),
    repoName: qt_property!(QString; NOTIFY updated),
    wrap: qt_property!(bool; NOTIFY updated),
    showSpace: qt_property!(bool; NOTIFY updated),
    lineNumbers: qt_property!(bool; NOTIFY updated),
    font: qt_property!(i32; NOTIFY updated),
    updated: qt_signal!(),

    select_commit: qt_method!(fn select_commit(&mut self, i: usize) {
        let oid = self.infos.get(i).and_then(|c| c.oid);
        self.showing = match oid {
            Some(o) => Showing::Commit(o),
            None => Showing::Working,
        };
        self.rebuild();
        self.updated();
    }),
    set_from: qt_method!(fn set_from(&mut self, i: usize) {
        if let Some(o) = self.infos.get(i).and_then(|c| c.oid) {
            self.from = Some(o);
            self.maybe_range();
        }
        self.rebuild();
        self.updated();
    }),
    set_to: qt_method!(fn set_to(&mut self, i: usize) {
        if let Some(o) = self.infos.get(i).and_then(|c| c.oid) {
            self.to = Some(o);
            self.maybe_range();
        }
        self.rebuild();
        self.updated();
    }),
    toggle_wrap: qt_method!(fn toggle_wrap(&mut self) {
        self.wrap = !self.wrap;
        self.updated();
    }),
    toggle_space: qt_method!(fn toggle_space(&mut self) {
        self.showSpace = !self.showSpace;
        self.rebuild();
        self.updated();
    }),
    toggle_line_numbers: qt_method!(fn toggle_line_numbers(&mut self) {
        self.lineNumbers = !self.lineNumbers;
        self.updated();
    }),
    font_inc: qt_method!(fn font_inc(&mut self) {
        self.font = (self.font + 1).min(28);
        self.updated();
    }),
    font_dec: qt_method!(fn font_dec(&mut self) {
        self.font = (self.font - 1).max(8);
        self.updated();
    }),
    toggle_node: qt_method!(fn toggle_node(&mut self, i: usize) {
        let is_dir = self.tree.get(i).map(|n| n.is_dir).unwrap_or(false);
        if !is_dir {
            return;
        }
        let now_expanded = !self.tree[i].expanded;
        self.tree[i].expanded = now_expanded;
        let full = self.node_path(i);
        if now_expanded {
            self.collapsed.remove(&full);
        } else {
            self.collapsed.insert(full);
        }
        self.recompute_visibility();
        self.treeJson = serde_json::to_string(&self.tree).unwrap_or_default().into();
        self.updated();
    }),

    // --- non-Qt state ---
    repo: Option<Repo>,
    hl: Option<Highlighter>,
    infos: Vec<CommitInfo>,
    showing: Showing,
    from: Option<Oid>,
    to: Option<Oid>,
    tree: Vec<TreeNodeJson>,
    /// Full directory paths the user has explicitly collapsed (default = expanded).
    collapsed: std::collections::HashSet<String>,
}

impl Backend {
    /// Reconstruct a node's full directory path by walking parents (names may contain
    /// '/' from collapsed chains, which is fine — it stays a stable key).
    fn node_path(&self, i: usize) -> String {
        let mut parts: Vec<&str> = Vec::new();
        let mut cur = i as i32;
        while cur >= 0 {
            let n = &self.tree[cur as usize];
            parts.push(&n.name);
            cur = n.parent;
        }
        parts.reverse();
        parts.join("/")
    }

    /// A node is visible iff every ancestor directory is expanded.
    fn recompute_visibility(&mut self) {
        for i in 0..self.tree.len() {
            let mut vis = true;
            let mut p = self.tree[i].parent;
            while p >= 0 {
                let pn = &self.tree[p as usize];
                if !pn.expanded {
                    vis = false;
                    break;
                }
                p = pn.parent;
            }
            self.tree[i].visible = vis;
        }
    }

    fn maybe_range(&mut self) {
        if let (Some(a), Some(b)) = (self.from, self.to) {
            self.showing = Showing::Range(a, b);
        }
    }

    fn rebuild(&mut self) {
        let repo = self.repo.as_ref().unwrap();
        let hl = self.hl.as_ref().unwrap();
        let ignore_ws = !self.showSpace;

        // commit list
        let commits: Vec<CommitJson> = self
            .infos
            .iter()
            .map(|c| {
                let current = match (self.showing, c.oid) {
                    (Showing::Working, _) if c.is_working_tree() => true,
                    (Showing::Commit(o), Some(oid)) => o == oid,
                    _ => false,
                };
                CommitJson {
                    short: c.short.clone(),
                    date: c.date.clone(),
                    author: c.author.clone(),
                    title: c.title.clone(),
                    current,
                    is_from: c.oid.is_some() && c.oid == self.from,
                    is_to: c.oid.is_some() && c.oid == self.to,
                    working: c.is_working_tree(),
                }
            })
            .collect();
        self.commitsJson = serde_json::to_string(&commits).unwrap_or_default().into();

        let diff: DiffSet = match self.showing {
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

        self.summary = diff.summary.clone().into();
        self.totalAdded = diff.added as i32;
        self.totalRemoved = diff.removed as i32;

        // build diff items first so we can record each file's first item index
        let mut items: Vec<DiffItemJson> = Vec::new();
        if let Some(msg) = &diff.message {
            items.push(DiffItemJson {
                kind: 2,
                title: msg.title.clone(),
                sub: format!("{} · {} · {}", msg.author, msg.date, msg.short),
                body: msg.body.clone(),
                ..Default::default()
            });
        }

        let mut files: Vec<FileJson> = Vec::new();
        for f in &diff.files {
            let item_index = items.len() as i32;
            files.push(FileJson {
                icon: file_icon(&f.path).to_string(),
                path: f.path.clone(),
                added: f.added as i32,
                removed: f.removed as i32,
                item_index,
            });

            if f.binary {
                items.push(DiffItemJson {
                    kind: 0,
                    code_kind: 0,
                    file: f.path.clone(),
                    html: "<i>Binary file not shown</i>".into(),
                    ..Default::default()
                });
                continue;
            }

            let syntax = hl.syntax_for(&f.path);
            for hunk in &f.hunks {
                items.push(DiffItemJson {
                    kind: 0,
                    code_kind: 3,
                    file: f.path.clone(),
                    html: format!(
                        "<span style=\"color:#0969da\">{}</span>",
                        html_escape(&hunk.header)
                    ),
                    ..Default::default()
                });
                for line in &hunk.lines {
                    let (code_kind, old_no, new_no, sign) = match line.kind {
                        LineKind::Added => (1, String::new(), num(line.new_no), "+"),
                        LineKind::Removed => (2, num(line.old_no), String::new(), "-"),
                        LineKind::Context => (0, num(line.old_no), num(line.new_no), " "),
                    };
                    items.push(DiffItemJson {
                        kind: 0,
                        code_kind,
                        old_no,
                        new_no,
                        sign: sign.to_string(),
                        html: spans_html(hl, syntax, &line.text),
                        file: f.path.clone(),
                        ..Default::default()
                    });
                }
            }
        }

        self.filesJson = serde_json::to_string(&files).unwrap_or_default().into();
        self.diffJson = serde_json::to_string(&items).unwrap_or_default().into();

        // ---- hierarchical file tree (flattened) ----
        let mut root = TreeBuilder::default();
        for f in &files {
            let comps: Vec<&str> = f.path.split('/').filter(|s| !s.is_empty()).collect();
            root.insert(
                &comps,
                FileJson {
                    icon: f.icon.clone(),
                    path: f.path.clone(),
                    added: f.added,
                    removed: f.removed,
                    item_index: f.item_index,
                },
            );
        }
        root.collapse();
        root.sort();
        let collapsed = &self.collapsed;
        let expanded_of = move |p: &str| !collapsed.contains(p);
        let mut nodes: Vec<TreeNodeJson> = Vec::new();
        root.flatten(0, -1, &mut nodes, &expanded_of, "");
        self.tree = nodes;
        self.recompute_visibility();
        self.treeJson = serde_json::to_string(&self.tree).unwrap_or_default().into();
    }
}

fn spans_html(hl: &Highlighter, syntax: &syntect::parsing::SyntaxReference, text: &str) -> String {
    let spans = hl.line(syntax, text);
    if spans.is_empty() {
        return "&nbsp;".into();
    }
    let mut out = String::new();
    for s in spans {
        out.push_str(&format!(
            "<span style=\"color:#{:02x}{:02x}{:02x}\">{}</span>",
            s.color.0,
            s.color.1,
            s.color.2,
            html_escape(&s.text)
        ));
    }
    out
}

/// Escape for Qt RichText, preserving leading/embedded spaces so code indentation survives.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace(' ', "&nbsp;")
        .replace('\t', "&nbsp;&nbsp;&nbsp;&nbsp;")
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

/// Best-effort desktop light/dark detection.
///
/// Qt 6.4's QML `Qt.styleHints.colorScheme` does not exist (added in 6.5), so we decide the
/// scheme here on the Rust side and expose it as the `dark` property. We honour an explicit
/// `GIT_REVIEW_DARK` override, then fall back to common desktop signals. In a headless
/// environment with no desktop preference, this yields **light** (per the spec).
fn detect_dark() -> bool {
    if let Ok(v) = std::env::var("GIT_REVIEW_DARK") {
        return matches!(v.as_str(), "1" | "true" | "yes" | "dark");
    }
    // GNOME / freedesktop hint sometimes exported into the environment.
    if let Ok(v) = std::env::var("GTK_THEME") {
        if v.to_ascii_lowercase().contains("dark") {
            return true;
        }
    }
    if let Ok(v) = std::env::var("QT_QPA_PLATFORMTHEME") {
        if v.to_ascii_lowercase().contains("dark") {
            return true;
        }
    }
    false
}

qrc!(register_qml,
    "qml" as "ui" {
        "main.qml",
    }
);

fn main() {
    let path = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("GIT_REVIEW_REPO").ok())
        .unwrap_or_else(|| ".".to_string());

    let repo = match Repo::open(&path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Could not open a git repository at '{path}': {e}");
            std::process::exit(1);
        }
    };

    let mut backend = Backend::default();
    backend.dark = detect_dark();
    backend.repoName = repo.workdir_name().into();
    backend.infos = repo.commits(500).unwrap_or_default();
    backend.showing = backend
        .infos
        .iter()
        .find_map(|c| c.oid.map(Showing::Commit))
        .unwrap_or(Showing::Working);
    backend.showSpace = true;
    backend.lineNumbers = true;
    backend.wrap = false;
    backend.font = 12;
    backend.hl = Some(Highlighter::new());
    backend.repo = Some(repo);
    backend.rebuild();

    register_qml();
    let mut engine = QmlEngine::new();
    let backend = QObjectBox::new(backend);
    engine.set_object_property("backend".into(), backend.pinned());
    engine.load_file("qrc:/ui/main.qml".into());
    engine.exec();
}
