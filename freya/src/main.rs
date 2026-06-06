//! git-review — a GitHub-style code review desktop app, built with Freya (Skia).
//!
//! Usage: `git-review-freya [path-to-repo]` (defaults to `$GIT_REVIEW_REPO`, then the cwd).
//!
//! Architecture: a single Freya/winit process. `git2` builds the framework-agnostic diff model
//! (`git.rs`) and `syntect` highlights it (`highlight.rs`) — both run in-process. The model is
//! flattened into a plain, `Clone`-able render tree (`RenderDiff`, in `model.rs`) stored in a
//! Freya `State`, then rendered with Freya's builder API (`rect`/`label`/`paragraph`/`ScrollView`/
//! `ResizableContainer`).

mod git;
mod highlight;
mod model;

use std::rc::Rc;
use std::sync::OnceLock;

use freya::prelude::*;
use git2::Oid;

use git::{CommitInfo, Repo};
use highlight::Highlighter;
use model::{RFile, RLine, RenderDiff};

// ---------- System light/dark theme (GitHub palettes, SPEC.md) ----------
/// Whether the active palette is dark — used by `highlight.rs` to pick a syntect theme too.
static IS_DARK: OnceLock<bool> = OnceLock::new();

/// The active palette, selected once at startup from the desktop colour scheme.
static THEME: OnceLock<Theme> = OnceLock::new();

/// One full palette. Every colour is stored as a raw `(u8, u8, u8)` tuple so the UI can hand it
/// straight to Freya's `Color::from`.
#[derive(Clone, Copy)]
pub struct Theme {
    pub bg: (u8, u8, u8),
    pub panel: (u8, u8, u8),
    pub border: (u8, u8, u8),
    pub text: (u8, u8, u8),
    pub muted: (u8, u8, u8),
    pub accent: (u8, u8, u8),
    pub add_fg: (u8, u8, u8),
    pub del_fg: (u8, u8, u8),
    pub add_bg: (u8, u8, u8),
    pub add_mark: (u8, u8, u8),
    pub del_bg: (u8, u8, u8),
    pub del_mark: (u8, u8, u8),
    pub row_sel: (u8, u8, u8),
    pub ep_on: (u8, u8, u8),
    pub hunk_fg: (u8, u8, u8),
    /// hunk-header (`@@ ... @@`) row background.
    pub hunk_bg: (u8, u8, u8),
    /// "raised" surface for inset buttons (toolbar/endpoints).
    pub button_bg: (u8, u8, u8),
}

impl Theme {
    /// GitHub light.
    const LIGHT: Theme = Theme {
        bg: (255, 255, 255),
        panel: (246, 248, 250),
        border: (208, 215, 222),
        text: (31, 35, 40),
        muted: (101, 109, 118),
        accent: (9, 105, 218),
        add_fg: (26, 127, 55),
        del_fg: (207, 34, 46),
        add_bg: (230, 255, 236),
        add_mark: (171, 242, 188),
        del_bg: (255, 235, 233),
        del_mark: (255, 129, 130),
        row_sel: (221, 244, 255),
        ep_on: (9, 105, 218),
        hunk_fg: (101, 109, 118),
        hunk_bg: (241, 248, 255),
        button_bg: (255, 255, 255),
    };

    /// GitHub dark.
    const DARK: Theme = Theme {
        bg: (13, 17, 23),
        panel: (22, 27, 34),
        border: (48, 54, 61),
        text: (230, 237, 243),
        muted: (139, 148, 158),
        accent: (47, 129, 247),
        add_fg: (63, 185, 80),
        del_fg: (248, 81, 73),
        add_bg: (18, 38, 30),
        add_mark: (46, 160, 67),
        del_bg: (37, 23, 28),
        del_mark: (248, 81, 73),
        row_sel: (31, 111, 235),
        ep_on: (47, 129, 247),
        hunk_fg: (139, 148, 158),
        hunk_bg: (28, 33, 40),
        button_bg: (33, 38, 45),
    };
}

/// The active palette (set in `main`; falls back to light if accessed before init).
fn th() -> &'static Theme {
    THEME.get_or_init(|| Theme::LIGHT)
}

const MONO: &str = "monospace";

/// Shared, non-`Clone` resources held once for the lifetime of the app and reused on every
/// recompute. Stashed behind `Rc` and stored with `use_hook` so the `State`s never own them.
pub struct Engine {
    pub repo: Repo,
    pub hl: Highlighter,
    pub repo_name: String,
}

