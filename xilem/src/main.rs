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

use git2::Oid;
use xilem::masonry::parley::style::GenericFamily;
use xilem::masonry::properties::types::{AsUnit, CrossAxisAlignment, MainAxisAlignment};
use xilem::winit::error::EventLoopError;

use xilem::style::Style as _;
use xilem::view::{
    flex, flex_col, flex_row, label, portal, prose, sized_box, Axis, FlexExt as _, FlexSpacer,
};
use xilem::{Color, EventLoop, FontWeight, WidgetView, WindowOptions, Xilem};

use crate::git::{ChangeKind, CommitInfo, DiffSet, FileDiff, LineKind, Repo};
use crate::highlight::Highlighter;

// --- Theme ------------------------------------------------------------------
// Both GitHub palettes (light + dark), selected at runtime from the desktop scheme.

#[derive(Clone, Copy)]
struct Theme {
    dark: bool,
    bg: Color,
    panel: Color,
    border: Color,
    text: Color,
    muted: Color,
    accent: Color,
    sel: Color,
    add_bg: Color,
    add_mark: Color,
    add_fg: Color,
    del_bg: Color,
    del_mark: Color,
    del_fg: Color,
    btn_bg: Color,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}

impl Theme {
    /// GitHub light palette (see SPEC.md).
    const fn light() -> Self {
        Self {
            dark: false,
            bg: rgb(0xff, 0xff, 0xff),
            panel: rgb(0xf6, 0xf8, 0xfa),
            border: rgb(0xd0, 0xd7, 0xde),
            text: rgb(0x1f, 0x23, 0x28),
            muted: rgb(0x65, 0x6d, 0x76),
            accent: rgb(0x09, 0x69, 0xda),
            sel: rgb(0xdd, 0xf4, 0xff),
            add_bg: rgb(0xe6, 0xff, 0xec),
            add_mark: rgb(0xab, 0xf2, 0xbc),
            add_fg: rgb(0x1a, 0x7f, 0x37),
            del_bg: rgb(0xff, 0xeb, 0xe9),
            del_mark: rgb(0xff, 0x81, 0x82),
            del_fg: rgb(0xcf, 0x22, 0x2e),
            btn_bg: rgb(0xe6, 0xe6, 0xe6),
        }
    }

    /// GitHub dark palette (see SPEC.md).
    const fn dark() -> Self {
        Self {
            dark: true,
            bg: rgb(0x0d, 0x11, 0x17),
            panel: rgb(0x16, 0x1b, 0x22),
            border: rgb(0x30, 0x36, 0x3d),
            text: rgb(0xe6, 0xed, 0xf3),
            muted: rgb(0x8b, 0x94, 0x9e),
            accent: rgb(0x2f, 0x81, 0xf7),
            sel: rgb(0x1f, 0x6f, 0xeb),
            add_bg: rgb(0x12, 0x26, 0x1e),
            add_mark: rgb(0x2e, 0xa0, 0x43),
            add_fg: rgb(0x3f, 0xb9, 0x50),
            del_bg: rgb(0x25, 0x17, 0x1c),
            del_mark: rgb(0xf8, 0x51, 0x49),
            del_fg: rgb(0xf8, 0x51, 0x49),
            btn_bg: rgb(0x21, 0x26, 0x2d),
        }
    }

    /// Hunk-header background reuses the selection tint.
    fn hunk_bg(&self) -> Color {
        self.sel
    }
}

/// Decide the palette: `GIT_REVIEW_THEME` override wins, otherwise the desktop scheme via
/// `dark-light`; headless / no preference → light (per SPEC.md).
fn detect_theme() -> Theme {
    if let Ok(v) = std::env::var("GIT_REVIEW_THEME") {
        match v.trim().to_ascii_lowercase().as_str() {
            "dark" => return Theme::dark(),
            "light" => return Theme::light(),
            _ => {}
        }
    }
    match dark_light::detect() {
        Ok(dark_light::Mode::Dark) => Theme::dark(),
        _ => Theme::light(),
    }
}

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

/// A node in the rendered file tree (folders and file leaves, flattened with a depth).
struct TreeRow {
    depth: usize,
    /// `Some(file_index)` for a leaf (index into `diff.files`), `None` for a folder.
    file: Option<usize>,
    /// Display name (file name, or the — possibly collapsed — folder name like `a/b/c`).
    name: String,
    /// Full folder path key (folders only); used to track collapsed state.
    key: String,
    expanded: bool,
}

