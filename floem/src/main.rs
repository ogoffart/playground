//! git-review — a GitHub-style code review desktop app, built with Floem.
//!
//! Usage: `git-review-floem [path-to-repo]` (defaults to the current directory, or
//! `$GIT_REVIEW_REPO`).

mod git;
mod highlight;
mod ui;

use git::Repo;

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

    ui::launch_app(repo);
}
