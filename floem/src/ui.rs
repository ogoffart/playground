//! The Floem UI for git-review.
//!
//! Floem is a fine-grained reactive UI library. State lives in `RwSignal`s; views subscribe to
//! the signals they read and re-render automatically when those signals change. The whole view
//! tree is built *once* — dynamic parts (the diff body, commit list, file tree) are rebuilt
//! through `dyn_container` / `dyn_stack` keyed on the signals that drive them.

use std::rc::Rc;

use floem::event::{Event, EventListener};
use floem::keyboard::Modifiers;
use floem::kurbo::{Point, Stroke};
use floem::peniko::Color;
use floem::reactive::{create_rw_signal, RwSignal, SignalGet, SignalUpdate, SignalWith};
use floem::text::{Attrs, AttrsList, TextLayout, Weight};
use floem::views::{
    container, dyn_container, dyn_stack, empty, h_stack, label, rich_text, scroll,
    v_stack, Decorators,
};
use floem::window::WindowConfig;
use floem::{IntoView, View, ViewId};

use git2::Oid;

use crate::git::{ChangeKind, CommitInfo, CommitMessage, DiffSet, FileDiff, LineKind, Repo};
use crate::highlight::Highlighter;

// --- GitHub-ish palette -----------------------------------------------------
fn rgb(hex: u32) -> Color {
    Color::rgb8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}
fn col(c: (u8, u8, u8)) -> Color {
    Color::rgb8(c.0, c.1, c.2)
}

/// The full runtime colour palette. `Copy` so style closures can capture it cheaply.
/// Chosen at startup from the desktop light/dark preference.
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
    del_bg: Color,
    del_mark: Color,
    add_fg: Color,
    del_fg: Color,
    hunk_bg: Color,
    hover: Color,
    /// Default syntax-highlight foreground, used as fallback for un-styled spans.
    code_fg: (u8, u8, u8),
}

impl Theme {
    fn light() -> Self {
        Theme {
            dark: false,
            bg: rgb(0xffffff),
            panel: rgb(0xf6f8fa),
            border: rgb(0xd0d7de),
            text: rgb(0x1f2328),
            muted: rgb(0x656d76),
            accent: rgb(0x0969da),
            sel: rgb(0xddf4ff),
            add_bg: rgb(0xe6ffec),
            add_mark: rgb(0xabf2bc),
            del_bg: rgb(0xffebe9),
            del_mark: rgb(0xff8182),
            add_fg: rgb(0x1a7f37),
            del_fg: rgb(0xcf222e),
            hunk_bg: rgb(0xddf4ff),
            hover: rgb(0xeef1f4),
            code_fg: (0x1f, 0x23, 0x28),
        }
    }

    fn dark() -> Self {
        Theme {
            dark: true,
            bg: rgb(0x0d1117),
            panel: rgb(0x161b22),
            border: rgb(0x30363d),
            text: rgb(0xe6edf3),
            muted: rgb(0x8b949e),
            accent: rgb(0x2f81f7),
            sel: rgb(0x1f6feb),
            add_bg: rgb(0x12261e),
            add_mark: rgb(0x2ea043),
            del_bg: rgb(0x25171c),
            del_mark: rgb(0xf85149),
            add_fg: rgb(0x3fb950),
            del_fg: rgb(0xf85149),
            hunk_bg: rgb(0x1f2937),
            hover: rgb(0x21262d),
            code_fg: (0xe6, 0xed, 0xf3),
        }
    }
}

