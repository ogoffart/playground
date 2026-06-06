//! Render-friendly, `Clone`-able view model.
//!
//! `git.rs` / `highlight.rs` produce the framework-agnostic data; this module flattens a `DiffSet`
//! (plus per-line syntax highlighting) into plain owned structs that live happily inside Dioxus
//! signals and are cheap to clone into RSX.

use crate::git::{ChangeKind, DiffSet, LineKind};
use crate::highlight::Highlighter;

/// One coloured piece of a code line, ready to become a `<span style=color:...>`.
#[derive(Clone, PartialEq)]
pub struct RSpan {
    pub color: String, // "rgb(r,g,b)"
    pub bold: bool,
    pub italic: bool,
    pub text: String,
}

#[derive(Clone, PartialEq)]
pub struct RLine {
    pub kind: LineKind,
    /// Hunk-header rows (the `@@ ... @@` separators) render in the accent style.
    pub is_hunk_header: bool,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub spans: Vec<RSpan>,
}

#[derive(Clone, PartialEq)]
pub struct RFile {
    pub path: String,
    pub header_label: String,
    pub kind_letter: char,
    pub icon: &'static str,
    pub added: u32,
    pub removed: u32,
    pub binary: bool,
    pub lines: Vec<RLine>,
}

#[derive(Clone, PartialEq)]
pub struct RMessage {
    pub short: String,
    pub author: String,
    pub date: String,
    pub title: String,
    pub body: String,
}

/// The whole main-view payload, precomputed once per recompute.
#[derive(Clone, PartialEq)]
pub struct RenderDiff {
    pub summary: String,
    pub added: u32,
    pub removed: u32,
    pub message: Option<RMessage>,
    pub files: Vec<RFile>,
    pub error: Option<String>,
}

impl RenderDiff {
    pub fn empty() -> Self {
        Self {
            summary: String::new(),
            added: 0,
            removed: 0,
            message: None,
            files: Vec::new(),
            error: None,
        }
    }

    pub fn error(msg: String) -> Self {
        let mut d = Self::empty();
        d.error = Some(msg);
        d
    }

    /// Flatten a `DiffSet`, running syntect over every (non-hunk-header) line.
    pub fn build(set: DiffSet, hl: &Highlighter) -> Self {
        let message = set.message.map(|m| RMessage {
            short: m.short,
            author: m.author,
            date: m.date,
            title: m.title,
            body: m.body,
        });

        let mut files = Vec::with_capacity(set.files.len());
        for f in set.files {
            let header_label = match (&f.old_path, f.kind) {
                (Some(old), ChangeKind::Renamed) => format!("{old}  →  {}", f.path),
                _ => f.path.clone(),
            };
            let icon = file_icon(&f.path);
            let mut lines = Vec::new();
            if !f.binary {
                let syntax = hl.syntax_for(&f.path);
                for hunk in &f.hunks {
                    lines.push(RLine {
                        kind: LineKind::Context,
                        is_hunk_header: true,
                        old_no: None,
                        new_no: None,
                        spans: vec![RSpan {
                            color: "rgb(101,109,118)".into(),
                            bold: false,
                            italic: false,
                            text: hunk.header.clone(),
                        }],
                    });
                    for line in &hunk.lines {
                        let spans = hl
                            .line(syntax, &line.text)
                            .into_iter()
                            .map(|s| RSpan {
                                color: format!("rgb({},{},{})", s.color.0, s.color.1, s.color.2),
                                bold: s.bold,
                                italic: s.italic,
                                text: s.text,
                            })
                            .collect::<Vec<_>>();
                        lines.push(RLine {
                            kind: line.kind,
                            is_hunk_header: false,
                            old_no: line.old_no,
                            new_no: line.new_no,
                            spans,
                        });
                    }
                }
            }
            files.push(RFile {
                path: f.path,
                header_label,
                kind_letter: f.kind.letter(),
                icon,
                added: f.added,
                removed: f.removed,
                binary: f.binary,
                lines,
            });
        }

        Self {
            summary: set.summary,
            added: set.added,
            removed: set.removed,
            message,
            files,
            error: None,
        }
    }
}

pub fn file_icon(path: &str) -> &'static str {
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
