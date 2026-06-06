//! git-review, Qt Widgets edition.
//!
//! A GitHub-style code-review desktop app. The data layer (git2 + syntect) lives in Rust; the UI is
//! genuine **Qt Widgets** (QSplitter / QTreeWidget / QListWidget / QTextEdit / QToolBar), built in a
//! small C++ shim (`src/shim.cpp`) compiled with the `cc` crate and driven through the C ABI in
//! `ffi.rs`. This is deliberately a QtWidgets app (not QML) to contrast with the qt-cxx /
//! qt-qmetaobject apps in this repo.

mod ffi;
mod git;
mod highlight;

use std::ffi::CString;

extern "C" {
    /// Runs the Qt application. Defined in `src/shim.cpp`.
    fn gr_run_app(argc: i32, argv: *const *const std::ffi::c_char, repo_path: *const std::ffi::c_char) -> i32;
}

fn main() {
    // Repo path: argv[1] | $GIT_REVIEW_REPO | cwd.
    let repo_path = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("GIT_REVIEW_REPO").ok())
        .unwrap_or_else(|| ".".to_string());

    // Keep argv alive for the duration of the Qt app.
    let args: Vec<CString> = std::env::args()
        .map(|a| CString::new(a).unwrap_or_default())
        .collect();
    let mut argv: Vec<*const std::ffi::c_char> = args.iter().map(|a| a.as_ptr()).collect();
    argv.push(std::ptr::null());
    let argc = args.len() as i32;

    let repo_c = CString::new(repo_path).unwrap();

    let code = unsafe { gr_run_app(argc, argv.as_ptr(), repo_c.as_ptr()) };
    std::process::exit(code);
}
