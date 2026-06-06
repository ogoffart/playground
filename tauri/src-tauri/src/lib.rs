//! Tauri backend for git-review.
//!
//! The framework-agnostic git/diff/highlight model lives in `git.rs` and `highlight.rs`
//! (copied verbatim from the egui reference app). This module wraps that model in
//! serde-serializable JSON structs and exposes them as `#[tauri::command]`s that the
//! plain HTML/CSS/JS frontend calls over IPC. Syntax highlighting is done here in Rust
//! (syntect spans -> `{text, color}` arrays) so the frontend stays a dumb renderer.

mod git;
mod highlight;

use std::sync::Mutex;

use git2::Oid;
use serde::Serialize;

use git::{DiffSet, LineKind, Repo};
use highlight::Highlighter;

// --- JSON model returned to the frontend ------------------------------------

#[derive(Serialize)]
struct CommitJson {
    /// Full hex oid, or `null` for the synthetic working-tree row.
    oid: Option<String>,
    short: String,
    author: String,
    date: String,
    title: String,
    is_working_tree: bool,
}

#[derive(Serialize)]
struct SpanJson {
    text: String,
    /// CSS hex color like `#1f2328`.
    color: String,
    bold: bool,
    italic: bool,
}

#[derive(Serialize)]
struct LineJson {
    /// "context" | "added" | "removed"
    kind: &'static str,
    old_no: Option<u32>,
    new_no: Option<u32>,
    /// Hunk-header rows render the raw text in the accent color, no highlighting.
    is_hunk_header: bool,
    spans: Vec<SpanJson>,
}

#[derive(Serialize)]
struct FileJson {
    path: String,
    old_path: Option<String>,
    /// "A" | "D" | "M" | "R"
    kind: char,
    added: u32,
    removed: u32,
    binary: bool,
    lines: Vec<LineJson>,
}

#[derive(Serialize)]
struct MessageJson {
    short: String,
    author: String,
    date: String,
    title: String,
    body: String,
}

#[derive(Serialize)]
struct DiffJson {
    message: Option<MessageJson>,
    files: Vec<FileJson>,
    added: u32,
    removed: u32,
    summary: String,
}

// --- shared app state -------------------------------------------------------

struct AppState {
    repo: Mutex<Repo>,
    hl: Highlighter,
    repo_name: String,
}

fn kind_str(k: LineKind) -> &'static str {
    match k {
        LineKind::Context => "context",
        LineKind::Added => "added",
        LineKind::Removed => "removed",
    }
}

fn hex(rgb: (u8, u8, u8)) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb.0, rgb.1, rgb.2)
}

/// Turn a model `DiffSet` into the serde JSON model, highlighting code lines.
fn diff_to_json(hl: &Highlighter, d: DiffSet) -> DiffJson {
    let files = d
        .files
        .into_iter()
        .map(|f| {
            let syntax = hl.syntax_for(&f.path);
            let mut lines: Vec<LineJson> = Vec::new();
            if !f.binary {
                for hunk in &f.hunks {
                    lines.push(LineJson {
                        kind: "context",
                        old_no: None,
                        new_no: None,
                        is_hunk_header: true,
                        spans: vec![SpanJson {
                            text: hunk.header.clone(),
                            color: hex((0x57, 0x60, 0x6a)),
                            bold: false,
                            italic: false,
                        }],
                    });
                    for line in &hunk.lines {
                        let spans = hl
                            .line(syntax, &line.text)
                            .into_iter()
                            .map(|s| SpanJson {
                                text: s.text,
                                color: hex(s.color),
                                bold: s.bold,
                                italic: s.italic,
                            })
                            .collect();
                        lines.push(LineJson {
                            kind: kind_str(line.kind),
                            old_no: line.old_no,
                            new_no: line.new_no,
                            is_hunk_header: false,
                            spans,
                        });
                    }
                }
            }
            FileJson {
                path: f.path,
                old_path: f.old_path,
                kind: f.kind.letter(),
                added: f.added,
                removed: f.removed,
                binary: f.binary,
                lines,
            }
        })
        .collect();

    DiffJson {
        message: d.message.map(|m| MessageJson {
            short: m.short,
            author: m.author,
            date: m.date,
            title: m.title,
            body: m.body,
        }),
        files,
        added: d.added,
        removed: d.removed,
        summary: d.summary,
    }
}

fn parse_oid(s: &str) -> Result<Oid, String> {
    Oid::from_str(s).map_err(|e| e.message().to_string())
}

// --- tauri commands ---------------------------------------------------------

#[tauri::command]
fn repo_name(state: tauri::State<AppState>) -> String {
    state.repo_name.clone()
}

#[tauri::command]
fn commits(state: tauri::State<AppState>) -> Result<Vec<CommitJson>, String> {
    let repo = state.repo.lock().unwrap();
    let list = repo.commits(500).map_err(|e| e.message().to_string())?;
    Ok(list
        .into_iter()
        .map(|c| CommitJson {
            is_working_tree: c.is_working_tree(),
            oid: c.oid.map(|o| o.to_string()),
            short: c.short,
            author: c.author,
            date: c.date,
            title: c.title,
        })
        .collect())
}

/// Query a diff. `mode` is one of:
///   "working"        -> working tree vs HEAD
///   "commit" + a     -> `git show a`
///   "range"  + a + b -> `git diff a b`
#[tauri::command]
fn diff(
    state: tauri::State<AppState>,
    mode: String,
    a: Option<String>,
    b: Option<String>,
    ignore_ws: bool,
) -> Result<DiffJson, String> {
    let repo = state.repo.lock().unwrap();
    let set = match mode.as_str() {
        "working" => repo.working_tree(ignore_ws),
        "commit" => {
            let oid = parse_oid(a.as_deref().ok_or("missing commit oid")?)?;
            repo.show(oid, ignore_ws)
        }
        "range" => {
            let from = parse_oid(a.as_deref().ok_or("missing from oid")?)?;
            let to = parse_oid(b.as_deref().ok_or("missing to oid")?)?;
            repo.range(from, to, ignore_ws)
        }
        other => return Err(format!("unknown diff mode: {other}")),
    }
    .map_err(|e| e.message().to_string())?;
    Ok(diff_to_json(&state.hl, set))
}

/// Resolve the repository path from argv[1], `$GIT_REVIEW_REPO`, or the cwd.
fn repo_path() -> std::path::PathBuf {
    if let Some(arg) = std::env::args().nth(1) {
        return std::path::PathBuf::from(arg);
    }
    if let Ok(env) = std::env::var("GIT_REVIEW_REPO") {
        return std::path::PathBuf::from(env);
    }
    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let path = repo_path();
    let repo = match Repo::open(&path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("git-review: failed to open repository at {path:?}: {e}");
            std::process::exit(1);
        }
    };
    let name = repo.workdir_name();
    let state = AppState {
        repo: Mutex::new(repo),
        hl: Highlighter::new(),
        repo_name: name,
    };

    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![repo_name, commits, diff])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