struct App {
    // `git2::Repository` is `Send` but not `Sync`; Xilem requires the app state to be
    // `Send + Sync` (views capture `PhantomData<State>` and must be `Send + Sync`). Wrapping the
    // repo in a `Mutex` makes `App` `Sync` without touching the shared `git.rs`.
    repo: std::sync::Mutex<Repo>,
    hl: Highlighter,
    theme: Theme,
    repo_name: String,
    commits: Vec<CommitInfo>,
    diff: DiffSet,
    showing: Showing,
    from: Option<Oid>,
    to: Option<Oid>,
    settings: Settings,
    error: Option<String>,
    /// Folder paths the user has collapsed (default = expanded).
    collapsed: std::collections::HashSet<String>,
    /// File the user last clicked in the tree (highlighted; diff scrolled toward it).
    selected_file: Option<usize>,
    /// Bumped on every scroll-to request so the diff portal view re-applies the offset.
    scroll_gen: u64,
}

impl App {
    fn new(repo: Repo, theme: Theme) -> Self {
        let hl = Highlighter::new(theme.dark);
        let repo_name = repo.workdir_name();
        let commits = repo.commits(500).unwrap_or_default();
        let showing = commits
            .iter()
            .find_map(|c| c.oid.map(Showing::Commit))
            .unwrap_or(Showing::Working);
        let mut app = Self {
            repo: std::sync::Mutex::new(repo),
            hl,
            theme,
            repo_name,
            commits,
            diff: empty_diff(),
            showing,
            from: None,
            to: None,
            settings: Settings::default(),
            error: None,
            collapsed: std::collections::HashSet::new(),
            selected_file: None,
            scroll_gen: 0,
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
        self.selected_file = None;
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

    /// Click a file leaf: select it and ask the diff portal to scroll toward it.
    fn scroll_to_file(&mut self, idx: usize) {
        self.selected_file = Some(idx);
        self.scroll_gen = self.scroll_gen.wrapping_add(1);
    }

    /// Toggle a folder's collapsed state.
    fn toggle_folder(&mut self, key: &str) {
        if !self.collapsed.remove(key) {
            self.collapsed.insert(key.to_string());
        }
    }

    /// Approximate vertical content fraction (0..1) of `selected_file`'s header within the diff,
    /// using cumulative line counts (rows are uniform height). Drives the portal scroll offset.
    fn scroll_fraction(&self) -> Option<f64> {
        let target = self.selected_file?;
        // total "rows" in the diff body: message (~3), then per file: header(1)+hunks+lines.
        let row = |f: &FileDiff| -> usize {
            1 + f
                .hunks
                .iter()
                .map(|h| 1 + h.lines.len())
                .sum::<usize>()
                + 1 // trailing spacer
        };
        let mut before = if self.diff.message.is_some() { 3 } else { 0 };
        let mut total = before;
        for (i, f) in self.diff.files.iter().enumerate() {
            if i == target {
                // position at the file's header
            } else if i < target {
                before += row(f);
            }
            total += row(f);
        }
        if total == 0 {
            return None;
        }
        Some((before as f64 / total as f64).clamp(0.0, 1.0))
    }
}

type AnyView = Box<xilem::AnyWidgetView<App>>;

// --- the root view ----------------------------------------------------------

fn app_logic(app: &mut App) -> impl WidgetView<App> + use<> {
    let t = app.theme;

    // The side panel is a vertical split: commit list on top, file tree below.
    let side = xilem::view::split(commit_list(app), file_tree(app))
        .split_axis(Axis::Vertical)
        .split_point(0.5)
        .min_size(120.px(), 120.px())
        .solid_bar(true);

    // Below the toolbar: side panel | diff, horizontally resizable (a real draggable splitter).
    let body = xilem::view::split(
        sized_box(side).expand().background_color(t.panel),
        sized_box(diff_view(app)).expand().background_color(t.bg),
    )
    .split_axis(Axis::Horizontal)
    .split_point(0.27)
    .min_size(180.px(), 360.px())
    .solid_bar(true);

    // Root is a vertical flex: [full-width toolbar] then [side | main].
    sized_box(
        flex_col((toolbar(app), sized_box(body).expand().flex(1.0)))
            .cross_axis_alignment(CrossAxisAlignment::Fill)
            .main_axis_alignment(MainAxisAlignment::Start)
            .must_fill_major_axis(true)
            .gap(0.px()),
    )
    .expand()
    .background_color(t.bg)
}

// --- toolbar (full width, pinned at the very top) ---------------------------

fn toolbar(app: &App) -> impl WidgetView<App> + use<> {
    let t = app.theme;
    let summary = label(app.diff.summary.clone())
        .font(GenericFamily::Monospace)
        .weight(FontWeight::BOLD)
        .text_size(13.0)
        .color(t.text);
    let added = label(format!("+{}", app.diff.added))
        .text_size(13.0)
        .color(t.add_fg);
    let removed = label(format!("\u{2212}{}", app.diff.removed))
        .text_size(13.0)
        .color(t.del_fg);

    let wrap = app.settings.word_wrap;
    let space = app.settings.show_space;
    let nums = app.settings.line_numbers;

    let buttons = flex_row((
        tool_button(t, "\u{21b6}", "Word wrap", wrap, |a: &mut App| {
            a.settings.word_wrap = !a.settings.word_wrap;
        }),
        tool_button(t, "\u{2423}", "Show space changes", space, |a: &mut App| {
            a.settings.show_space = !a.settings.show_space;
            a.recompute();
        }),
        tool_button(t, "A-", "Decrease font size", false, |a: &mut App| {
            a.settings.font_size = (a.settings.font_size - 1.0).max(8.0);
        }),
        tool_button(t, "A+", "Increase font size", false, |a: &mut App| {
            a.settings.font_size = (a.settings.font_size + 1.0).min(28.0);
        }),
        tool_button(t, "#", "Show line numbers", nums, |a: &mut App| {
            a.settings.line_numbers = !a.settings.line_numbers;
        }),
    ))
    .gap(4.px());

    // A distinct bar with a bottom border / subtle background so it reads as a toolbar.
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
    .background_color(t.panel)
    .border(t.border, 1.0)
    .padding(8.0)
}

fn tool_button(
    t: Theme,
    glyph: &str,
    _tip: &str,
    active: bool,
    cb: impl Fn(&mut App) + Send + Sync + 'static,
) -> impl WidgetView<App> {
    let fg = if active { t.accent } else { t.text };
    let bg = if active { t.sel } else { Color::TRANSPARENT };
    let lbl = label(glyph.to_string())
        .font(GenericFamily::Monospace)
        .text_size(13.0)
        .weight(if active {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        })
        .color(fg);
    sized_box(
        xilem::view::button(lbl, move |a: &mut App| cb(a))
            .background_color(bg)
            .border(t.border, 1.0)
            .corner_radius(4.0),
    )
}

// --- commit list ------------------------------------------------------------

fn commit_list(app: &App) -> impl WidgetView<App> + use<> {
    let t = app.theme;
    let header = section_header(t, format!("COMMITS \u{b7} {}", app.repo_name));

    let rows: Vec<AnyView> = app.commits.iter().map(|c| commit_row(app, c)).collect();

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
    .background_color(t.panel)
}

fn commit_row(app: &App, c: &CommitInfo) -> AnyView {
    let t = app.theme;
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
        endpoint_button(t, "from", is_from, move |a: &mut App| {
            a.from = oid;
            a.maybe_range();
        })
        .boxed()
    } else {
        sized_box(label("")).width(64.px()).boxed()
    };
    let to_btn: AnyView = if oid.is_some() {
        endpoint_button(t, "to", is_to, move |a: &mut App| {
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
            .color(t.accent)
            .boxed(),
        label(c.date.clone()).text_size(10.5).color(t.muted).boxed(),
        flex_spacer(),
        sized_box(label(ellipsize(&c.author, 16)).text_size(10.5).color(t.muted)).boxed(),
    ))
    .cross_axis_alignment(CrossAxisAlignment::Center)
    .gap(5.px());

    // line 2: clickable title -> opens the commit
    let open_target = match c.oid {
        Some(oid) => Showing::Commit(oid),
        None => Showing::Working,
    };
    let title = xilem::view::button(
        label(ellipsize(&c.title, 64)).text_size(12.0).color(t.text),
        move |a: &mut App| a.select(open_target),
    )
    .background_color(Color::TRANSPARENT)
    .border(Color::TRANSPARENT, 0.0)
    .padding(0.0);

    let bg = if is_current { t.sel } else { t.panel };
    sized_box(
        flex_col((line1.boxed(), title.boxed()))
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .gap(2.px()),
    )
    .expand_width()
    .background_color(bg)
    .border(t.border, 0.5)
    .padding(xilem::style::Padding::from(6.0))
    .boxed()
}

fn endpoint_button(
    t: Theme,
    text: &str,
    active: bool,
    cb: impl Fn(&mut App) + Send + Sync + 'static,
) -> impl WidgetView<App> {
    let bg = if active { t.accent } else { t.btn_bg };
    let fg = if active { t.bg } else { t.text };
    sized_box(
        xilem::view::button(
            label(text.to_string()).text_size(9.5).color(fg),
            move |a: &mut App| cb(a),
        )
        .background_color(bg)
        .border(t.border, 0.5)
        .corner_radius(3.0)
        .padding(xilem::style::Padding::from(1.0)),
    )
}

// --- file tree (real, hierarchical) -----------------------------------------

/// Build a flattened, GitHub-style file tree from the diff's file paths, honouring the user's
/// collapsed-folder set. Single-child directory chains are merged into one node (`a/b/c`); when a
/// folder is collapsed, its descendants are omitted.
fn build_tree(app: &App) -> Vec<TreeRow> {
    // An intermediate in-memory tree keyed by path component.
    #[derive(Default)]
    struct Node {
        children: std::collections::BTreeMap<String, Node>,
        file: Option<usize>,
    }
    let mut root = Node::default();
    for (i, f) in app.diff.files.iter().enumerate() {
        let mut cur = &mut root;
        let comps: Vec<&str> = f.path.split('/').filter(|s| !s.is_empty()).collect();
        for (j, comp) in comps.iter().enumerate() {
            let last = j + 1 == comps.len();
            cur = cur.children.entry((*comp).to_string()).or_default();
            if last {
                cur.file = Some(i);
            }
        }
    }

    let mut rows = Vec::new();
    // Recursive flatten. `prefix` is the folder path key built so far.
    fn walk(
        node: &Node,
        name: String,
        prefix: String,
        depth: usize,
        collapsed: &std::collections::HashSet<String>,
        rows: &mut Vec<TreeRow>,
    ) {
        if let Some(idx) = node.file {
            if node.children.is_empty() {
                rows.push(TreeRow {
                    depth,
                    file: Some(idx),
                    name,
                    key: String::new(),
                    expanded: false,
                });
                return;
            }
        }

        // Collapse single-child directory chains GitHub-style: a/b/c.
        let mut disp = name;
        let mut cur = node;
        let mut key = prefix;
        if !key.is_empty() {
            key.push('/');
        }
        key.push_str(&disp);
        while cur.children.len() == 1 && cur.file.is_none() {
            let (child_name, child) = cur.children.iter().next().unwrap();
            if !child.children.is_empty() {
                disp = format!("{disp}/{child_name}");
                key = format!("{key}/{child_name}");
                cur = child;
            } else {
                break;
            }
        }

        let expanded = !collapsed.contains(&key);
        rows.push(TreeRow {
            depth,
            file: None,
            name: format!("{disp}/"),
            key: key.clone(),
            expanded,
        });
        if expanded {
            for (child_name, child) in &cur.children {
                walk(
                    child,
                    child_name.clone(),
                    key.clone(),
                    depth + 1,
                    collapsed,
                    rows,
                );
            }
        }
    }

    for (name, child) in &root.children {
        walk(
            child,
            name.clone(),
            String::new(),
            0,
            &app.collapsed,
            &mut rows,
        );
    }
    rows
}

fn file_tree(app: &App) -> impl WidgetView<App> + use<> {
    let t = app.theme;
    let header = section_header(t, format!("FILES ({})", app.diff.files.len()));

    let tree = build_tree(app);
    let rows: Vec<AnyView> = tree
        .iter()
        .map(|r| {
            if let Some(idx) = r.file {
                file_leaf(app, r, idx)
            } else {
                folder_row(app, r)
            }
        })
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
    .background_color(t.panel)
}

fn folder_row(app: &App, r: &TreeRow) -> AnyView {
    let t = app.theme;
    let key = r.key.clone();
    let arrow = if r.expanded { "\u{25be}" } else { "\u{25b8}" }; // ▾ / ▸
    let content = flex_row((
        sized_box(label(""))
            .width((r.depth as f64 * 12.0).px())
            .boxed(),
        label(arrow.to_string())
            .text_size(10.0)
            .color(t.muted)
            .boxed(),
        label("\u{1f4c1}").text_size(12.0).boxed(), // 📁
        label(ellipsize(&r.name, 28))
            .weight(FontWeight::BOLD)
            .text_size(12.0)
            .color(t.text)
            .boxed(),
    ))
    .cross_axis_alignment(CrossAxisAlignment::Center)
    .gap(4.px());

    sized_box(
        xilem::view::button(content, move |a: &mut App| a.toggle_folder(&key))
            .background_color(Color::TRANSPARENT)
            .border(Color::TRANSPARENT, 0.0)
            .padding(xilem::style::Padding::from(3.0)),
    )
    .expand_width()
    .boxed()
}

fn file_leaf(app: &App, r: &TreeRow, idx: usize) -> AnyView {
    let t = app.theme;
    let f = &app.diff.files[idx];
    let selected = app.selected_file == Some(idx);
    let content = flex_row((
        sized_box(label(""))
            .width((r.depth as f64 * 12.0 + 12.0).px())
            .boxed(),
        label(file_icon(&f.path)).text_size(12.0).boxed(),
        sized_box(label(ellipsize(&r.name, 22)).text_size(12.0).color(t.text)).boxed(),
        flex_spacer(),
        label(format!("+{}", f.added))
            .text_size(10.5)
            .color(t.add_fg)
            .boxed(),
        label(format!("\u{2212}{}", f.removed))
            .text_size(10.5)
            .color(t.del_fg)
            .boxed(),
    ))
    .cross_axis_alignment(CrossAxisAlignment::Center)
    .gap(4.px());

    let bg = if selected { t.sel } else { Color::TRANSPARENT };
    sized_box(
        xilem::view::button(content, move |a: &mut App| a.scroll_to_file(idx))
            .background_color(bg)
            .border(Color::TRANSPARENT, 0.0)
            .padding(xilem::style::Padding::from(3.0)),
    )
    .expand_width()
    .boxed()
}

// --- diff view --------------------------------------------------------------

fn diff_view(app: &App) -> AnyView {
    let t = app.theme;
    if let Some(err) = &app.error {
        return diff_portal(
            app,
            sized_box(label(format!("Error: {err}")).text_size(13.0).color(t.del_fg))
                .expand_width()
                .boxed(),
        );
    }

    let mut blocks: Vec<AnyView> = Vec::new();

    // Commit message (single-commit view only).
    if let Some(msg) = &app.diff.message {
        blocks.push(commit_message(t, msg));
    }

    let fs = app.settings.font_size;
    for f in &app.diff.files {
        blocks.push(file_header(t, f));
        blocks.push(file_body(app, f, fs));
        blocks.push(sized_box(label("")).height(10.px()).boxed());
    }

    diff_portal(
        app,
        sized_box(
            flex_col(blocks)
                .cross_axis_alignment(CrossAxisAlignment::Fill)
                .main_axis_alignment(MainAxisAlignment::Start)
                .gap(0.px()),
        )
        .expand_width()
        .background_color(t.bg)
        .padding(xilem::style::Padding::from(8.0))
        .boxed(),
    )
}

/// Wrap the diff body in a portal that scrolls toward the selected file. Clicking a file leaf
/// bumps `scroll_gen`; this view re-applies the approximate offset on the next rebuild.
fn diff_portal(app: &App, body: AnyView) -> AnyView {
    use crate::scroll::scroll_portal;
    scroll_portal(body, app.scroll_gen, app.scroll_fraction()).boxed()
}

fn commit_message(t: Theme, msg: &crate::git::CommitMessage) -> AnyView {
    let mut kids: Vec<AnyView> = vec![
        label(msg.title.clone())
            .weight(FontWeight::BOLD)
            .text_size(18.0)
            .color(t.text)
            .boxed(),
        label(format!("{}  \u{b7}  {}  \u{b7}  {}", msg.author, msg.date, msg.short))
            .text_size(11.0)
            .color(t.muted)
            .boxed(),
    ];
    if !msg.body.is_empty() {
        kids.push(prose(msg.body.clone()).text_color(t.text).text_size(12.0).boxed());
    }
    sized_box(
        flex_col(kids)
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .gap(6.px()),
    )
    .expand_width()
    .background_color(t.panel)
    .border(t.border, 1.0)
    .padding(xilem::style::Padding::from(12.0))
    .boxed()
}

fn file_header(t: Theme, f: &FileDiff) -> AnyView {
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
            label(label_text)
                .weight(FontWeight::BOLD)
                .text_size(12.5)
                .color(t.text)
                .boxed(),
            label(format!("[{}]", f.kind.letter()))
                .text_size(10.5)
                .color(t.muted)
                .boxed(),
            flex_spacer(),
            label(format!("+{}", f.added))
                .text_size(12.0)
                .color(t.add_fg)
                .boxed(),
            label(format!("\u{2212}{}", f.removed))
                .text_size(12.0)
                .color(t.del_fg)
                .boxed(),
        ))
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .gap(6.px()),
    )
    .expand_width()
    .background_color(t.panel)
    .border(t.border, 1.0)
    .padding(xilem::style::Padding::from(6.0))
    .boxed()
}