/// `provide_context` requires `Clone`; the renderer is single-threaded so the `Rc` never crosses
/// threads. The newtype keeps the bound satisfied.
#[derive(Clone)]
pub struct EngineCtx(pub Rc<Engine>);

/// What the main view is currently displaying.
#[derive(Clone, Copy, PartialEq)]
enum Showing {
    Working,
    Commit(Oid),
    Range(Oid, Oid),
}

#[derive(Clone, Copy, PartialEq)]
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
            font_size: 13.0,
            line_numbers: true,
        }
    }
}

/// A `Clone + PartialEq` commit row for the list (the raw `CommitInfo` is neither).
#[derive(Clone, PartialEq)]
struct Row {
    oid: Option<Oid>,
    short: String,
    author: String,
    date: String,
    title: String,
}

impl From<&CommitInfo> for Row {
    fn from(c: &CommitInfo) -> Self {
        Row {
            oid: c.oid,
            short: c.short.clone(),
            author: c.author.clone(),
            date: c.date.clone(),
            title: c.title.clone(),
        }
    }
}

fn recompute(engine: &Engine, showing: Showing, settings: Settings) -> RenderDiff {
    let ignore_ws = !settings.show_space;
    let res = match showing {
        Showing::Working => engine.repo.working_tree(ignore_ws),
        Showing::Commit(oid) => engine.repo.show(oid, ignore_ws),
        Showing::Range(a, b) => engine.repo.range(a, b, ignore_ws),
    };
    match res {
        Ok(set) => RenderDiff::build(set, &engine.hl),
        Err(e) => RenderDiff::error(e.message().to_string()),
    }
}

/// Pick the palette from the desktop colour scheme. `GIT_REVIEW_THEME=dark|light` overrides;
/// otherwise the `dark-light` crate is consulted, and an unspecified scheme (e.g. headless) →
/// light, per SPEC.md.
fn select_theme() -> bool {
    let dark = match std::env::var("GIT_REVIEW_THEME").ok().as_deref() {
        Some("dark") => true,
        Some("light") => false,
        _ => matches!(dark_light::detect(), Ok(dark_light::Mode::Dark)),
    };
    IS_DARK.set(dark).ok();
    let _ = THEME.set(if dark { Theme::DARK } else { Theme::LIGHT });
    dark
}

fn main() {
    // Resolve the theme before anything reads `th()`.
    select_theme();

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

    launch(
        LaunchConfig::new().with_window(
            WindowConfig::new_app(AppRoot { engine })
                .with_title("git-review · freya")
                .with_size(1280.0, 800.0)
                .with_min_size(760.0, 520.0)
                .with_background(Color::from(th().bg)),
        ),
    );
}

/// Root application component. Holds the engine and hands it to the tree via context.
struct AppRoot {
    engine: Rc<Engine>,
}

impl App for AppRoot {
    fn render(&self) -> impl IntoElement {
        provide_context(EngineCtx(self.engine.clone()));
        app()
    }
}

