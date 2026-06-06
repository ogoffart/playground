//! git-review — a GitHub-style code review desktop app, built with Xilem.
//!
//! Usage: `git-review-xilem [path-to-repo]` (defaults to the current directory, or
//! `$GIT_REVIEW_REPO`).
//!
//! Xilem is alpha (0.4); this implementation maps the shared `git-review` spec onto Xilem's
//! widget set (flex / split / portal / label / prose / button). See `README.md` for how the
//! tricky parts (sticky headers, scroll-to, multi-colour rich text) are approximated and where
//! Xilem's current API forced a compromise.

mod git;
mod highlight;

use std::sync::Arc;

use git2::Oid;
use xilem::masonry::parley::style::GenericFamily;
use xilem::masonry::properties::types::{AsUnit, CrossAxisAlignment, MainAxisAlignment};
use xilem::winit::error::EventLoopError;

use xilem::style::Style as _;
use xilem::view::{
    flex, flex_col, flex_row, label, portal, prose, sized_box, Axis, FlexExt as _, FlexSpacer,
};
use xilem::{
    Color, EventLoop, FontWeight, WidgetView, WindowOptions, Xilem,
};

use crate::git::{ChangeKind, CommitInfo, DiffSet, FileDiff, LineKind, Repo};
use crate::highlight::Highlighter;

// --- GitHub-ish palette -----------------------------------------------------
const BG: Color = Color::from_rgb8(0xff, 0xff, 0xff);
const PANEL: Color = Color::from_rgb8(0xf6, 0xf8, 0xfa);
const BORDER: Color = Color::from_rgb8(0xd0, 0xd7, 0xde);
const TEXT: Color = Color::from_rgb8(0x1f, 0x23, 0x28);
const MUTED: Color = Color::from_rgb8(0x65, 0x6d, 0x76);
const ACCENT: Color = Color::from_rgb8(0x09, 0x69, 0xda);
const SEL: Color = Color::from_rgb8(0xdd, 0xf4, 0xff);
const ADD_BG: Color = Color::from_rgb8(0xe6, 0xff, 0xec);
const ADD_MARK: Color = Color::from_rgb8(0xab, 0xf2, 0xbc);
const DEL_BG: Color = Color::from_rgb8(0xff, 0xeb, 0xe9);
const DEL_MARK: Color = Color::from_rgb8(0xff, 0x81, 0x82);
const ADD_FG: Color = Color::from_rgb8(0x1a, 0x7f, 0x37);
const DEL_FG: Color = Color::from_rgb8(0xcf, 0x22, 0x2e);
const HUNK_BG: Color = Color::from_rgb8(0xdd, 0xf4, 0xff);
const BTN_BG: Color = Color::from_rgb8(0xe6, 0xe6, 0xe6);

struct Settings {
    word_wrap: bool,
    show_space: bool,
    font_size: f32,
    line_numbers: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            word_wrap: false,
            show_space: true,
            font_size: 12.0,
            line_numbers: true,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Showing {
    Working,
    Commit(Oid),
    Range(Oid, Oid),
}

struct App {
    // `git2::Repository` is `Send` but not `Sync`; Xilem requires the app state to be
    // `Send + Sync` (views capture `PhantomData<State>` and must be `Send + Sync`). Wrapping the
    // repo in a `Mutex` makes `App` `Sync` without touching the shared `git.rs`.
    repo: std::sync::Mutex<Repo>,
    hl: Highlighter,
    repo_name: String,
    commits: Vec<CommitInfo>,
    diff: DiffSet,
    showing: Showing,
    from: Option<Oid>,
    to: Option<Oid>,
    settings: Settings,
    error: Option<String>,
}

impl App {
    fn new(repo: Repo) -> Self {
        let hl = Highlighter::new();
        let repo_name = repo.workdir_name();
        let commits = repo.commits(500).unwrap_or_default();
        let showing = commits
            .iter()
            .find_map(|c| c.oid.map(Showing::Commit))
            .unwrap_or(Showing::Working);
        let mut app = Self {
            repo: std::sync::Mutex::new(repo),
            hl,
            repo_name,
            commits,
            diff: empty_diff(),
            showing,
            from: None,
            to: None,
            settings: Settings::default(),
            error: None,
        };
        app.recompute();
        app
    }