fn file_body(app: &App, f: &FileDiff, fs: f32) -> AnyView {
    let t = app.theme;
    if f.binary {
        return sized_box(label("Binary file not shown").text_size(fs).color(t.muted))
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
    let t = app.theme;
    let (row_bg, mark_bg, sign, sign_fg) = match kind {
        LineKind::Added => (t.add_bg, t.add_mark, '+', t.add_fg),
        LineKind::Removed => (t.del_bg, t.del_mark, '-', t.del_fg),
        LineKind::Context => (Color::TRANSPARENT, Color::TRANSPARENT, ' ', t.muted),
    };

    let num = |n: Option<u32>| -> String { n.map(|n| n.to_string()).unwrap_or_default() };

    let mut parts: Vec<AnyView> = Vec::new();

    if app.settings.line_numbers && !hunk_header {
        parts.push(
            sized_box(
                label(num(old_no))
                    .font(GenericFamily::Monospace)
                    .text_size(fs)
                    .color(t.muted),
            )
            .width((fs * 2.6).px())
            .boxed(),
        );
        parts.push(
            sized_box(
                label(num(new_no))
                    .font(GenericFamily::Monospace)
                    .text_size(fs)
                    .color(t.muted),
            )
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
                .color(t.muted)
                .boxed(),
        );
    } else if let Some(syntax) = syntax {
        let spans = app.hl.line(syntax, text);
        if spans.is_empty() {
            parts.push(label(" ").font(GenericFamily::Monospace).text_size(fs).boxed());
        } else if app.settings.word_wrap {
            // When wrapping, join into one prose block (loses per-span colour) so long lines
            // wrap; otherwise each span is its own non-wrapping label.
            let joined: String = spans.iter().map(|s| s.text.as_str()).collect();
            parts.push(prose(joined).text_color(t.text).text_size(fs).boxed());
        } else {
            for s in &spans {
                if s.text.is_empty() {
                    continue;
                }
                parts.push(
                    label(s.text.replace('\t', "    "))
                        .font(GenericFamily::Monospace)
                        .weight(if s.bold {
                            FontWeight::BOLD
                        } else {
                            FontWeight::NORMAL
                        })
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
                .color(t.text)
                .boxed(),
        );
    }

    let bg = if hunk_header { t.hunk_bg() } else { row_bg };
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

fn section_header(t: Theme, text: String) -> impl WidgetView<App> + use<> {
    sized_box(
        label(text)
            .weight(FontWeight::BOLD)
            .text_size(10.5)
            .color(t.muted),
    )
    .expand_width()
    .background_color(t.panel)
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

mod scroll;

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

    let theme = detect_theme();
    let app = App::new(repo, theme);

    Xilem::new_simple(
        app,
        app_logic,
        WindowOptions::new("git-review \u{b7} xilem")
            .with_initial_inner_size(xilem::dpi::LogicalSize::new(1280.0, 800.0))
            .with_min_inner_size(xilem::dpi::LogicalSize::new(700.0, 480.0)),
    )
    .run_in(EventLoop::with_user_event())?;
    Ok(())
}