fn app() -> Element {
    let engine = consume_context::<EngineCtx>().0;

    // Commit list (built once).
    let rows: Vec<Row> = use_hook(|| {
        engine
            .repo
            .commits(500)
            .unwrap_or_default()
            .iter()
            .map(Row::from)
            .collect()
    });

    let initial_showing = rows
        .iter()
        .find_map(|r| r.oid.map(Showing::Commit))
        .unwrap_or(Showing::Working);

    let showing = use_state(|| initial_showing);
    let from = use_state(|| None::<Oid>);
    let to = use_state(|| None::<Oid>);
    let settings = use_state(Settings::default);
    // Folders collapsed in the file tree, keyed by their full prefix path. Empty = all expanded.
    let collapsed = use_state(std::collections::HashSet::<String>::new);

    // Scroll controller for the diff body — used for click-to-scroll from the file tree.
    let diff_scroll = use_scroll_controller(ScrollConfig::default);

    // The rendered diff is recomputed whenever the selection or whitespace flag changes.
    // (font / wrap / line-numbers are pure view tweaks and reuse the cached model.)
    let diff: Memo<RenderDiff> = {
        let engine = engine.clone();
        use_memo(move || {
            let s = *showing.read();
            let st = *settings.read();
            let _ = st.show_space; // track whitespace flag
            recompute(&engine, s, st)
        })
    };

    let st = *settings.read();
    let d = diff.read();
    let cur = *showing.read();
    let cur_from = *from.read();
    let cur_to = *to.read();

    // Approximate per-file scroll offsets (Freya has no scrollIntoView-by-id), accumulated from a
    // fixed line height. Click a file in the tree to jump near its section.
    let line_h = st.font_size + 8.0;
    let mut file_offsets: Vec<f32> = Vec::with_capacity(d.files.len());
    {
        let mut y = 0.0_f32;
        if d.message.is_some() {
            y += 130.0; // commit message block (approx)
        }
        for f in d.files.iter() {
            file_offsets.push(y);
            y += 34.0; // file header
            y += f.lines.len() as f32 * line_h;
            y += 12.0; // section gap
        }
    }

    // ----- side panel: commit list + file tree -----
    let commit_list = {
        let mut sv = ScrollView::new().spacing(2.0);
        for r in rows.iter().cloned() {
            let is_current = row_is_current(cur, &r);
            let is_from = r.oid.is_some() && r.oid == cur_from;
            let is_to = r.oid.is_some() && r.oid == cur_to;
            sv = sv.child(commit_row(
                r, is_current, is_from, is_to, showing, from, to,
            ));
        }
        rect()
            .expanded()
            .background(Color::from(th().panel))
            .child(section_head(format!(
                "COMMITS · {}",
                engine.repo_name
            )))
            .child(rect().expanded().padding(Gaps::from(6.0)).child(sv))
    };

    let file_tree = {
        // Build a real hierarchical tree from the file paths, collapsing single-child dir chains.
        let tree = build_file_tree(&d.files, &file_offsets);
        let collapsed_set = collapsed.read().clone();
        let mut rendered: Vec<Element> = Vec::new();
        for node in &tree {
            render_tree_node(node, 0, &collapsed_set, collapsed, diff_scroll, &mut rendered);
        }
        let mut sv = ScrollView::new().spacing(1.0);
        for el in rendered {
            sv = sv.child(el);
        }
        rect()
            .expanded()
            .background(Color::from(th().panel))
            .child(section_head(format!("FILES ({})", d.files.len())))
            .child(rect().expanded().padding(Gaps::from(6.0)).child(sv))
    };

    let side_panel = ResizableContainer::new()
        .direction(Direction::vertical())
        .panel(
            ResizablePanel::new(PanelSize::percent(50.0))
                .min_size(120.0)
                .child(commit_list),
        )
        .panel(
            ResizablePanel::new(PanelSize::percent(50.0))
                .min_size(120.0)
                .child(file_tree),
        );

    // ----- full-width toolbar, pinned at the very top -----
    let toolbar = build_toolbar(&d, st, settings);

    // ----- main view: just the scrollable diff body now (toolbar lives above) -----
    let body = build_diff_body(&d, st, diff_scroll);

    let main_view = rect()
        .expanded()
        .background(Color::from(th().bg))
        .child(body);

    // ----- below the toolbar: resizable [ side panel | main view ] -----
    let split = ResizableContainer::new()
        .direction(Direction::horizontal())
        .panel(
            ResizablePanel::new(PanelSize::px(330.0))
                .min_size(180.0)
                .child(side_panel),
        )
        .panel(
            ResizablePanel::new(PanelSize::px(900.0))
                .min_size(360.0)
                .child(main_view),
        );

    // ----- root: column [ toolbar ] then [ side | main ] -----
    rect()
        .expanded()
        .background(Color::from(th().bg))
        .color(Color::from(th().text))
        .font_family(default_ui_font())
        .child(toolbar)
        .child(rect().expanded().child(split))
        .into()
}

fn default_ui_font() -> &'static str {
    "Inter"
}

// ---------- small UI pieces ----------

fn section_head(text: String) -> Element {
    rect()
        .width(Size::fill())
        .padding(Gaps::from((6.0, 10.0)))
        .background(Color::from(th().panel))
        .border(
            Border::new()
                .width(1.0)
                .alignment(BorderAlignment::Inner)
                .fill(Color::from(th().border)),
        )
        .child(
            label()
                .text(text)
                .font_size(11.0)
                .font_weight(FontWeight::BOLD)
                .color(Color::from(th().muted))
                .max_lines(1),
        )
        .into()
}

