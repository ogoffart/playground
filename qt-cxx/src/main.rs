//! git-review — a GitHub-style code review desktop app, built with **cxx-qt** (KDAB's Qt/Rust
//! bridge).
//!
//! Compared to the qmetaobject-rs sibling app, the QObject here is declared through a
//! `#[cxx_qt::bridge]` module: properties are listed as `#[qproperty(...)]` on the QObject type,
//! slots are `#[qinvokable]` free functions, and CXX-Qt generates the C++ QObject + MOC glue at
//! build time. The inner Rust state lives in a plain `BackendRust` struct. The model is still
//! produced by the shared `git`/`highlight` modules and serialized to JSON strings consumed by the
//! same QML UI.
//!
//! Usage: `git-review-cxxqt [path-to-repo]` (defaults to cwd / `$GIT_REVIEW_REPO`).
#![allow(clippy::too_many_lines)]

mod git;
mod highlight;

use std::sync::Mutex;

use cxx_qt::CxxQtType;
use git2::Oid;
use serde::Serialize;

use git::{CommitInfo, DiffSet, LineKind, Repo};
use highlight::Highlighter;

/// The repository path is resolved in `main` (from argv / env / cwd) and read by the QObject's
/// `initialize()` hook, since QML constructs the object and cannot pass constructor arguments.
static REPO_PATH: Mutex<Option<String>> = Mutex::new(None);

#[derive(Clone, Copy, PartialEq, Default)]
enum Showing {
    #[default]
    Working,
    Commit(Oid),
    Range(Oid, Oid),
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

/// The CXX-Qt bridge: declares the `Backend` QObject, its Q_PROPERTYs, signals, and slots.
#[cxx_qt::bridge]
mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, commits_json, cxx_name = "commitsJson")]
        #[qproperty(QString, files_json, cxx_name = "filesJson")]
        #[qproperty(QString, diff_json, cxx_name = "diffJson")]
        #[qproperty(QString, summary)]
        #[qproperty(i32, total_added, cxx_name = "totalAdded")]
        #[qproperty(i32, total_removed, cxx_name = "totalRemoved")]
        #[qproperty(QString, repo_name, cxx_name = "repoName")]
        #[qproperty(bool, wrap)]
        #[qproperty(bool, show_space, cxx_name = "showSpace")]
        #[qproperty(bool, line_numbers, cxx_name = "lineNumbers")]
        #[qproperty(i32, font)]
        type Backend = super::BackendRust;
    }

    extern "RustQt" {
        #[qinvokable]
        fn select_commit(self: Pin<&mut Backend>, i: i32);
        #[qinvokable]
        fn set_from(self: Pin<&mut Backend>, i: i32);
        #[qinvokable]
        fn set_to(self: Pin<&mut Backend>, i: i32);
        #[qinvokable]
        fn toggle_wrap(self: Pin<&mut Backend>);
        #[qinvokable]
        fn toggle_space(self: Pin<&mut Backend>);
        #[qinvokable]
        fn toggle_line_numbers(self: Pin<&mut Backend>);
        #[qinvokable]
        fn font_inc(self: Pin<&mut Backend>);
        #[qinvokable]
        fn font_dec(self: Pin<&mut Backend>);
    }

    // Run Rust code right after the QObject is constructed (opens the repo, builds the model).
    impl cxx_qt::Initialize for Backend {}
}

/// The inner Rust state of the `Backend` QObject. Q_PROPERTY storage lives here too (the
/// `#[qproperty]` attributes above name these fields).
pub struct BackendRust {
    // --- property storage (mirrors the #[qproperty] list) ---
    commits_json: cxx_qt_lib::QString,
    files_json: cxx_qt_lib::QString,
    diff_json: cxx_qt_lib::QString,
    summary: cxx_qt_lib::QString,
    total_added: i32,
    total_removed: i32,
    repo_name: cxx_qt_lib::QString,
    wrap: bool,
    show_space: bool,
    line_numbers: bool,
    font: i32,

    // --- non-Qt state ---
    repo: Option<Repo>,
    hl: Option<Highlighter>,
    infos: Vec<CommitInfo>,
    showing: Showing,
    from: Option<Oid>,
    to: Option<Oid>,
}

impl Default for BackendRust {
    fn default() -> Self {
        Self {
            commits_json: cxx_qt_lib::QString::default(),
            files_json: cxx_qt_lib::QString::default(),
            diff_json: cxx_qt_lib::QString::default(),
            summary: cxx_qt_lib::QString::default(),
            total_added: 0,
            total_removed: 0,
            repo_name: cxx_qt_lib::QString::default(),
            wrap: false,
            show_space: true,
            line_numbers: true,
            font: 12,
            repo: None,
            hl: None,
            infos: Vec::new(),
            showing: Showing::Working,
            from: None,
            to: None,
        }
    }
}

impl cxx_qt::Initialize for qobject::Backend {
    fn initialize(mut self: core::pin::Pin<&mut Self>) {
        let path = REPO_PATH
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .unwrap_or_else(|| ".".to_string());

        let repo = match Repo::open(&path) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Could not open a git repository at '{path}': {e}");
                std::process::exit(1);
            }
        };

        let infos = repo.commits(500).unwrap_or_default();
        let showing = infos
            .iter()
            .find_map(|c| c.oid.map(Showing::Commit))
            .unwrap_or(Showing::Working);

        let name = repo.workdir_name();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.infos = infos;
            rust.showing = showing;
            rust.hl = Some(Highlighter::new());
            rust.repo = Some(repo);
        }
        self.as_mut().set_repo_name(cxx_qt_lib::QString::from(&name));
        self.rebuild();
    }
}