/// Detect the desktop colour scheme. Honors `GIT_REVIEW_THEME` override; headless => light.
fn detect_theme() -> Theme {
    if let Ok(v) = std::env::var("GIT_REVIEW_THEME") {
        match v.to_ascii_lowercase().as_str() {
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

const MONO: &str = "ui-monospace, SF Mono, Menlo, Consolas, monospace";

#[derive(Clone, Copy, PartialEq)]
enum Showing {
    Working,
    Commit(Oid),
    Range(Oid, Oid),
}

/// All app state, held as reactive signals. `Copy` signals are cheap to clone into closures.
#[derive(Clone)]
struct AppState {
    repo: Rc<Repo>,
    hl: Rc<Highlighter>,
    theme: Theme,
    repo_name: String,
    commits: Rc<Vec<CommitInfo>>,

    showing: RwSignal<Showing>,
    from: RwSignal<Option<Oid>>,
    to: RwSignal<Option<Oid>>,

    // The computed diff. Wrapped in Rc so the (large) value is cheap to clone out of the signal.
    diff: RwSignal<Rc<DiffSet>>,
    error: RwSignal<Option<String>>,

    // Settings
    word_wrap: RwSignal<bool>,
    show_space: RwSignal<bool>,
    font_size: RwSignal<f64>,
    line_numbers: RwSignal<bool>,

    // Layout sizes (resizable panels)
    side_width: RwSignal<f64>,
    commits_height: RwSignal<f64>,

    // ViewIds of each file header in the diff body, so the file tree can scroll to them.
    file_ids: RwSignal<Rc<Vec<ViewId>>>,
    scroll_target: RwSignal<Option<ViewId>>,

    // File-tree expansion: the set of folder keys that are *collapsed* (expanded by default).
    collapsed: RwSignal<std::collections::HashSet<String>>,
}

impl AppState {
    fn recompute(&self) {
        let ignore_ws = !self.show_space.get();
        let res = match self.showing.get() {
            Showing::Working => self.repo.working_tree(ignore_ws),
            Showing::Commit(oid) => self.repo.show(oid, ignore_ws),
            Showing::Range(a, b) => self.repo.range(a, b, ignore_ws),
        };
        match res {
            Ok(d) => {
                self.diff.set(Rc::new(d));
                self.error.set(None);
            }
            Err(e) => self.error.set(Some(e.message().to_string())),
        }
    }

    fn select(&self, showing: Showing) {
        if self.showing.get() != showing {
            self.showing.set(showing);
            self.recompute();
        }
    }

    fn maybe_range(&self) {
        if let (Some(a), Some(b)) = (self.from.get(), self.to.get()) {
            self.select(Showing::Range(a, b));
        }
    }
}

pub fn launch_app(repo: Repo) {
    let theme = detect_theme();
    let hl = if theme.dark {
        Highlighter::with_dark()
    } else {
        Highlighter::new()
    };
    let repo_name = repo.workdir_name();
    let commits = repo.commits(500).unwrap_or_default();
    let showing = commits
        .iter()
        .find_map(|c| c.oid.map(Showing::Commit))
        .unwrap_or(Showing::Working);

    let state = AppState {
        repo: Rc::new(repo),
        hl: Rc::new(hl),
        theme,
        repo_name,
        commits: Rc::new(commits),
        showing: create_rw_signal(showing),
        from: create_rw_signal(None),
        to: create_rw_signal(None),
        diff: create_rw_signal(Rc::new(empty_diff())),
        error: create_rw_signal(None),
        word_wrap: create_rw_signal(false),
        show_space: create_rw_signal(true),
        font_size: create_rw_signal(12.0),
        line_numbers: create_rw_signal(true),
        side_width: create_rw_signal(320.0),
        commits_height: create_rw_signal(360.0),
        file_ids: create_rw_signal(Rc::new(Vec::new())),
        scroll_target: create_rw_signal(None),
        collapsed: create_rw_signal(std::collections::HashSet::new()),
    };
    state.recompute();

    let cfg = WindowConfig::default()
        .size((1280.0, 800.0))
        .title("git-review · floem");

    floem::Application::new()
        .window(move |_| app_view(state.clone()), Some(cfg))
        .run();
}

fn app_view(state: AppState) -> impl IntoView {
    let th = state.theme;
    let main = diff_view(state.clone())
        .style(|s| s.flex_grow(1.0).flex_basis(0.0).min_width(0.0).height_full());

    // Full-width toolbar pinned at the very top, above the side panel and the diff.
    let body = h_stack((side_panel(state.clone()), main))
        .style(|s| s.flex_grow(1.0).flex_basis(0.0).width_full().min_height(0.0));

    v_stack((toolbar(state.clone()), body)).style(move |s| {
        s.size_full()
            .flex_col()
            .background(th.bg)
            .color(th.text)
            .font_size(13.0)
    })
}

// --- side panel (resizable: commit list over file tree) ---------------------

fn side_panel(state: AppState) -> impl IntoView {
    let st = state.clone();
    let th = state.theme;
    let panel = v_stack((commit_list(state.clone()), v_splitter(state.clone()), file_tree(state.clone())))
        .style(move |s| {
            s.width(st.side_width.get())
                .height_full()
                .background(th.panel)
                .border_right(1.0)
                .border_color(th.border)
                .flex_col()
        });

    h_stack((panel, h_splitter(state)))
        .style(|s| s.height_full())
}

/// A vertical drag-handle that resizes the side panel width.
fn h_splitter(state: AppState) -> impl IntoView {
    let dragging = create_rw_signal(false);
    let last = create_rw_signal(0.0f64);
    let st = state.clone();
    let th = state.theme;
    empty()
        .style(move |s| {
            s.width(6.0)
                .height_full()
                .background(if dragging.get() { th.accent } else { Color::TRANSPARENT })
                .cursor(floem::style::CursorStyle::ColResize)
                .hover(move |s| s.background(th.border))
        })
        .on_event_stop(EventListener::PointerDown, move |e| {
            if let Event::PointerDown(ev) = e {
                dragging.set(true);
                last.set(window_x(ev.pos, ev.modifiers));
            }
        })
        .on_event_stop(EventListener::PointerMove, move |e| {
            if dragging.get() {
                if let Event::PointerMove(ev) = e {
                    let x = window_x(ev.pos, ev.modifiers);
                    let dx = x - last.get();
                    last.set(x);
                    let w = (st.side_width.get() + dx).clamp(200.0, 640.0);
                    st.side_width.set(w);
                }
            }
        })
        .on_event_stop(EventListener::PointerUp, move |_| dragging.set(false))
}

/// A horizontal drag-handle that resizes the commit-list height inside the side panel.
fn v_splitter(state: AppState) -> impl IntoView {
    let dragging = create_rw_signal(false);
    let last = create_rw_signal(0.0f64);
    let st = state.clone();
    let th = state.theme;
    empty()
        .style(move |s| {
            s.height(6.0)
                .width_full()
                .background(if dragging.get() { th.accent } else { th.border })
                .cursor(floem::style::CursorStyle::RowResize)
                .hover(move |s| s.background(th.accent))
        })
        .on_event_stop(EventListener::PointerDown, move |e| {
            if let Event::PointerDown(ev) = e {
                dragging.set(true);
                last.set(ev.pos.y);
            }
        })
        .on_event_stop(EventListener::PointerMove, move |e| {
            if dragging.get() {
                if let Event::PointerMove(ev) = e {
                    let dy = ev.pos.y;
                    let h = (st.commits_height.get() + dy).clamp(120.0, 700.0);
                    st.commits_height.set(h);
                }
            }
        })
        .on_event_stop(EventListener::PointerUp, move |_| dragging.set(false))
}

// `pos` for pointer events is local to the handle. Since the handle barely moves horizontally,
// using the local x and tracking deltas between consecutive moves is stable enough. The modifier
// param is unused but kept to mirror the event shape.
fn window_x(pos: Point, _m: Modifiers) -> f64 {
    pos.x
}

// --- commit list ------------------------------------------------------------

fn commit_list(state: AppState) -> impl IntoView {
    let header = section_header(state.theme, format!("COMMITS · {}", state.repo_name));

    let st = state.clone();
    let commits = state.commits.clone();
    let rows = dyn_stack(
        move || commits.iter().cloned().enumerate().collect::<Vec<_>>(),
        |(i, _)| *i,
        move |(_, c)| commit_row(st.clone(), c),
    )
    .style(|s| s.flex_col().width_full());

    let h = state.commits_height;
    v_stack((header, scroll(rows).style(|s| s.flex_grow(1.0).flex_basis(0.0).width_full())))
        .style(move |s| s.height(h.get()).width_full().flex_col())
}

fn commit_row(state: AppState, c: CommitInfo) -> impl IntoView {
    let th = state.theme;
    let oid = c.oid;
    let is_wt = c.is_working_tree();

    // current-row highlight
    let st_cur = state.clone();
    let highlight = move || match (st_cur.showing.get(), oid) {
        (Showing::Working, _) if is_wt => true,
        (Showing::Commit(o), Some(x)) => o == x,
        _ => false,
    };

    // line 1: endpoint buttons + sha + date + author
    let from_sig = state.from;
    let to_sig = state.to;

    let st_f = state.clone();
    let from_btn = endpoint_button(th, "◀", "Compare from this commit", move || {
        oid.is_some() && oid == from_sig.get()
    }, move || {
        if let Some(o) = oid {
            st_f.from.set(Some(o));
            st_f.maybe_range();
        }
    }, oid.is_some());

    let st_t = state.clone();
    let to_btn = endpoint_button(th, "▶", "Compare to this commit", move || {
        oid.is_some() && oid == to_sig.get()
    }, move || {
        if let Some(o) = oid {
            st_t.to.set(Some(o));
            st_t.maybe_range();
        }
    }, oid.is_some());

    let sha = label(move || c.short.clone())
        .style(move |s| s.color(th.accent).font_family(MONO.to_string()).font_size(11.0));
    let date = label({
        let d = c.date.clone();
        move || d.clone()
    })
    .style(move |s| s.color(th.muted).font_size(11.0));
    let author = label({
        let a = c.author.clone();
        move || a.clone()
    })
    .style(move |s| {
        s.color(th.muted)
            .font_size(11.0)
            .flex_grow(1.0)
            .flex_basis(0.0)
            .min_width(0.0)
            .justify_end()
            .text_ellipsis()
    });

    let line1 = h_stack((from_btn, to_btn, sha, date, author))
        .style(|s| s.items_center().gap(6.0).width_full().min_width(0.0));

    // line 2: title
    let title = label({
        let t = c.title.clone();
        move || t.clone()
    })
    .style(move |s| s.color(th.text).font_size(12.0).width_full().min_width(0.0).text_ellipsis());

    let st_click = state.clone();
    v_stack((line1, title))
        .style(move |s| {
            s.width_full()
                .min_width(0.0)
                .padding_horiz(8.0)
                .padding_vert(5.0)
                .gap(2.0)
                .border_bottom(1.0)
                .border_color(th.border)
                .apply_if(highlight(), |s| s.background(th.sel).apply_if(th.dark, |s| s.color(rgb(0xffffff))))
                .hover(move |s| s.background(th.hover))
        })
        .on_click_stop(move |_| match oid {
            Some(o) => st_click.select(Showing::Commit(o)),
            None => st_click.select(Showing::Working),
        })
}

// --- file tree --------------------------------------------------------------

/// A flattened row of the hierarchical file tree, ready to render.
#[derive(Clone)]
struct TreeRow {
    depth: usize,
    /// A folder node: its stable key (full path prefix) and display name.
    folder: Option<(String, String)>,
    /// A file leaf: display name, the index of the file in `DiffSet::files`, +added/-removed.
    file: Option<(String, usize, u32, u32)>,
}

/// Intermediate tree built from the flat `FileDiff` list.
struct TreeNode {
    name: String,
    /// `Some(file_index)` if this is a leaf (a file), else it's a directory.
    file_index: Option<usize>,
    added: u32,
    removed: u32,
    children: Vec<TreeNode>,
}

impl TreeNode {
    fn dir(name: String) -> Self {
        TreeNode { name, file_index: None, added: 0, removed: 0, children: Vec::new() }
    }
}

/// Build a hierarchical tree from the diff's flat list of file paths.
fn build_tree(files: &[FileDiff]) -> Vec<TreeNode> {
    let mut root = TreeNode::dir(String::new());
    for (idx, f) in files.iter().enumerate() {
        let comps: Vec<&str> = f.path.split('/').filter(|c| !c.is_empty()).collect();
        let mut cur = &mut root;
        for (i, comp) in comps.iter().enumerate() {
            let is_leaf = i + 1 == comps.len();
            if is_leaf {
                cur.children.push(TreeNode {
                    name: (*comp).to_string(),
                    file_index: Some(idx),
                    added: f.added,
                    removed: f.removed,
                    children: Vec::new(),
                });
            } else {
                // find or create the directory child
                let pos = cur
                    .children
                    .iter()
                    .position(|c| c.file_index.is_none() && c.name == *comp);
                cur = match pos {
                    Some(p) => &mut cur.children[p],
                    None => {
                        cur.children.push(TreeNode::dir((*comp).to_string()));
                        cur.children.last_mut().unwrap()
                    }
                };
            }
        }
    }
    let mut roots = root.children;
    collapse_chains(&mut roots);
    roots
}

/// Collapse single-child directory chains GitHub-style: `a/` containing only `b/` becomes `a/b/`.
fn collapse_chains(nodes: &mut Vec<TreeNode>) {
    for node in nodes.iter_mut() {
        if node.file_index.is_none() {
            // First recurse so inner chains are collapsed.
            collapse_chains(&mut node.children);
            // Then fold this node into its single directory child.
            while node.file_index.is_none()
                && node.children.len() == 1
                && node.children[0].file_index.is_none()
            {
                let mut child = node.children.remove(0);
                node.name = format!("{}/{}", node.name, child.name);
                node.children = std::mem::take(&mut child.children);
            }
        }
    }
}

/// Flatten the tree into visible rows, honoring the set of collapsed folder keys.
fn flatten_tree(
    nodes: &[TreeNode],
    depth: usize,
    prefix: &str,
    collapsed: &std::collections::HashSet<String>,
    out: &mut Vec<TreeRow>,
) {
    for node in nodes {
        if let Some(fi) = node.file_index {
            out.push(TreeRow {
                depth,
                folder: None,
                file: Some((node.name.clone(), fi, node.added, node.removed)),
            });
        } else {
            let key = if prefix.is_empty() {
                node.name.clone()
            } else {
                format!("{prefix}/{}", node.name)
            };
            out.push(TreeRow {
                depth,
                folder: Some((key.clone(), node.name.clone())),
                file: None,
            });
            if !collapsed.contains(&key) {
                flatten_tree(&node.children, depth + 1, &key, collapsed, out);
            }
        }
    }
}

fn file_tree(state: AppState) -> impl IntoView {
    let th = state.theme;
    let st_h = state.clone();
    let header = dyn_container(
        move || st_h.diff.with(|d| d.files.len()),
        move |n| section_header(th, format!("FILES ({n})")),
    );

    let st = state.clone();
    let rows = dyn_stack(
        move || {
            // Re-flatten whenever the diff or the collapsed set changes.
            let collapsed = st.collapsed.get();
            let tree = st.diff.with(|d| build_tree(&d.files));
            let mut out = Vec::new();
            flatten_tree(&tree, 0, "", &collapsed, &mut out);
            out.into_iter().enumerate().collect::<Vec<_>>()
        },
        // Key by content so toggles re-render correctly.
        |(i, r): &(usize, TreeRow)| {
            let tag = match (&r.folder, &r.file) {
                (Some((k, _)), _) => format!("d:{k}"),
                (_, Some((_, fi, _, _))) => format!("f:{fi}"),
                _ => String::new(),
            };
            (*i, tag)
        },
        {
            let st2 = state.clone();
            move |(_, r)| tree_row(st2.clone(), r)
        },
    )
    .style(|s| s.flex_col().width_full());

    v_stack((header, scroll(rows).style(|s| s.flex_grow(1.0).flex_basis(0.0).width_full())))
        .style(|s| s.flex_grow(1.0).flex_basis(0.0).width_full().flex_col())
}

fn tree_row(state: AppState, row: TreeRow) -> impl IntoView {
    let th = state.theme;
    let indent = 8.0 + row.depth as f64 * 14.0;

    if let Some((key, name)) = row.folder.clone() {
        // Folder row: disclosure triangle + folder icon + name.
        let st_tri = state.clone();
        let key_tri = key.clone();
        let triangle = label(move || {
            if st_tri.collapsed.with(|c| c.contains(&key_tri)) { "▸".to_string() } else { "▾".to_string() }
        })
        .style(move |s| s.color(th.muted).font_size(10.0).width(12.0));

        let folder_icon = label(|| "📁".to_string()).style(|s| s.font_size(13.0));
        let nm = name.clone();
        let folder_name = label(move || nm.clone()).style(move |s| {
            s.color(th.text)
                .font_size(12.0)
                .font_weight(Weight::SEMIBOLD)
                .flex_grow(1.0)
                .flex_basis(0.0)
                .min_width(0.0)
                .text_ellipsis()
        });

        let st = state.clone();
        return h_stack((triangle, folder_icon, folder_name))
            .style(move |s| {
                s.items_center()
                    .gap(5.0)
                    .width_full()
                    .min_width(0.0)
                    .padding_left(indent)
                    .padding_right(8.0)
                    .padding_vert(3.0)
                    .hover(move |s| s.background(th.hover))
                    .cursor(floem::style::CursorStyle::Pointer)
            })
            .on_click_stop(move |_| {
                st.collapsed.update(|c| {
                    if !c.remove(&key) {
                        c.insert(key.clone());
                    }
                });
            })
            .into_any();
    }

    // File leaf row: icon + file NAME + +a −r counts.
    let (name, idx, added, removed) = row.file.clone().unwrap();
    let icon = label({
        let g = file_icon(&name);
        move || g.to_string()
    })
    .style(|s| s.font_size(13.0).width(14.0));

    let nm = name.clone();
    let label_name = label(move || nm.clone()).style(move |s| {
        s.color(th.text)
            .font_size(12.0)
            .flex_grow(1.0)
            .flex_basis(0.0)
            .min_width(0.0)
            .text_ellipsis()
    });

    let plus = label(move || format!("+{added}")).style(move |s| s.color(th.add_fg).font_size(11.0));
    let minus = label(move || format!("−{removed}")).style(move |s| s.color(th.del_fg).font_size(11.0));

    let st = state.clone();
    h_stack((icon, label_name, plus, minus))
        .style(move |s| {
            s.items_center()
                .gap(6.0)
                .width_full()
                .min_width(0.0)
                .padding_left(indent)
                .padding_right(8.0)
                .padding_vert(3.0)
                .hover(move |s| s.background(th.hover))
                .cursor(floem::style::CursorStyle::Pointer)
        })
        .on_click_stop(move |_| {
            if let Some(id) = st.file_ids.with(|ids| ids.get(idx).copied()) {
                st.scroll_target.set(Some(id));
            }
        })
        .into_any()
}

// --- toolbar ----------------------------------------------------------------

fn toolbar(state: AppState) -> impl IntoView {
    let th = state.theme;
    let st_sum = state.clone();
    let summary = label(move || st_sum.diff.with(|d| d.summary.clone()))
        .style(move |s| s.font_family(MONO.to_string()).font_weight(Weight::BOLD).color(th.text).font_size(12.0));

    let st_a = state.clone();
    let added = label(move || format!("+{}", st_a.diff.with(|d| d.added)))
        .style(move |s| s.color(th.add_fg).font_size(12.0));
    let st_r = state.clone();
    let removed = label(move || format!("−{}", st_r.diff.with(|d| d.removed)))
        .style(move |s| s.color(th.del_fg).font_size(12.0));

    let spacer = empty().style(|s| s.flex_grow(1.0));

    // tool buttons
    let ww = state.word_wrap;
    let wrap_btn = tool_button(th, "⤶", move || {
        if ww.get() { "Word wrap: on" } else { "Word wrap: off" }
    }, move || ww.get(), move || ww.set(!ww.get()));

    let st_sp = state.clone();
    let sp = state.show_space;
    let space_btn = tool_button(th, "␣", move || {
        if sp.get() { "Show space changes: on" } else { "Show space changes: off (whitespace ignored)" }
    }, move || sp.get(), move || {
        sp.set(!sp.get());
        st_sp.recompute();
    });

    let fs = state.font_size;
    let dec_btn = tool_button(th, "A-", || "Decrease font size", || false, move || {
        fs.set((fs.get() - 1.0).max(8.0));
    });
    let inc_btn = tool_button(th, "A+", || "Increase font size", || false, move || {
        fs.set((fs.get() + 1.0).min(28.0));
    });

    let ln = state.line_numbers;
    let num_btn = tool_button(th, "#", move || {
        if ln.get() { "Line numbers: on" } else { "Line numbers: off" }
    }, move || ln.get(), move || ln.set(!ln.get()));

    h_stack((
        summary, added, removed, spacer,
        wrap_btn, space_btn, dec_btn, inc_btn, num_btn,
    ))
    .style(move |s| {
        s.items_center()
            .gap(8.0)
            .width_full()
            .flex_shrink(0.0)
            .padding_horiz(10.0)
            .padding_vert(7.0)
            .background(th.panel)
            .border_bottom(1.0)
            .border_color(th.border)
    })
}

fn tool_button(
    th: Theme,
    glyph: &'static str,
    tip: impl Fn() -> &'static str + 'static,
    active: impl Fn() -> bool + 'static + Copy,
    on_click: impl Fn() + 'static,
) -> impl IntoView {
    use floem::views::TooltipExt;
    label(move || glyph.to_string())
        .style(move |s| {
            let a = active();
            s.font_family(MONO.to_string())
                .font_size(13.0)
                .padding_horiz(7.0)
                .padding_vert(3.0)
                .border(1.0)
                .border_color(th.border)
                .border_radius(5.0)
                .cursor(floem::style::CursorStyle::Pointer)
                .apply_if(a, |s| s.background(th.sel).color(if th.dark { rgb(0xffffff) } else { th.accent }).font_weight(Weight::BOLD))
                .apply_if(!a, |s| s.background(th.bg).color(th.text))
                .hover(move |s| s.background(th.hover))
        })
        .on_click_stop(move |_| on_click())
        .tooltip(move || {
            let txt = tip();
            label(move || txt.to_string())
                .style(|s| s.padding(5.0).background(rgb(0x24292f)).color(rgb(0xffffff)).border_radius(4.0).font_size(11.0))
        })
}

fn endpoint_button(
    th: Theme,
    glyph: &'static str,
    tip: &'static str,
    active: impl Fn() -> bool + 'static + Copy,
    on_click: impl Fn() + 'static,
    enabled: bool,
) -> impl IntoView {
    use floem::views::TooltipExt;
    label(move || glyph.to_string())
        .style(move |s| {
            let s = s
                .font_size(10.0)
                .padding_horiz(4.0)
                .padding_vert(1.0)
                .border_radius(4.0)
                .min_width(16.0)
                .justify_center();
            if !enabled {
                return s.color(Color::TRANSPARENT).background(Color::TRANSPARENT);
            }
            let a = active();
            s.cursor(floem::style::CursorStyle::Pointer)
                .apply_if(a, |s| s.background(th.accent).color(rgb(0xffffff)))
                .apply_if(!a, |s| s.background(th.border).color(th.muted))
        })
        .on_click_stop(move |_| {
            if enabled {
                on_click()
            }
        })
        .tooltip(move || {
            label(move || tip.to_string())
                .style(|s| s.padding(5.0).background(rgb(0x24292f)).color(rgb(0xffffff)).border_radius(4.0).font_size(11.0))
        })
}

// --- diff view --------------------------------------------------------------

fn diff_view(state: AppState) -> impl IntoView {
    let st = state.clone();
    // Rebuild the entire body whenever the diff, settings, or font change.
    let body = dyn_container(
        move || {
            // subscribe to everything that affects layout/content
            let _ = st.font_size.get();
            let _ = st.word_wrap.get();
            let _ = st.line_numbers.get();
            st.diff.get()
        },
        {
            let st = state.clone();
            move |diff| diff_body(st.clone(), diff).into_any()
        },
    );

    let th = state.theme;
    let st_err = state.clone();
    let error = dyn_container(
        move || st_err.error.get(),
        move |err| match err {
            Some(e) => label(move || format!("Error: {e}"))
                .style(move |s| s.color(th.del_fg).padding(12.0))
                .into_any(),
            None => empty().into_any(),
        },
    );

    let st_scroll = state.clone();
    let ww = state.word_wrap;
    scroll(v_stack((error, body)).style(|s| s.flex_col().min_width(0.0)))
        .scroll_to_view(move || st_scroll.scroll_target.get())
        .style(move |s| {
            // word-wrap off => allow horizontal scrolling (children keep natural width)
            s.flex_grow(1.0)
                .flex_basis(0.0)
                .min_width(0.0)
                .width_full()
                .height_full()
                .background(th.bg)
                .apply_if(ww.get(), |s| s.flex_col())
        })
}

fn diff_body(state: AppState, diff: Rc<DiffSet>) -> impl IntoView {
    let mut children: Vec<floem::AnyView> = Vec::new();
    let mut ids: Vec<ViewId> = Vec::new();

    let th = state.theme;
    if let Some(msg) = &diff.message {
        children.push(commit_message_view(th, msg).into_any());
    }

    let wrap = state.word_wrap.get();
    let font_size = state.font_size.get();
    let line_numbers = state.line_numbers.get();

    for f in &diff.files {
        let header = file_header(th, f);
        let body = file_body(th, state.hl.clone(), f, font_size, line_numbers, wrap);
        // Convert the file section into a concrete view so we can grab its `ViewId`,
        // which the file tree uses as a scroll target.
        let section = v_stack((header, body))
            .style(|s| s.flex_col().min_width(0.0).margin_bottom(10.0))
            .into_view();
        ids.push(section.id());
        children.push(section.into_any());
    }

    // Publish the header ViewIds so the file tree can scroll to them.
    state.file_ids.set(Rc::new(ids));

    stack_from_children(children).style(move |s| {
        let s = s.flex_col().padding(10.0).min_width(0.0);
        if wrap {
            s.width_full()
        } else {
            s
        }
    })
}

fn stack_from_children(children: Vec<floem::AnyView>) -> floem::views::Stack {
    floem::views::stack_from_iter(children)
}

fn file_header(th: Theme, f: &FileDiff) -> impl IntoView {
    let icon = label({
        let g = file_icon(&f.path);
        move || g.to_string()
    })
    .style(|s| s.font_size(13.0));

    let name = match (&f.old_path, f.kind) {
        (Some(old), ChangeKind::Renamed) => format!("{old}  →  {}", f.path),
        _ => f.path.clone(),
    };
    let title = label(move || name.clone())
        .style(move |s| s.color(th.text).font_weight(Weight::BOLD).font_size(12.5).min_width(0.0).text_ellipsis().flex_shrink(1.0));
    let letter = f.kind.letter();
    let kind = label(move || format!("[{letter}]")).style(move |s| s.color(th.muted).font_size(11.0));

    let spacer = empty().style(|s| s.flex_grow(1.0));
    let added = f.added;
    let removed = f.removed;
    let plus = label(move || format!("+{added}")).style(move |s| s.color(th.add_fg).font_size(12.0));
    let minus = label(move || format!("−{removed}")).style(move |s| s.color(th.del_fg).font_size(12.0));

    h_stack((icon, title, kind, spacer, plus, minus)).style(move |s| {
        s.items_center()
            .gap(6.0)
            .width_full()
            .min_width(0.0)
            .padding_horiz(10.0)
            .padding_vert(6.0)
            .background(th.panel)
            .border(1.0)
            .border_color(th.border)
    })
}

fn file_body(
    th: Theme,
    hl: Rc<Highlighter>,
    f: &FileDiff,
    font_size: f64,
    line_numbers: bool,
    wrap: bool,
) -> impl IntoView {
    if f.binary {
        return container(
            label(|| "Binary file not shown".to_string())
                .style(move |s| s.color(th.muted).font_style(floem::text::Style::Italic).padding(12.0)),
        )
        .style(|s| s.width_full())
        .into_any();
    }

    let syntax = hl.syntax_for(&f.path);
    let mut rows: Vec<floem::AnyView> = Vec::new();
    for hunk in &f.hunks {
        rows.push(diff_row(th, &hl, syntax, LineKind::Context, None, None, &hunk.header, true, font_size, line_numbers, wrap).into_any());
        for line in &hunk.lines {
            rows.push(
                diff_row(th, &hl, syntax, line.kind, line.old_no, line.new_no, &line.text, false, font_size, line_numbers, wrap)
                    .into_any(),
            );
        }
    }

    stack_from_children(rows)
        .style(move |s| {
            let s = s.flex_col().min_width(0.0).border_horiz(Stroke::new(1.0)).border_color(th.border);
            if wrap { s.width_full() } else { s }
        })
        .into_any()
}

#[allow(clippy::too_many_arguments)]
fn diff_row(
    th: Theme,
    hl: &Highlighter,
    syntax: &syntect::parsing::SyntaxReference,
    kind: LineKind,
    old_no: Option<u32>,
    new_no: Option<u32>,
    text: &str,
    hunk_header: bool,
    font_size: f64,
    line_numbers: bool,
    wrap: bool,
) -> impl IntoView {
    let char_w = font_size * 0.62;
    let num_w = char_w * 4.5;
    let sign_w = char_w * 2.0;

    let (row_bg, mark_bg, sign, sign_fg) = match kind {
        LineKind::Added => (th.add_bg, th.add_mark, '+', th.add_fg),
        LineKind::Removed => (th.del_bg, th.del_mark, '-', th.del_fg),
        LineKind::Context => (Color::TRANSPARENT, Color::TRANSPARENT, ' ', th.muted),
    };
    let row_bg = if hunk_header { th.hunk_bg } else { row_bg };

    // gutter: line numbers + sign marker
    let mut gutter_children: Vec<floem::AnyView> = Vec::new();
    if line_numbers && !hunk_header {
        let on = old_no.map(|n| n.to_string()).unwrap_or_default();
        let nn = new_no.map(|n| n.to_string()).unwrap_or_default();
        gutter_children.push(
            label(move || on.clone())
                .style(move |s| s.width(num_w).color(th.muted).font_family(MONO.to_string()).font_size(font_size as f32).justify_end())
                .into_any(),
        );
        gutter_children.push(
            label(move || nn.clone())
                .style(move |s| s.width(num_w).color(th.muted).font_family(MONO.to_string()).font_size(font_size as f32).justify_end().padding_right(4.0))
                .into_any(),
        );
    }
    let sign_str = sign.to_string();
    gutter_children.push(
        label(move || if hunk_header { String::new() } else { sign_str.clone() })
            .style(move |s| {
                s.width(sign_w)
                    .color(sign_fg)
                    .font_family(MONO.to_string())
                    .font_size(font_size as f32)
                    .justify_center()
                    .background(if hunk_header { Color::TRANSPARENT } else { mark_bg })
            })
            .into_any(),
    );
    let gutter = stack_from_children(gutter_children).style(|s| s.flex_row().flex_shrink(0.0).items_start());

    // code: rich syntax-highlighted text (or plain for hunk header)
    let code_fg = th.code_fg;
    let code = if hunk_header {
        let t = text.to_string();
        let muted = th.muted;
        rich_text(move || plain_layout(&t, font_size as f32, muted, 700))
            .into_any()
    } else {
        let spans = hl.line(syntax, text);
        let display = if text.is_empty() { String::new() } else { text.to_string() };
        rich_text(move || spans_layout(&display, &spans, font_size as f32, code_fg)).into_any()
    };
    let code = code.style(move |s| {
        let s = s.padding_left(6.0).font_family(MONO.to_string()).font_size(font_size as f32);
        if wrap {
            s.flex_grow(1.0).flex_basis(0.0).min_width(0.0)
        } else {
            s.flex_shrink(0.0)
        }
    });

    h_stack((gutter, code)).style(move |s| {
        let s = s
            .items_start()
            .background(row_bg)
            .line_height(1.4);
        if wrap {
            s.width_full().min_width(0.0)
        } else {
            // natural width so the horizontal scrollbar can show long lines
            s.min_width_full()
        }
    })
}

// --- commit message ---------------------------------------------------------

fn commit_message_view(th: Theme, msg: &CommitMessage) -> impl IntoView {
    let title = label({
        let t = msg.title.clone();
        move || t.clone()
    })
    .style(move |s| s.color(th.text).font_size(18.0).font_weight(Weight::BOLD));

    let meta = label({
        let m = format!("{}  ·  {}  ·  {}", msg.author, msg.date, msg.short);
        move || m.clone()
    })
    .style(move |s| s.color(th.muted).font_size(11.0).margin_top(2.0));

    let body = msg.body.clone();
    let has_body = !body.is_empty();
    let body_view = label(move || body.clone()).style(move |s| {
        let s = s.color(th.text).font_family(MONO.to_string()).font_size(12.0).margin_top(8.0);
        if has_body { s } else { s.height(0.0) }
    });

    v_stack((title, meta, body_view)).style(move |s| {
        s.flex_col()
            .width_full()
            .min_width(0.0)
            .padding(12.0)
            .margin_bottom(10.0)
            .background(th.panel)
            .border(1.0)
            .border_color(th.border)
            .border_radius(6.0)
    })
}

// --- helpers ----------------------------------------------------------------

fn section_header(th: Theme, text: String) -> impl IntoView {
    label(move || text.clone()).style(move |s| {
        s.color(th.muted)
            .font_size(11.0)
            .font_weight(Weight::BOLD)
            .padding_horiz(8.0)
            .padding_vert(5.0)
            .width_full()
    })
}

/// Build a `TextLayout` whose runs are coloured from syntect spans.
fn spans_layout(display: &str, spans: &[crate::highlight::Span], font_size: f32, default_fg: (u8, u8, u8)) -> TextLayout {
    let mut layout = TextLayout::new();
    let family = [floem::text::FamilyOwned::Monospace];
    let default = Attrs::new()
        .font_size(font_size)
        .family(&family)
        .color(col(default_fg));
    let mut attrs_list = AttrsList::new(default);

    if spans.is_empty() {
        layout.set_text(if display.is_empty() { " " } else { display }, attrs_list);
        return layout;
    }

    let mut byte = 0usize;
    let mut full = String::new();
    for sp in spans {
        let start = byte;
        full.push_str(&sp.text);
        byte += sp.text.len();
        let mut a = Attrs::new()
            .font_size(font_size)
            .family(&family)
            .color(col(sp.color));
        if sp.bold {
            a = a.weight(Weight::BOLD);
        }
        if sp.italic {
            a = a.style(floem::text::Style::Italic);
        }
        attrs_list.add_span(start..byte, a);
    }
    if full.is_empty() {
        full.push(' ');
    }
    layout.set_text(&full, attrs_list);
    layout
}

fn plain_layout(text: &str, font_size: f32, color: Color, weight: u16) -> TextLayout {
    let mut layout = TextLayout::new();
    let family = [floem::text::FamilyOwned::Monospace];
    let attrs = Attrs::new()
        .font_size(font_size)
        .family(&family)
        .color(color)
        .raw_weight(weight);
    let attrs_list = AttrsList::new(attrs);
    layout.set_text(if text.is_empty() { " " } else { text }, attrs_list);
    layout
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

fn file_icon(path: &str) -> &'static str {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext {
        "rs" => "🦀",
        "py" => "🐍",
        "js" | "ts" | "tsx" | "jsx" => "📜",
        "md" => "📝",
        "toml" | "yaml" | "yml" | "json" | "ini" | "cfg" => "⚙",
        "png" | "jpg" | "jpeg" | "gif" | "svg" => "🖼",
        "sh" | "bash" => "💲",
        "html" | "css" => "🌐",
        _ => "📄",
    }
}
