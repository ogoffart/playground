//! Git access and diff model built on top of `git2` (libgit2).
//!
//! This module is intentionally framework-agnostic: it produces a plain data model
//! (`CommitInfo`, `FileDiff`, `Hunk`, `DiffLine`) that the UI layer renders. The same module is
//! re-implemented, near-identically, in each framework app in this repo.

use std::path::Path;

use git2::{Delta, DiffOptions, Oid, Repository};

/// One row in the commit list. The first row in the list is a synthetic "working tree" entry.
#[derive(Clone)]
pub struct CommitInfo {
    pub oid: Option<Oid>, // None => working tree / uncommitted changes
    pub short: String,
    pub author: String,
    pub date: String,
    pub title: String,
}

impl CommitInfo {
    pub fn is_working_tree(&self) -> bool {
        self.oid.is_none()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Deleted,
    Modified,
    Renamed,
}

impl ChangeKind {
    pub fn letter(self) -> char {
        match self {
            ChangeKind::Added => 'A',
            ChangeKind::Deleted => 'D',
            ChangeKind::Modified => 'M',
            ChangeKind::Renamed => 'R',
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

#[derive(Clone)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
}

#[derive(Clone)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone)]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub kind: ChangeKind,
    pub added: u32,
    pub removed: u32,
    pub hunks: Vec<Hunk>,
    pub binary: bool,
}

/// The full result shown in the main view.
pub struct DiffSet {
    /// Present only when showing a single commit.
    pub message: Option<CommitMessage>,
    pub files: Vec<FileDiff>,
    pub added: u32,
    pub removed: u32,
    /// Human summary, e.g. `git show a1b2c3d` or `git diff a1b2c3d e4f5a6b`.
    pub summary: String,
}

#[derive(Clone)]
pub struct CommitMessage {
    pub short: String,
    pub author: String,
    pub date: String,
    pub title: String,
    pub body: String,
}

pub struct Repo {
    repo: Repository,
}

impl Repo {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, git2::Error> {
        let repo = Repository::discover(path)?;
        Ok(Self { repo })
    }

    pub fn workdir_name(&self) -> String {
        self.repo
            .workdir()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "repository".into())
    }

    /// The commit list: a synthetic working-tree row, then commits reachable from HEAD.
    pub fn commits(&self, limit: usize) -> Result<Vec<CommitInfo>, git2::Error> {
        let mut out = vec![CommitInfo {
            oid: None,
            short: "·······".into(),
            author: "You".into(),
            date: "now".into(),
            title: "Uncommitted changes (working tree)".into(),
        }];

        if self.repo.head().is_err() {
            // Unborn branch / empty repo: only the working-tree row.
            return Ok(out);
        }

        let mut walk = self.repo.revwalk()?;
        walk.push_head()?;
        walk.set_sorting(git2::Sort::TIME)?;
        for oid in walk.take(limit).flatten() {
            let commit = self.repo.find_commit(oid)?;
            let title = commit
                .summary()
                .unwrap_or("(no message)")
                .trim()
                .to_string();
            out.push(CommitInfo {
                oid: Some(oid),
                short: short_oid(&oid),
                author: commit.author().name().unwrap_or("unknown").to_string(),
                date: format_time(commit.time().seconds()),
                title,
            });
        }
        Ok(out)
    }

    /// Build the diff for a single commit (vs. its first parent), including the message.
    pub fn show(&self, oid: Oid, ignore_ws: bool) -> Result<DiffSet, git2::Error> {
        let commit = self.repo.find_commit(oid)?;
        let new_tree = commit.tree()?;
        let old_tree = commit.parent(0).ok().map(|p| p.tree()).transpose()?;

        let mut opts = diff_opts(ignore_ws);
        let diff = self.repo.diff_tree_to_tree(
            old_tree.as_ref(),
            Some(&new_tree),
            Some(&mut opts),
        )?;
        let (files, added, removed) = collect(&diff);

        let msg = CommitMessage {
            short: short_oid(&oid),
            author: format!(
                "{} <{}>",
                commit.author().name().unwrap_or("unknown"),
                commit.author().email().unwrap_or("")
            ),
            date: format_time(commit.time().seconds()),
            title: commit.summary().unwrap_or("").to_string(),
            body: commit
                .body()
                .map(|b| b.trim().to_string())
                .unwrap_or_default(),
        };

        Ok(DiffSet {
            message: Some(msg),
            files,
            added,
            removed,
            summary: format!("git show {}", short_oid(&oid)),
        })
    }

    /// Diff a range `from..to` (two tree endpoints).
    pub fn range(&self, from: Oid, to: Oid, ignore_ws: bool) -> Result<DiffSet, git2::Error> {
        let from_tree = self.repo.find_commit(from)?.tree()?;
        let to_tree = self.repo.find_commit(to)?.tree()?;
        let mut opts = diff_opts(ignore_ws);
        let diff = self.repo.diff_tree_to_tree(
            Some(&from_tree),
            Some(&to_tree),
            Some(&mut opts),
        )?;
        let (files, added, removed) = collect(&diff);
        Ok(DiffSet {
            message: None,
            files,
            added,
            removed,
            summary: format!("git diff {} {}", short_oid(&from), short_oid(&to)),
        })
    }