#[allow(clippy::too_many_arguments)]
fn commit_row(
    row: Row,
    is_current: bool,
    is_from: bool,
    is_to: bool,
    mut showing: State<Showing>,
    mut from: State<Option<Oid>>,
    mut to: State<Option<Oid>>,
) -> Element {
    let oid = row.oid;
    let bg = if is_current {
        Color::from(th().row_sel)
    } else {
        Color::from(th().panel)
    };

    // Endpoint buttons (only for real commits).
    let line1_lead: Element = if let Some(o) = oid {
        rect()
            .horizontal()
            .spacing(3.0)
            .child(endpoint_button("◀", is_from, move || {
                from.set(Some(o));
                if let (Some(a), Some(b)) = (*from.peek(), *to.peek()) {
                    showing.set(Showing::Range(a, b));
                }
            }))
            .child(endpoint_button("▶", is_to, move || {
                to.set(Some(o));
                if let (Some(a), Some(b)) = (*from.peek(), *to.peek()) {
                    showing.set(Showing::Range(a, b));
                }
            }))
            .into()
    } else {
        rect().width(Size::px(2.0)).into()
    };

    let mut open_showing = showing;
    let open = move |_: Event<PressEventData>| {
        let next = match oid {
            Some(o) => Showing::Commit(o),
            None => Showing::Working,
        };
        if *open_showing.peek() != next {
            open_showing.set(next);
        }
    };

    rect()
        .width(Size::fill())
        .padding(Gaps::from((5.0, 7.0)))
        .corner_radius(6.0)
        .background(bg)
        .spacing(2.0)
        .on_press(open)
        .child(
            rect()
                .width(Size::fill())
                .horizontal()
                .cross_align(Alignment::Center)
                .spacing(6.0)
                .child(line1_lead)
                .child(
                    label()
                        .text(row.short)
                        .font_size(11.0)
                        .font_family(MONO)
                        .color(Color::from(th().accent))
                        .max_lines(1),
                )
                .child(
                    label()
                        .text(row.date)
                        .font_size(11.0)
                        .color(Color::from(th().muted))
                        .max_lines(1),
                )
                .child(
                    label()
                        .text(row.author)
                        .font_size(11.0)
                        .color(Color::from(th().muted))
                        .max_lines(1),
                ),
        )
        .child(
            rect()
                .width(Size::fill())
                .overflow(Overflow::Clip)
                .child(
                    label()
                        .text(row.title)
                        .font_size(13.0)
                        .color(Color::from(th().text))
                        .max_lines(1)
                        .text_overflow(TextOverflow::Ellipsis),
                ),
        )
        .into()
}

fn endpoint_button(glyph: &'static str, active: bool, on_click: impl FnMut() + 'static) -> Element {
    let (bg, fg) = if active {
        (Color::from(th().ep_on), Color::from((255, 255, 255)))
    } else {
        (Color::from(th().button_bg), Color::from(th().muted))
    };
    let mut cb = on_click;
    rect()
        .padding(Gaps::from((1.0, 5.0)))
        .corner_radius(4.0)
        .background(bg)
        .border(
            Border::new()
                .width(1.0)
                .alignment(BorderAlignment::Inner)
                .fill(Color::from(th().border)),
        )
        .on_press(move |e: Event<PressEventData>| {
            e.stop_propagation();
            cb();
        })
        .child(label().text(glyph).font_size(10.0).color(fg).max_lines(1))
        .into()
}

// ---------- hierarchical file tree ----------

/// A node in the file tree: either a folder (with its full prefix path and children) or a file
/// leaf carrying its render data and approximate scroll offset.
enum TreeNode {
    Folder {
        /// Display name — possibly a collapsed chain like `a/b/c`.
        name: String,
        /// Full path prefix used as the collapse key (e.g. `src/ui`).
        full: String,
        children: Vec<TreeNode>,
    },
    File {
        file: RFile,
        offset: f32,
    },
}

/// Mutable builder node used while constructing the tree from flat paths.
struct Build {
    name: String,
    full: String,
    children: Vec<Build>,
    leaf: Option<(RFile, f32)>,
}

impl Build {
    fn dir(name: &str, full: String) -> Self {
        Build {
            name: name.to_string(),
            full,
            children: Vec::new(),
            leaf: None,
        }
    }
}

