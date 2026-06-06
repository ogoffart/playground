//! C ABI bridge between the Rust data layer (git2 + syntect) and the C++ Qt Widgets shim.
//!
//! The Rust side owns a single [`Repo`] + [`Highlighter`] behind a global. The C++ shim builds the
//! whole UI and, on user interaction, calls these functions to fetch JSON it then renders into
//! widgets. JSON keeps the surface tiny and avoids hand-rolling structs across the boundary.

use std::ffi::{c_char, CStr, CString};
use std::sync::Mutex;

use git2::Oid;
use serde::Serialize;

use crate::git::{ChangeKind, DiffSet, LineKind, Repo};
use crate::highlight::Highlighter;

struct State {
    repo: Repo,
    hl: Highlighter,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn cstr(s: String) -> *mut c_char {
    CString::new(s).unwrap_or_default().into_raw()
}

/// Free a string previously returned by this module. Called from C++.
#[no_mangle]
pub extern "C" fn gr_free(p: *mut c_char) {
    if !p.is_null() {
        unsafe {
            let _ = CString::from_raw(p);
        }
    }
}

/// Open the repository at `path`. Returns 1 on success, 0 on failure.
#[no_mangle]
pub extern "C" fn gr_open(path: *const c_char) -> i32 {
    let path = unsafe { CStr::from_ptr(path) }.to_string_lossy().into_owned();
    match Repo::open(&path) {
        Ok(repo) => {
            *STATE.lock().unwrap() = Some(State {
                repo,
                hl: Highlighter::new(),
            });
            1
        }
        Err(e) => {
            eprintln!("git-review: failed to open repo at {path}: {e}");
            0
        }
    }
}

// ---- JSON serialization models (UI-facing) --------------------------------

#[derive(Serialize)]
struct CommitJson {
    oid: String, // "" for working tree
    short: String,
    author: String,
    date: String,
    title: String,
    working: bool,
}

#[derive(Serialize)]
struct SpanJson {
    text: String,
    color: String, // "#rrggbb"
    bold: bool,
    italic: bool,
}

#[derive(Serialize)]
struct LineJson {
    kind: u8, // 0 context, 1 added, 2 removed
    old_no: i64, // -1 when absent
    new_no: i64,
    spans: Vec<SpanJson>,
}

#[derive(Serialize)]
struct HunkJson {
    header: String,
    lines: Vec<LineJson>,
}

#[derive(Serialize)]
struct FileJson {
    path: String,
    old_path: String,
    kind: char,
    added: u32,
    removed: u32,
    binary: bool,
    hunks: Vec<HunkJson>,
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
    summary: String,
    added: u32,
    removed: u32,
    has_message: bool,
    message: MessageJson,
    files: Vec<FileJson>,
}

fn line_kind_code(k: LineKind) -> u8 {
    match k {
        LineKind::Context => 0,
        LineKind::Added => 1,
        LineKind::Removed => 2,
    }
}

fn change_letter(k: ChangeKind) -> char {
    k.letter()
}

/// Window title (repo name).
#[no_mangle]
pub extern "C" fn gr_repo_name() -> *mut c_char {
    let g = STATE.lock().unwrap();
    let name = g
        .as_ref()
        .map(|s| s.repo.workdir_name())
        .unwrap_or_else(|| "repository".into());
    cstr(name)
}

/// JSON array of commit rows (working tree first).
#[no_mangle]
pub extern "C" fn gr_commits() -> *mut c_char {
    let g = STATE.lock().unwrap();
    let Some(state) = g.as_ref() else {
        return cstr("[]".into());
    };
    let commits = state.repo.commits(500).unwrap_or_default();
    let list: Vec<CommitJson> = commits
        .into_iter()
        .map(|c| CommitJson {
            oid: c.oid.map(|o| o.to_string()).unwrap_or_default(),
            short: c.short,
            author: c.author,
            date: c.date,
            title: c.title,
            working: c.oid.is_none(),
        })
        .collect();
    cstr(serde_json::to_string(&list).unwrap_or_else(|_| "[]".into()))
}

fn ignore_ws_from(flag: i32) -> bool {
    // show_space_changes == 0  => ignore whitespace.
    flag == 0
}

/// Diff JSON for the working tree. `show_space_changes`: 1 to show ws diffs, 0 to ignore.
#[no_mangle]
pub extern "C" fn gr_working(show_space_changes: i32) -> *mut c_char {
    diff_json(|s| s.repo.working_tree(ignore_ws_from(show_space_changes)))
}

/// Diff JSON for a single commit (git show).
#[no_mangle]
pub extern "C" fn gr_show(oid: *const c_char, show_space_changes: i32) -> *mut c_char {
    let oid = unsafe { CStr::from_ptr(oid) }.to_string_lossy().into_owned();
    let Ok(oid) = Oid::from_str(&oid) else {
        return cstr("null".into());
    };
    diff_json(|s| s.repo.show(oid, ignore_ws_from(show_space_changes)))
}

/// Diff JSON for a range from..to.
#[no_mangle]
pub extern "C" fn gr_range(
    from: *const c_char,
    to: *const c_char,
    show_space_changes: i32,
) -> *mut c_char {
    let from = unsafe { CStr::from_ptr(from) }.to_string_lossy().into_owned();
    let to = unsafe { CStr::from_ptr(to) }.to_string_lossy().into_owned();
    let (Ok(from), Ok(to)) = (Oid::from_str(&from), Oid::from_str(&to)) else {
        return cstr("null".into());
    };
    diff_json(|s| s.repo.range(from, to, ignore_ws_from(show_space_changes)))
}

fn diff_json(
    f: impl FnOnce(&State) -> Result<DiffSet, git2::Error>,
) -> *mut c_char {
    let g = STATE.lock().unwrap();
    let Some(state) = g.as_ref() else {
        return cstr("null".into());
    };
    let ds = match f(state) {
        Ok(ds) => ds,
        Err(e) => {
            eprintln!("git-review: diff error: {e}");
            return cstr("null".into());
        }
    };
    cstr(serialize_diff(state, &ds))
}

fn serialize_diff(state: &State, ds: &DiffSet) -> String {
    let mut files = Vec::with_capacity(ds.files.len());
    for fd in &ds.files {
        let syntax = state.hl.syntax_for(&fd.path);
        let mut hunks = Vec::with_capacity(fd.hunks.len());
        for h in &fd.hunks {
            let mut lines = Vec::with_capacity(h.lines.len());
            for dl in &h.lines {
                let spans = if fd.binary {
                    Vec::new()
                } else {
                    state
                        .hl
                        .line(syntax, &dl.text)
                        .into_iter()
                        .map(|sp| SpanJson {
                            text: sp.text,
                            color: format!("#{:02x}{:02x}{:02x}", sp.color.0, sp.color.1, sp.color.2),
                            bold: sp.bold,
                            italic: sp.italic,
                        })
                        .collect()
                };
                lines.push(LineJson {
                    kind: line_kind_code(dl.kind),
                    old_no: dl.old_no.map(|n| n as i64).unwrap_or(-1),
                    new_no: dl.new_no.map(|n| n as i64).unwrap_or(-1),
                    spans,
                });
            }
            hunks.push(HunkJson {
                header: h.header.clone(),
                lines,
            });
        }
        files.push(FileJson {
            path: fd.path.clone(),
            old_path: fd.old_path.clone().unwrap_or_default(),
            kind: change_letter(fd.kind),
            added: fd.added,
            removed: fd.removed,
            binary: fd.binary,
            hunks,
        });
    }

    let (has_message, message) = match &ds.message {
        Some(m) => (
            true,
            MessageJson {
                short: m.short.clone(),
                author: m.author.clone(),
                date: m.date.clone(),
                title: m.title.clone(),
                body: m.body.clone(),
            },
        ),
        None => (
            false,
            MessageJson {
                short: String::new(),
                author: String::new(),
                date: String::new(),
                title: String::new(),
                body: String::new(),
            },
        ),
    };

    let out = DiffJson {
        summary: ds.summary.clone(),
        added: ds.added,
        removed: ds.removed,
        has_message,
        message,
        files,
    };
    serde_json::to_string(&out).unwrap_or_else(|_| "null".into())
}
