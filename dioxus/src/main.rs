//! git-review — a GitHub-style code review desktop app, built with Dioxus 0.7 (desktop/wry).
//!
//! Usage: `git-review-dioxus [path-to-repo]` (defaults to the current directory, or
//! `$GIT_REVIEW_REPO`).
//!
//! Architecture: a single Dioxus desktop process. `git2` builds the diff model and `syntect`
//! highlights it — both run directly in-process (no separate backend). The model is turned into a
//! plain, `Clone`-able render tree (`RenderDiff`) stored in a Dioxus signal; RSX + a `<style>`
//! block render it as HTML/CSS inside the wry webview.

mod git;
mod highlight;
mod model;
mod ui;

use std::rc::Rc;

use dioxus::desktop::{Config, WindowBuilder};
use dioxus::prelude::*;

use git::Repo;
use highlight::Highlighter;

/// Shared, non-`Clone` resources held once for the lifetime of the app and reused on every
/// recompute. Stored behind `Rc` and stashed with `use_hook` so signals don't have to own them.
pub struct Engine {
    pub repo: Repo,
    pub hl: Highlighter,
    pub repo_name: String,
}

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

    let repo_name = repo.workdir_name();
    let engine = Rc::new(Engine {
        repo,
        hl: Highlighter::new(),
        repo_name,
    });

    let window = WindowBuilder::new()
        .with_title("git-review · dioxus")
        .with_inner_size(dioxus::desktop::tao::dpi::LogicalSize::new(1200.0, 820.0))
        .with_min_inner_size(dioxus::desktop::tao::dpi::LogicalSize::new(700.0, 480.0));

    let cfg = Config::new().with_window(window).with_menu(None);

    LaunchBuilder::desktop()
        .with_cfg(cfg)
        // Hand the engine to the component tree via context.
        .with_context(EngineCtx(engine))
        .launch(ui::app);
}

/// `with_context` requires `Clone + Send + Sync`. The desktop renderer is single-threaded, so the
/// `Rc<Engine>` never actually crosses threads; we assert that with a thin newtype.
#[derive(Clone)]
pub struct EngineCtx(pub Rc<Engine>);

// SAFETY: Dioxus desktop runs the entire VirtualDom and all event handlers on one thread (the wry
// event loop). The `Rc<Engine>` is created on that thread and only ever read on it; it is never
// sent to or shared with another thread despite the bound demanded by `with_context`.
unsafe impl Send for EngineCtx {}
unsafe impl Sync for EngineCtx {}