/// Build a hierarchical tree from the diff's files (using `FileDiff.path`), pairing each leaf with
/// its approximate scroll offset, then collapse single-child directory chains GitHub-style.
fn build_file_tree(files: &[RFile], offsets: &[f32]) -> Vec<TreeNode> {
    let mut root = Build::dir("", String::new());

    for (i, f) in files.iter().enumerate() {
        let offset = offsets.get(i).copied().unwrap_or(0.0);
        let parts: Vec<&str> = f.path.split('/').collect();
        let mut cur = &mut root;
        for (depth, part) in parts.iter().enumerate() {
            let is_leaf = depth == parts.len() - 1;
            if is_leaf {
                let full = if cur.full.is_empty() {
                    part.to_string()
                } else {
                    format!("{}/{}", cur.full, part)
                };
                cur.children.push(Build {
                    name: part.to_string(),
                    full,
                    children: Vec::new(),
                    leaf: Some((f.clone(), offset)),
                });
            } else {
                let full = if cur.full.is_empty() {
                    part.to_string()
                } else {
                    format!("{}/{}", cur.full, part)
                };
                // Reuse an existing folder child or create one.
                let pos = cur
                    .children
                    .iter()
                    .position(|c| c.leaf.is_none() && c.name == *part);
                let idx = match pos {
                    Some(p) => p,
                    None => {
                        cur.children.push(Build::dir(part, full));
                        cur.children.len() - 1
                    }
                };
                cur = &mut cur.children[idx];
            }
        }
    }

    root.children.into_iter().map(finalize).collect()
}

/// Convert a builder node into a `TreeNode`, collapsing single-child folder chains.
fn finalize(mut b: Build) -> TreeNode {
    if let Some((file, offset)) = b.leaf {
        return TreeNode::File { file, offset };
    }
    // Collapse `a/` → `b/` chains while this folder has exactly one folder child and no files.
    loop {
        if b.children.len() == 1 && b.children[0].leaf.is_none() {
            let child = b.children.remove(0);
            b.name = format!("{}/{}", b.name, child.name);
            b.full = child.full;
            b.children = child.children;
        } else {
            break;
        }
    }
    let children = b.children.into_iter().map(finalize).collect();
    TreeNode::Folder {
        name: b.name,
        full: b.full,
        children,
    }
}

/// Recursively render a tree node into flat rows (Freya has no native tree widget), honouring the
/// collapsed set. `depth` drives the indentation.
fn render_tree_node(
    node: &TreeNode,
    depth: usize,
    collapsed: &std::collections::HashSet<String>,
    collapsed_state: State<std::collections::HashSet<String>>,
    scroll: ScrollController,
    out: &mut Vec<Element>,
) {
    let indent = 8.0 + depth as f32 * 14.0;
    match node {
        TreeNode::Folder {
            name,
            full,
            children,
        } => {
            let is_collapsed = collapsed.contains(full);
            out.push(folder_row(name, full, is_collapsed, indent, collapsed_state));
            if !is_collapsed {
                for c in children {
                    render_tree_node(c, depth + 1, collapsed, collapsed_state, scroll, out);
                }
            }
        }
        TreeNode::File { file, offset } => {
            out.push(file_leaf_row(file.clone(), *offset, indent, scroll));
        }
    }
}

fn folder_row(
    name: &str,
    full: &str,
    collapsed: bool,
    indent: f32,
    mut state: State<std::collections::HashSet<String>>,
) -> Element {
    let key = full.to_string();
    let disclosure = if collapsed { "▸" } else { "▾" };
    rect()
        .width(Size::fill())
        .horizontal()
        .cross_align(Alignment::Center)
        .spacing(4.0)
        .padding(Gaps::from((3.0, 6.0)))
        .corner_radius(5.0)
        .background(Color::from(th().panel))
        .on_press(move |_: Event<PressEventData>| {
            let mut w = state.write();
            if !w.insert(key.clone()) {
                w.remove(&key);
            }
        })
        .child(
            rect()
                .width(Size::px(indent))
                .main_align(Alignment::End)
                .horizontal()
                .child(
                    label()
                        .text(disclosure)
                        .font_size(10.0)
                        .color(Color::from(th().muted))
                        .max_lines(1),
                ),
        )
        .child(label().text("📁").font_size(13.0).max_lines(1))
        .child(
            label()
                .text(name.to_string())
                .font_size(12.0)
                .font_weight(FontWeight::BOLD)
                .color(Color::from(th().text))
                .max_lines(1),
        )
        .into()
}

