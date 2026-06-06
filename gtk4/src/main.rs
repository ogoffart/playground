//! git-review — a GitHub-style code review desktop app, built with GTK4 (gtk4-rs).
//!
//! Usage: `git-review-gtk4 [path-to-repo]` (defaults to the current directory, or
//! `$GIT_REVIEW_REPO`).

mod git;
mod highlight;
mod app;

use git::Repo;

use gtk::prelude::*;
use gtk::{gio, glib, Application};

const APP_ID: &str = "dev.slint.git-review-gtk4";

fn main() -> glib::ExitCode {
    let path = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("GIT_REVIEW_REPO").ok())
        .unwrap_or_else(|| ".".to_string());

    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();

    app.connect_startup(|_| app::load_css());

    app.connect_activate(move |application| match Repo::open(&path) {
        Ok(repo) => app::build_ui(application, repo),
        Err(e) => {
            eprintln!("Could not open a git repository at '{path}': {e}");
            std::process::exit(1);
        }
    });

    // Don't treat argv[1] (the repo path) as a file to open.
    app.run_with_args::<&str>(&[])
}