    /// Diff the working tree (incl. staged + unstaged) against HEAD.
    pub fn working_tree(&self, ignore_ws: bool) -> Result<DiffSet, git2::Error> {
        let head_tree = self
            .repo
            .head()
            .ok()
            .and_then(|h| h.peel_to_tree().ok());
        let mut opts = diff_opts(ignore_ws);
        opts.include_untracked(true).recurse_untracked_dirs(true);
        let diff = self
            .repo
            .diff_tree_to_workdir_with_index(head_tree.as_ref(), Some(&mut opts))?;
        let (files, added, removed) = collect(&diff);
        Ok(DiffSet {
            message: None,
            files,
            added,
            removed,
            summary: "working tree (uncommitted changes)".into(),
        })
    }
}

fn diff_opts(ignore_ws: bool) -> DiffOptions {
    let mut opts = DiffOptions::new();
    opts.context_lines(3);
    if ignore_ws {
        opts.ignore_whitespace(true);
        opts.ignore_whitespace_change(true);
        opts.ignore_whitespace_eol(true);
    }
    opts
}

fn kind_of(delta: Delta) -> ChangeKind {
    match delta {
        Delta::Added | Delta::Copied | Delta::Untracked => ChangeKind::Added,
        Delta::Deleted => ChangeKind::Deleted,
        Delta::Renamed => ChangeKind::Renamed,
        _ => ChangeKind::Modified,
    }
}

/// Walk a `git2::Diff` into our `FileDiff` model via the line/hunk callbacks.
///
/// `git2::Diff::foreach` takes four independent `FnMut` closures, so they share the accumulator
/// through interior mutability (`RefCell`/`Cell`) rather than a single `&mut` borrow.
fn collect(diff: &git2::Diff) -> (Vec<FileDiff>, u32, u32) {
    use std::cell::{Cell, RefCell};
    let files: RefCell<Vec<FileDiff>> = RefCell::new(Vec::new());
    let total_added = Cell::new(0u32);
    let total_removed = Cell::new(0u32);

    let _ = diff.foreach(
        &mut |delta, _progress| {
            let new_path = delta
                .new_file()
                .path()
                .map(|p| p.to_string_lossy().into_owned());
            let old_path = delta
                .old_file()
                .path()
                .map(|p| p.to_string_lossy().into_owned());
            let path = new_path
                .clone()
                .or_else(|| old_path.clone())
                .unwrap_or_else(|| "<unknown>".into());
            files.borrow_mut().push(FileDiff {
                path,
                old_path: old_path.filter(|o| Some(o) != new_path.as_ref()),
                kind: kind_of(delta.status()),
                added: 0,
                removed: 0,
                hunks: Vec::new(),
                binary: delta.flags().is_binary(),
            });
            true
        },
        Some(&mut |_delta, _binary| {
            if let Some(f) = files.borrow_mut().last_mut() {
                f.binary = true;
            }
            true
        }),
        Some(&mut |_delta, hunk| {
            if let Some(f) = files.borrow_mut().last_mut() {
                let header = std::str::from_utf8(hunk.header())
                    .unwrap_or("")
                    .trim_end()
                    .to_string();
                f.hunks.push(Hunk {
                    header,
                    lines: Vec::new(),
                });
            }
            true
        }),
        Some(&mut |_delta, _hunk, line| {
            let mut files = files.borrow_mut();
            if let Some(f) = files.last_mut() {
                let text = String::from_utf8_lossy(line.content())
                    .trim_end_matches(['\n', '\r'])
                    .to_string();
                let (kind, old_no, new_no) = match line.origin() {
                    '+' => {
                        f.added += 1;
                        total_added.set(total_added.get() + 1);
                        (LineKind::Added, None, Some(line.new_lineno().unwrap_or(0)))
                    }
                    '-' => {
                        f.removed += 1;
                        total_removed.set(total_removed.get() + 1);
                        (LineKind::Removed, Some(line.old_lineno().unwrap_or(0)), None)
                    }
                    _ => (LineKind::Context, line.old_lineno(), line.new_lineno()),
                };
                // Lines arrive only inside a hunk; push to the current hunk.
                if let Some(h) = f.hunks.last_mut() {
                    h.lines.push(DiffLine {
                        kind,
                        old_no,
                        new_no,
                        text,
                    });
                }
            }
            true
        }),
    );

    (files.into_inner(), total_added.get(), total_removed.get())
}

fn short_oid(oid: &Oid) -> String {
    oid.to_string()[..7].to_string()
}

fn format_time(secs: i64) -> String {
    jiff::Timestamp::from_second(secs)
        .ok()
        .map(|ts| ts.strftime("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "????-??-??".into())
}
