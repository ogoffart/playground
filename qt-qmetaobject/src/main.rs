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
    diffJson: qt_property!(QString; NOTIFY updated),
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

    // --- non-Qt state ---
    repo: Option<Repo>,
    hl: Option<Highlighter>,
    infos: Vec<CommitInfo>,
    showing: Showing,
    from: Option<Oid>,
    to: Option<Oid>,
}

impl Backend {
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