impl qobject::Backend {
    fn maybe_range(self: core::pin::Pin<&mut Self>) {
        let mut rust = self.rust_mut();
        if let (Some(a), Some(b)) = (rust.from, rust.to) {
            rust.showing = Showing::Range(a, b);
        }
    }

    fn select_commit(mut self: core::pin::Pin<&mut Self>, i: i32) {
        let oid = self.rust().infos.get(i as usize).and_then(|c| c.oid);
        self.as_mut().rust_mut().showing = match oid {
            Some(o) => Showing::Commit(o),
            None => Showing::Working,
        };
        self.rebuild();
    }

    fn set_from(mut self: core::pin::Pin<&mut Self>, i: i32) {
        let oid = self.rust().infos.get(i as usize).and_then(|c| c.oid);
        if let Some(o) = oid {
            self.as_mut().rust_mut().from = Some(o);
            self.as_mut().maybe_range();
        }
        self.rebuild();
    }

    fn set_to(mut self: core::pin::Pin<&mut Self>, i: i32) {
        let oid = self.rust().infos.get(i as usize).and_then(|c| c.oid);
        if let Some(o) = oid {
            self.as_mut().rust_mut().to = Some(o);
            self.as_mut().maybe_range();
        }
        self.rebuild();
    }

    fn toggle_wrap(mut self: core::pin::Pin<&mut Self>) {
        let v = !*self.as_ref().wrap();
        self.as_mut().set_wrap(v);
    }

    fn toggle_space(mut self: core::pin::Pin<&mut Self>) {
        let v = !*self.as_ref().show_space();
        self.as_mut().set_show_space(v);
        self.rebuild();
    }

    fn toggle_line_numbers(mut self: core::pin::Pin<&mut Self>) {
        let v = !*self.as_ref().line_numbers();
        self.as_mut().set_line_numbers(v);
    }

    fn font_inc(mut self: core::pin::Pin<&mut Self>) {
        let v = (*self.as_ref().font() + 1).min(28);
        self.as_mut().set_font(v);
    }

    fn font_dec(mut self: core::pin::Pin<&mut Self>) {
        let v = (*self.as_ref().font() - 1).max(8);
        self.as_mut().set_font(v);
    }

    /// Recompute the JSON model from the current `showing`/settings and push it to the properties
    /// (each `set_*` auto-emits the matching `*_changed` signal the QML bindings listen on).
    fn rebuild(mut self: core::pin::Pin<&mut Self>) {
        let ignore_ws = !*self.as_ref().show_space();

        // ---- commit list ----
        let commits: Vec<CommitJson> = {
            let this = self.as_ref();
            let r = this.rust();
            r.infos
                .iter()
                .map(|c| {
                    let current = match (r.showing, c.oid) {
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
                        is_from: c.oid.is_some() && c.oid == r.from,
                        is_to: c.oid.is_some() && c.oid == r.to,
                        working: c.is_working_tree(),
                    }
                })
                .collect()
        };
        let commits_json = serde_json::to_string(&commits).unwrap_or_default();

        // ---- diff set ----
        let diff: DiffSet = {
            let this = self.as_ref();
            let r = this.rust();
            let repo = r.repo.as_ref().unwrap();
            match r.showing {
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
            })
        };

        let summary = diff.summary.clone();
        let total_added = diff.added as i32;
        let total_removed = diff.removed as i32;

        // ---- diff items + file list ----
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
        {
            let this = self.as_ref();
            let r = this.rust();
            let hl = r.hl.as_ref().unwrap();
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
        }

        let files_json = serde_json::to_string(&files).unwrap_or_default();
        let diff_json = serde_json::to_string(&items).unwrap_or_default();

        // ---- push to the QObject's properties (auto-emits *_changed) ----
        self.as_mut()
            .set_commits_json(cxx_qt_lib::QString::from(&commits_json));
        self.as_mut()
            .set_files_json(cxx_qt_lib::QString::from(&files_json));
        self.as_mut()
            .set_diff_json(cxx_qt_lib::QString::from(&diff_json));
        self.as_mut().set_summary(cxx_qt_lib::QString::from(&summary));
        self.as_mut().set_total_added(total_added);
        self.as_mut().set_total_removed(total_removed);
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

fn main() {
    let path = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("GIT_REVIEW_REPO").ok())
        .unwrap_or_else(|| ".".to_string());
    *REPO_PATH.lock().unwrap() = Some(path);

    // Standard CXX-Qt QML startup: create the app + engine, load the QML module's main.qml.
    let mut app = cxx_qt_lib::QGuiApplication::new();
    let mut engine = cxx_qt_lib::QQmlApplicationEngine::new();

    if let Some(engine) = engine.as_mut() {
        engine.load(&cxx_qt_lib::QUrl::from(
            "qrc:/qt/qml/com/example/gitreview/qml/main.qml",
        ));
    }

    if let Some(app) = app.as_mut() {
        app.exec();
    }
}