fn file_leaf_row(f: RFile, offset: f32, indent: f32, mut scroll: ScrollController) -> Element {
    let name = f
        .path
        .rsplit('/')
        .next()
        .unwrap_or(&f.path)
        .to_string();
    rect()
        .width(Size::fill())
        .horizontal()
        .main_align(Alignment::SpaceBetween)
        .cross_align(Alignment::Center)
        .spacing(6.0)
        .padding(Gaps::from((4.0, 6.0)))
        .corner_radius(5.0)
        .background(Color::from(th().panel))
        .on_press(move |_: Event<PressEventData>| {
            scroll.scroll_to_y(-(offset as i32));
        })
        .child(
            rect()
                .horizontal()
                .cross_align(Alignment::Center)
                .spacing(6.0)
                .child(rect().width(Size::px(indent)))
                .child(label().text(f.icon).font_size(13.0).max_lines(1))
                .child(
                    label()
                        .text(name)
                        .font_size(12.0)
                        .color(Color::from(th().text))
                        .max_lines(1),
                ),
        )
        .child(
            rect()
                .horizontal()
                .spacing(5.0)
                .cross_align(Alignment::Center)
                .child(
                    label()
                        .text(format!("+{}", f.added))
                        .font_size(11.0)
                        .color(Color::from(th().add_fg))
                        .max_lines(1),
                )
                .child(
                    label()
                        .text(format!("−{}", f.removed))
                        .font_size(11.0)
                        .color(Color::from(th().del_fg))
                        .max_lines(1),
                ),
        )
        .into()
}

fn build_toolbar(d: &RenderDiff, st: Settings, settings: State<Settings>) -> Element {
    let summary = rect()
        .horizontal()
        .cross_align(Alignment::Center)
        .spacing(10.0)
        .child(
            label()
                .text(d.summary.clone())
                .font_size(13.0)
                .font_family(MONO)
                .color(Color::from(th().text))
                .max_lines(1),
        )
        .child(
            label()
                .text(format!("+{}", d.added))
                .font_size(13.0)
                .color(Color::from(th().add_fg))
                .max_lines(1),
        )
        .child(
            label()
                .text(format!("−{}", d.removed))
                .font_size(13.0)
                .color(Color::from(th().del_fg))
                .max_lines(1),
        );

    let mut s1 = settings;
    let mut s2 = settings;
    let mut s3 = settings;
    let mut s4 = settings;
    let mut s5 = settings;

    let tools = rect()
        .horizontal()
        .cross_align(Alignment::Center)
        .spacing(6.0)
        .child(tool_button(
            "⤶",
            "Word wrap",
            st.word_wrap,
            move || s1.write().word_wrap ^= true,
        ))
        .child(tool_button(
            "␣",
            "Show space changes",
            st.show_space,
            move || s2.write().show_space ^= true,
        ))
        .child(tool_button("A-", "Decrease font size", false, move || {
            let mut w = s3.write();
            w.font_size = (w.font_size - 1.0).max(8.0);
        }))
        .child(tool_button("A+", "Increase font size", false, move || {
            let mut w = s4.write();
            w.font_size = (w.font_size + 1.0).min(28.0);
        }))
        .child(tool_button(
            "#",
            "Line numbers",
            st.line_numbers,
            move || s5.write().line_numbers ^= true,
        ));

    rect()
        .width(Size::fill())
        .horizontal()
        .main_align(Alignment::SpaceBetween)
        .cross_align(Alignment::Center)
        .padding(Gaps::from((8.0, 12.0)))
        .background(Color::from(th().panel))
        .border(
            Border::new()
                .width(1.0)
                .alignment(BorderAlignment::Inner)
                .fill(Color::from(th().border)),
        )
        .child(summary)
        .child(tools)
        .into()
}

fn tool_button(
    glyph: &'static str,
    tip: &'static str,
    active: bool,
    on_click: impl FnMut() + 'static,
) -> Element {
    let (bg, fg) = if active {
        (Color::from(th().row_sel), Color::from(th().accent))
    } else {
        (Color::from(th().button_bg), Color::from(th().text))
    };
    let mut cb = on_click;
    let btn = rect()
        .min_width(Size::px(30.0))
        .padding(Gaps::from((5.0, 8.0)))
        .corner_radius(6.0)
        .cross_align(Alignment::Center)
        .main_align(Alignment::Center)
        .background(bg)
        .border(
            Border::new()
                .width(1.0)
                .alignment(BorderAlignment::Inner)
                .fill(Color::from(th().border)),
        )
        .on_press(move |_: Event<PressEventData>| cb())
        .child(
            label()
                .text(glyph)
                .font_size(13.0)
                .font_weight(FontWeight::BOLD)
                .color(fg)
                .max_lines(1),
        );

    TooltipContainer::new(Tooltip::new(tip))
        .child(btn)
        .into()
}

