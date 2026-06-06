//! Entry point for the GPUI git-review app.
//!
//! Opens the repository given as argv[1] (or `$GIT_REVIEW_REPO`, default `.`) and runs the
//! GPUI application, mirroring the other framework apps in this repo.

mod app;
mod git;
mod highlight;

use gpui::{
    px, size, App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions,
};

use crate::app::ReviewApp;
use crate::git::Repo;

fn main() {
    let path = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("GIT_REVIEW_REPO").ok())
        .unwrap_or_else(|| ".".to_string());

    let repo = match Repo::open(&path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("failed to open git repository at {path:?}: {e}");
            std::process::exit(1);
        }
    };

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1280.0), px(860.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("git-review (GPUI)".into()),
                    ..Default::default()
                }),
                focus: true,
                ..Default::default()
            },
            |window, cx| cx.new(|cx| ReviewApp::new(repo, window, cx)),
        )
        .unwrap();
        cx.activate(true);
    });
}