    fn recompute(&mut self) {
        let ignore_ws = !self.settings.show_space;
        let repo = self.repo.lock().unwrap();
        let res = match self.showing {
            Showing::Working => repo.working_tree(ignore_ws),
            Showing::Commit(oid) => repo.show(oid, ignore_ws),
            Showing::Range(a, b) => repo.range(a, b, ignore_ws),
        };
        match res {
            Ok(d) => {
                self.diff = d;
                self.error = None;
            }
            Err(e) => self.error = Some(e.message().to_string()),
        }
    }

    fn select(&mut self, showing: Showing) {
        if self.showing != showing {
            self.showing = showing;
            self.recompute();
        }
    }

    fn maybe_range(&mut self) {
        if let (Some(a), Some(b)) = (self.from, self.to) {
            self.select(Showing::Range(a, b));
        }
    }
}

type AnyView = Box<xilem::AnyWidgetView<App>>;

// --- the root view ----------------------------------------------------------

fn app_logic(app: &mut App) -> impl WidgetView<App> + use<> {
    // The side panel is a vertical split: commit list on top, file tree below.
    let side = xilem::view::split(commit_list(app), file_tree(app))
        .split_axis(Axis::Vertical)
        .split_point(0.5)
        .min_size(120.px(), 120.px())
        .solid_bar(true);

    // Main view: toolbar pinned at top, scrollable diff below.
    let main = flex_col((toolbar(app), diff_view(app).flex(1.0)))
        .cross_axis_alignment(CrossAxisAlignment::Fill)
        .main_axis_alignment(MainAxisAlignment::Start)
        .must_fill_major_axis(true)
        .gap(0.px());

    // The whole left side panel is horizontally resizable against the main view (Xilem's
    // `split` provides a real draggable splitter).
    sized_box(
        xilem::view::split(
            sized_box(side).expand().background_color(PANEL),
            sized_box(main).expand().background_color(BG),
        )
        .split_axis(Axis::Horizontal)
        .split_point(0.27)
        .min_size(180.px(), 360.px())
        .solid_bar(true),
    )
    .expand()
    .background_color(BG)
}

// --- toolbar ----------------------------------------------------------------

fn toolbar(app: &App) -> impl WidgetView<App> + use<> {
    let summary = label(app.diff.summary.clone())
        .font(GenericFamily::Monospace)
        .weight(FontWeight::BOLD)
        .text_size(13.0)
        .color(TEXT);
    let added = label(format!("+{}", app.diff.added)).text_size(13.0).color(ADD_FG);
    let removed = label(format!("\u{2212}{}", app.diff.removed))
        .text_size(13.0)
        .color(DEL_FG);

    let wrap = app.settings.word_wrap;
    let space = app.settings.show_space;
    let nums = app.settings.line_numbers;

    let buttons = flex_row((
        tool_button("\u{21b6}", "Word wrap", wrap, |a: &mut App| {
            a.settings.word_wrap = !a.settings.word_wrap;
        }),
        tool_button("\u{2423}", "Show space changes", space, |a: &mut App| {
            a.settings.show_space = !a.settings.show_space;
            a.recompute();
        }),
        tool_button("A-", "Decrease font size", false, |a: &mut App| {
            a.settings.font_size = (a.settings.font_size - 1.0).max(8.0);
        }),
        tool_button("A+", "Increase font size", false, |a: &mut App| {
            a.settings.font_size = (a.settings.font_size + 1.0).min(28.0);
        }),
        tool_button("#", "Show line numbers", nums, |a: &mut App| {
            a.settings.line_numbers = !a.settings.line_numbers;
        }),
    ))
    .gap(4.px());

    sized_box(
        flex_row((
            FlexSpacer::Fixed(8.px()),
            summary,
            added,
            removed,
            FlexSpacer::Flex(1.0),
            buttons,
            FlexSpacer::Fixed(6.px()),
        ))
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .gap(6.px()),
    )
    .expand_width()
    .background_color(BG)
    .border(BORDER, 1.0)
    .padding(6.0)
}

fn tool_button(
    glyph: &str,
    _tip: &str,
    active: bool,
    cb: impl Fn(&mut App) + Send + Sync + 'static,
) -> impl WidgetView<App> {
    let fg = if active { ACCENT } else { TEXT };
    let bg = if active { SEL } else { Color::TRANSPARENT };
    let lbl = label(glyph.to_string())
        .font(GenericFamily::Monospace)
        .text_size(13.0)
        .weight(if active { FontWeight::BOLD } else { FontWeight::NORMAL })
        .color(fg);
    sized_box(
        xilem::view::button(lbl, move |a: &mut App| cb(a))
            .background_color(bg)
            .border(BORDER, 1.0)
            .corner_radius(4.0),
    )
}

// --- commit list ------------------------------------------------------------

fn commit_list(app: &App) -> impl WidgetView<App> + use<> {
    let header = section_header(format!("COMMITS \u{b7} {}", app.repo_name));

    let rows: Vec<AnyView> = app
        .commits
        .iter()
        .map(|c| commit_row(app, c))
        .collect();

    sized_box(
        flex_col((
            header,
            portal(
                flex_col(rows)
                    .cross_axis_alignment(CrossAxisAlignment::Fill)
                    .gap(0.px()),
            )
            .flex(1.0),
        ))
        .cross_axis_alignment(CrossAxisAlignment::Fill)
        .must_fill_major_axis(true)
        .gap(0.px()),
    )
    .expand()
    .background_color(PANEL)
}

fn commit_row(app: &App, c: &CommitInfo) -> AnyView {
    let is_current = match (app.showing, c.oid) {
        (Showing::Working, _) if c.is_working_tree() => true,
        (Showing::Commit(o), Some(oid)) => o == oid,
        _ => false,
    };
    let is_from = c.oid.is_some() && c.oid == app.from;
    let is_to = c.oid.is_some() && c.oid == app.to;

    // line 1: endpoint buttons + sha + date + author
    let oid = c.oid;
    let from_btn: AnyView = if oid.is_some() {
        endpoint_button("from", is_from, move |a: &mut App| {
            a.from = oid;
            a.maybe_range();
        })
        .boxed()
    } else {
        sized_box(label("")).width(64.px()).boxed()
    };
    let to_btn: AnyView = if oid.is_some() {
        endpoint_button("to", is_to, move |a: &mut App| {
            a.to = oid;
            a.maybe_range();
        })
        .boxed()
    } else {
        sized_box(label("")).width(0.px()).boxed()
    };

    let line1 = flex_row((
        from_btn,
        to_btn,
        label(c.short.clone())
            .font(GenericFamily::Monospace)
            .text_size(11.0)
            .color(ACCENT)
            .boxed(),
        label(c.date.clone()).text_size(10.5).color(MUTED).boxed(),
        flex_spacer(),
        sized_box(label(ellipsize(&c.author, 16)).text_size(10.5).color(MUTED))
            .boxed(),
    ))
    .cross_axis_alignment(CrossAxisAlignment::Center)
    .gap(5.px());

    // line 2: clickable title -> opens the commit
    let open_target = match c.oid {
        Some(oid) => Showing::Commit(oid),
        None => Showing::Working,
    };
    let title = xilem::view::button(
        label(ellipsize(&c.title, 64)).text_size(12.0).color(TEXT),
        move |a: &mut App| a.select(open_target),
    )
    .background_color(Color::TRANSPARENT)
    .border(Color::TRANSPARENT, 0.0)
    .padding(0.0);

    let bg = if is_current { SEL } else { PANEL };
    sized_box(
        flex_col((line1.boxed(), title.boxed()))
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .gap(2.px()),
    )
    .expand_width()
    .background_color(bg)
    .border(BORDER, 0.5)
    .padding(xilem::style::Padding::from(6.0))
    .boxed()
}

fn endpoint_button(
    text: &str,
    active: bool,
    cb: impl Fn(&mut App) + Send + Sync + 'static,
) -> impl WidgetView<App> {
    let bg = if active { ACCENT } else { BTN_BG };
    let fg = if active { BG } else { TEXT };
    sized_box(
        xilem::view::button(label(text.to_string()).text_size(9.5).color(fg), move |a: &mut App| {
            cb(a)
        })
        .background_color(bg)
        .border(BORDER, 0.5)
        .corner_radius(3.0)
        .padding(xilem::style::Padding::from(1.0)),
    )
}

// --- file tree --------------------------------------------------------------

fn file_tree(app: &App) -> impl WidgetView<App> + use<> {
    let header = section_header(format!("FILES ({})", app.diff.files.len()));

    let rows: Vec<AnyView> = app
        .diff
        .files
        .iter()
        .enumerate()
        .map(|(i, f)| file_row(i, f))
        .collect();

    sized_box(
        flex_col((
            header,
            portal(
                flex_col(rows)
                    .cross_axis_alignment(CrossAxisAlignment::Fill)
                    .gap(0.px()),
            )
            .flex(1.0),
        ))
        .cross_axis_alignment(CrossAxisAlignment::Fill)
        .must_fill_major_axis(true)
        .gap(0.px()),
    )
    .expand()
    .background_color(PANEL)
}

fn file_row(_i: usize, f: &FileDiff) -> AnyView {
    // Xilem's Portal exposes no scroll-to API at the view layer (alpha limitation), so a file
    // row cannot programmatically scroll the diff. We keep the row as a non-interactive entry
    // showing the icon, path and per-file counts; see README.
    sized_box(
        flex_row((
            label(file_icon(&f.path)).text_size(13.0).boxed(),
            sized_box(label(ellipsize(&f.path, 30)).text_size(12.0).color(TEXT))
                .boxed(),
            flex_spacer(),
            label(format!("+{}", f.added)).text_size(10.5).color(ADD_FG).boxed(),
            label(format!("\u{2212}{}", f.removed))
                .text_size(10.5)
                .color(DEL_FG)
                .boxed(),
        ))
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .gap(5.px()),
    )
    .expand_width()
    .padding(xilem::style::Padding::from(4.0))
    .boxed()
}

// --- diff view --------------------------------------------------------------

fn diff_view(app: &App) -> AnyView {
    if let Some(err) = &app.error {
        return portal(
            sized_box(label(format!("Error: {err}")).text_size(13.0).color(DEL_FG))
                .expand_width(),
        )
        .boxed();
    }

    let mut blocks: Vec<AnyView> = Vec::new();

    // Commit message (single-commit view only).
    if let Some(msg) = &app.diff.message {
        blocks.push(commit_message(msg));
    }

    let fs = app.settings.font_size;
    for f in &app.diff.files {
        blocks.push(file_header(f));
        blocks.push(file_body(app, f, fs));
        blocks.push(sized_box(label("")).height(10.px()).boxed());
    }

    portal(
        sized_box(
            flex_col(blocks)
                .cross_axis_alignment(CrossAxisAlignment::Fill)
                .main_axis_alignment(MainAxisAlignment::Start)
                .gap(0.px()),
        )
        .expand_width()
        .background_color(BG)
        .padding(xilem::style::Padding::from(8.0)),
    )
    .boxed()
}

fn commit_message(msg: &crate::git::CommitMessage) -> AnyView {
    let mut kids: Vec<AnyView> = vec![
        label(msg.title.clone())
            .weight(FontWeight::BOLD)
            .text_size(18.0)
            .color(TEXT)
            .boxed(),
        label(format!("{}  \u{b7}  {}  \u{b7}  {}", msg.author, msg.date, msg.short))
            .text_size(11.0)
            .color(MUTED)
            .boxed(),
    ];
    if !msg.body.is_empty() {
        kids.push(
            prose(msg.body.clone())
                .text_color(TEXT)
                .text_size(12.0)
                .boxed(),
        );
    }
    sized_box(
        flex_col(kids)
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .gap(6.px()),
    )
    .expand_width()
    .background_color(PANEL)
    .border(BORDER, 1.0)
    .padding(xilem::style::Padding::from(12.0))
    .boxed()
}

fn file_header(f: &FileDiff) -> AnyView {
    // NOTE: Masonry has no "sticky" sub-region inside a Portal, so file headers scroll with the
    // content rather than pinning. They are styled as a distinct bar so they remain a clear
    // section divider. See README.
    let label_text = match (&f.old_path, f.kind) {
        (Some(old), ChangeKind::Renamed) => format!("{old}  \u{2192}  {}", f.path),
        _ => f.path.clone(),
    };
    sized_box(
        flex_row((
            label(file_icon(&f.path)).text_size(13.0).boxed(),
            label(label_text).weight(FontWeight::BOLD).text_size(12.5).color(TEXT).boxed(),
            label(format!("[{}]", f.kind.letter())).text_size(10.5).color(MUTED).boxed(),
            flex_spacer(),
            label(format!("+{}", f.added)).text_size(12.0).color(ADD_FG).boxed(),
            label(format!("\u{2212}{}", f.removed)).text_size(12.0).color(DEL_FG).boxed(),
        ))
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .gap(6.px()),
    )
    .expand_width()
    .background_color(PANEL)
    .border(BORDER, 1.0)
    .padding(xilem::style::Padding::from(6.0))
    .boxed()
}

fn file_body(app: &App, f: &FileDiff, fs: f32) -> AnyView {
    if f.binary {
        return sized_box(
            label("Binary file not shown").text_size(fs).color(MUTED),
        )
        .padding(xilem::style::Padding::from(8.0))
        .boxed();
    }

    let syntax = app.hl.syntax_for(&f.path);
    let mut rows: Vec<AnyView> = Vec::new();
    for hunk in &f.hunks {
        rows.push(diff_line(app, LineKind::Context, None, None, &hunk.header, None, true, fs));
        for line in &hunk.lines {
            rows.push(diff_line(
                app,
                line.kind,
                line.old_no,
                line.new_no,
                &line.text,
                Some(syntax),
                false,
                fs,
            ));
        }
    }
    flex_col(rows)
        .cross_axis_alignment(CrossAxisAlignment::Fill)
        .gap(0.px())
        .boxed()
}

#[allow(clippy::too_many_arguments)]
fn diff_line(
    app: &App,
    kind: LineKind,
    old_no: Option<u32>,
    new_no: Option<u32>,
    text: &str,
    syntax: Option<&syntect::parsing::SyntaxReference>,
    hunk_header: bool,
    fs: f32,
) -> AnyView {
    let (row_bg, mark_bg, sign, sign_fg) = match kind {
        LineKind::Added => (ADD_BG, ADD_MARK, '+', ADD_FG),
        LineKind::Removed => (DEL_BG, DEL_MARK, '-', DEL_FG),
        LineKind::Context => (Color::TRANSPARENT, Color::TRANSPARENT, ' ', MUTED),
    };

    let num = |n: Option<u32>| -> String {
        n.map(|n| n.to_string()).unwrap_or_default()
    };

    let mut parts: Vec<AnyView> = Vec::new();

    if app.settings.line_numbers && !hunk_header {
        parts.push(
            sized_box(label(num(old_no)).font(GenericFamily::Monospace).text_size(fs).color(MUTED))
                .width((fs * 2.6).px())
                .boxed(),
        );
        parts.push(
            sized_box(label(num(new_no)).font(GenericFamily::Monospace).text_size(fs).color(MUTED))
                .width((fs * 2.6).px())
                .boxed(),
        );
    }

    // sign / marker column (stronger green/red strip)
    if !hunk_header {
        parts.push(
            sized_box(
                label(sign.to_string())
                    .font(GenericFamily::Monospace)
                    .text_size(fs)
                    .color(sign_fg),
            )
            .width((fs * 1.2).px())
            .background_color(mark_bg)
            .boxed(),
        );
    }

    // code: one label per syntax span (Masonry's Label is single-style, so multi-colour text is
    // approximated by laying spans out in a flex_row).
    if hunk_header {
        parts.push(
            label(text.to_string())
                .font(GenericFamily::Monospace)
                .text_size(fs)
                .color(MUTED)
                .boxed(),
        );
    } else if let Some(syntax) = syntax {
        let spans = app.hl.line(syntax, text);
        if spans.is_empty() {
            parts.push(
                label(" ").font(GenericFamily::Monospace).text_size(fs).boxed(),
            );
        } else if app.settings.word_wrap {
            // When wrapping, join into one prose block (loses per-span colour) so long lines
            // wrap; otherwise each span is its own non-wrapping label.
            let joined: String = spans.iter().map(|s| s.text.as_str()).collect();
            parts.push(
                prose(joined)
                    .text_color(TEXT)
                    .text_size(fs)
                    .boxed(),
            );
        } else {
            for s in &spans {
                if s.text.is_empty() {
                    continue;
                }
                parts.push(
                    label(s.text.replace('\t', "    "))
                        .font(GenericFamily::Monospace)
                        .weight(if s.bold { FontWeight::BOLD } else { FontWeight::NORMAL })
                        .text_size(fs)
                        .color(Color::from_rgb8(s.color.0, s.color.1, s.color.2))
                        .boxed(),
                );
            }
        }
    } else {
        parts.push(
            label(text.to_string())
                .font(GenericFamily::Monospace)
                .text_size(fs)
                .color(TEXT)
                .boxed(),
        );
    }

    let bg = if hunk_header { HUNK_BG } else { row_bg };
    sized_box(
        flex(Axis::Horizontal, parts)
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .main_axis_alignment(MainAxisAlignment::Start)
            .gap(0.px()),
    )
    .expand_width()
    .background_color(bg)
    .boxed()
}

// --- small helpers ----------------------------------------------------------

fn section_header(text: String) -> impl WidgetView<App> + use<> {
    sized_box(
        label(text)
            .weight(FontWeight::BOLD)
            .text_size(10.5)
            .color(MUTED),
    )
    .expand_width()
    .background_color(PANEL)
    .padding(xilem::style::Padding::from(6.0))
}

fn file_icon(path: &str) -> &'static str {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext {
        "rs" => "\u{1f980}",
        "py" => "\u{1f40d}",
        "js" | "ts" | "tsx" | "jsx" => "\u{1f4dc}",
        "md" => "\u{1f4dd}",
        "toml" | "yaml" | "yml" | "json" | "ini" | "cfg" => "\u{2699}",
        "png" | "jpg" | "jpeg" | "gif" | "svg" => "\u{1f5bc}",
        "sh" | "bash" => "\u{1f4b2}",
        "html" | "css" => "\u{1f310}",
        _ => "\u{1f4c4}",
    }
}

fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('\u{2026}');
        out
    }
}

fn empty_diff() -> DiffSet {
    DiffSet {
        message: None,
        files: Vec::new(),
        added: 0,
        removed: 0,
        summary: String::new(),
    }
}

// --- boxing helpers ---------------------------------------------------------
// We rely on xilem's built-in `WidgetView::boxed()` (-> Box<AnyWidgetView<App>>) for our
// `AnyView` alias. A flexible spacer inside an `AnyView` list is approximated by an
// `expand_width` transparent box (flex weighting can't be carried through a boxed any-view).

fn flex_spacer() -> AnyView {
    sized_box(label("")).expand_width().boxed()
}

fn main() -> Result<(), EventLoopError> {
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

    let app = App::new(repo);
    let _ = Arc::new(()); // (kept for parity; no async state)

    Xilem::new_simple(
        app,
        app_logic,
        WindowOptions::new("git-review \u{b7} xilem")
            .with_initial_inner_size(xilem::dpi::LogicalSize::new(1200.0, 820.0))
            .with_min_inner_size(xilem::dpi::LogicalSize::new(700.0, 480.0)),
    )
    .run_in(EventLoop::with_user_event())?;
    Ok(())
}