fn build_diff_body(d: &RenderDiff, st: Settings, scroll: ScrollController) -> Element {
    let mut content = ScrollView::new_controlled(scroll).spacing(12.0);

    if let Some(err) = &d.error {
        content = content.child(
            rect().padding(Gaps::from(16.0)).child(
                label()
                    .text(format!("Error: {err}"))
                    .font_size(14.0)
                    .color(Color::from(th().del_fg)),
            ),
        );
    } else {
        if let Some(msg) = &d.message {
            content = content.child(commit_message(msg));
        }
        if d.files.is_empty() {
            content = content.child(
                rect().padding(Gaps::from(16.0)).child(
                    label()
                        .text("No changes to show.")
                        .font_size(14.0)
                        .color(Color::from(th().muted)),
                ),
            );
        }
        for (i, f) in d.files.iter().enumerate() {
            content = content.child(file_section(i, f.clone(), st));
        }
    }

    rect()
        .expanded()
        .padding(Gaps::from(10.0))
        .background(Color::from(th().bg))
        .child(content)
        .into()
}

fn commit_message(msg: &model::RMessage) -> Element {
    let mut card = rect()
        .width(Size::fill())
        .padding(Gaps::from(12.0))
        .corner_radius(8.0)
        .spacing(4.0)
        .background(Color::from(th().panel))
        .border(
            Border::new()
                .width(1.0)
                .alignment(BorderAlignment::Inner)
                .fill(Color::from(th().border)),
        )
        .child(
            label()
                .width(Size::fill())
                .text(msg.title.clone())
                .font_size(16.0)
                .font_weight(FontWeight::BOLD)
                .color(Color::from(th().text)),
        )
        .child(
            label()
                .text(format!("{}  ·  {}  ·  {}", msg.author, msg.date, msg.short))
                .font_size(12.0)
                .color(Color::from(th().muted))
                .max_lines(1),
        );

    if !msg.body.is_empty() {
        card = card.child(
            label()
                .width(Size::fill())
                .text(msg.body.clone())
                .font_size(13.0)
                .font_family(MONO)
                .color(Color::from(th().text)),
        );
    }
    card.into()
}

fn file_section(idx: usize, file: RFile, st: Settings) -> Element {
    // Per-file header. Freya has no native sticky positioning; this is a regular header that
    // scrolls with its section (documented limitation — see README).
    let header = rect()
        .width(Size::fill())
        .horizontal()
        .main_align(Alignment::SpaceBetween)
        .cross_align(Alignment::Center)
        .spacing(8.0)
        .padding(Gaps::from((6.0, 10.0)))
        .background(Color::from(th().panel))
        .border(
            Border::new()
                .width(1.0)
                .alignment(BorderAlignment::Inner)
                .fill(Color::from(th().border)),
        )
        .child(
            rect()
                .horizontal()
                .cross_align(Alignment::Center)
                .spacing(8.0)
                .child(label().text(file.icon).font_size(13.0).max_lines(1))
                .child(
                    label()
                        .text(file.header_label.clone())
                        .font_size(13.0)
                        .font_weight(FontWeight::BOLD)
                        .font_family(MONO)
                        .color(Color::from(th().text))
                        .max_lines(1),
                ),
        )
        .child(
            rect()
                .horizontal()
                .spacing(6.0)
                .cross_align(Alignment::Center)
                .child(
                    label()
                        .text(format!("[{}]", file.kind_letter))
                        .font_size(11.0)
                        .color(Color::from(th().muted))
                        .max_lines(1),
                )
                .child(
                    label()
                        .text(format!("+{}", file.added))
                        .font_size(12.0)
                        .color(Color::from(th().add_fg))
                        .max_lines(1),
                )
                .child(
                    label()
                        .text(format!("−{}", file.removed))
                        .font_size(12.0)
                        .color(Color::from(th().del_fg))
                        .max_lines(1),
                ),
        );

    let mut section = rect()
        .key(idx)
        .width(Size::fill())
        .corner_radius(8.0)
        .overflow(Overflow::Clip)
        .border(
            Border::new()
                .width(1.0)
                .alignment(BorderAlignment::Inner)
                .fill(Color::from(th().border)),
        )
        .background(Color::from(th().bg))
        .child(header);

    if file.binary {
        section = section.child(
            rect().padding(Gaps::from(12.0)).child(
                label()
                    .text("Binary file not shown")
                    .font_size(13.0)
                    .color(Color::from(th().muted)),
            ),
        );
    } else {
        let mut code = rect();
        for (li, line) in file.lines.iter().enumerate() {
            code = code.child(diff_line(li, line, st));
        }
        if st.word_wrap {
            // Wrapped: code fills the viewport width, long lines wrap.
            section = section.child(code.width(Size::fill()));
        } else {
            // No wrap: long lines extend; a horizontal ScrollView keeps the header/counts pinned
            // to the viewport while the code scrolls sideways.
            section = section.child(
                ScrollView::new()
                    .direction(Direction::horizontal())
                    .height(Size::Inner)
                    .show_scrollbar(true)
                    .child(code),
            );
        }
    }

    section.into()
}

