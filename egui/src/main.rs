//! git-review — a GitHub-style code review desktop app, built with egui.
//!
//! Usage: `git-review-egui [path-to-repo]` (defaults to the current directory, or
//! `$GIT_REVIEW_REPO`).

mod app;
mod git;
mod highlight;

use app::App;
use git::Repo;

fn main() -> eframe::Result<()> {
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

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([700.0, 480.0])
            .with_title("git-review · egui"),
        ..Default::default()
    };

    eframe::run_native(
        "git-review-egui",
        options,
        Box::new(|_cc| Ok(Box::new(App::new(repo)))),
    )
}