fn diff_line(li: usize, line: &RLine, st: Settings) -> Element {
    use git::LineKind;

    let fs = st.font_size;

    if line.is_hunk_header {
        let text = line
            .spans
            .first()
            .map(|s| s.text.clone())
            .unwrap_or_default();
        return rect()
            .key(li)
            .width(if st.word_wrap { Size::fill() } else { Size::Inner })
            .min_width(Size::fill())
            .padding(Gaps::from((2.0, 8.0)))
            .background(Color::from(th().hunk_bg))
            .child(
                label()
                    .text(text)
                    .font_size(fs)
                    .font_family(MONO)
                    .color(Color::from(th().hunk_fg))
                    .max_lines(1),
            )
            .into();
    }

    let (row_bg, mark_bg, sign) = match line.kind {
        LineKind::Added => (Color::from(th().add_bg), Color::from(th().add_mark), "+"),
        LineKind::Removed => (Color::from(th().del_bg), Color::from(th().del_mark), "-"),
        LineKind::Context => (Color::from(th().bg), Color::from(th().bg), " "),
    };

    // Wrap mode: rows fill the viewport (text wraps). No-wrap: rows size to content so the
    // enclosing horizontal ScrollView can pan over long lines.
    let row_width = if st.word_wrap { Size::fill() } else { Size::Inner };
    let mut row = rect()
        .key(li)
        .width(row_width)
        .horizontal()
        .background(row_bg);

    // Line-number gutter (toggleable).
    if st.line_numbers {
        let old = line.old_no.map(|n| n.to_string()).unwrap_or_default();
        let new = line.new_no.map(|n| n.to_string()).unwrap_or_default();
        row = row
            .child(gutter_cell(old, fs))
            .child(gutter_cell(new, fs));
    }

    // Change marker column (stronger green/red).
    row = row.child(
        rect()
            .width(Size::px(fs))
            .background(mark_bg)
            .cross_align(Alignment::Center)
            .child(
                label()
                    .text(sign)
                    .font_size(fs)
                    .font_family(MONO)
                    .color(Color::from(th().muted))
                    .max_lines(1),
            ),
    );

    // Highlighted line text via a paragraph of styled spans.
    let mut para = paragraph().max_lines(if st.word_wrap { None } else { Some(1) });
    if line.spans.is_empty() {
        para = para.span(Span::new(" ").font_size(fs).font_family(MONO));
    } else {
        for sp in line.spans.iter() {
            let mut span = Span::new(sp.text.clone())
                .font_size(fs)
                .font_family(MONO)
                .color(Color::from(sp.color));
            if sp.bold {
                span = span.font_weight(FontWeight::BOLD);
            }
            if sp.italic {
                span = span.font_slant(FontSlant::Italic);
            }
            para = para.span(span);
        }
    }

    let text_width = if st.word_wrap { Size::fill() } else { Size::Inner };
    row = row.child(
        rect()
            .width(text_width)
            .padding(Gaps::from((1.0, 6.0)))
            .child(para),
    );

    row.into()
}

fn gutter_cell(text: String, fs: f32) -> Element {
    rect()
        .width(Size::px(fs * 3.0))
        .padding(Gaps::from((1.0, 4.0)))
        .background(Color::from(th().panel))
        .cross_align(Alignment::End)
        .child(
            label()
                .text(text)
                .font_size(fs - 1.0)
                .font_family(MONO)
                .color(Color::from(th().muted))
                .max_lines(1),
        )
        .into()
}

fn row_is_current(cur: Showing, r: &Row) -> bool {
    match (cur, r.oid) {
        (Showing::Working, None) => true,
        (Showing::Commit(o), Some(oid)) => o == oid,
        _ => false,
    }
}
