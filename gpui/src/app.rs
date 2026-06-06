//! The GPUI UI for git-review.
//!
//! GPUI is a retained, GPU-accelerated UI framework (Zed). The UI is described with a
//! tailwind-ish `div()` flex builder, state lives in an `Entity<ReviewApp>`, and large lists are
//! virtualized with `uniform_list`. This mirrors the egui/slint/iced apps feature-for-feature.

use std::ops::Range;
use std::rc::Rc;

use git2::Oid;
use gpui::{
    div, prelude::*, px, rgb, rgba, App, Context, CursorStyle, FontWeight, Hsla,
    MouseButton, MouseMoveEvent, MouseUpEvent, Pixels, ScrollStrategy, SharedString, StyledText,
    TextRun, UniformListScrollHandle, Window,
};

use crate::git::{ChangeKind, CommitInfo, CommitMessage, DiffSet, LineKind, Repo};
use crate::highlight::Highlighter;

const MONO: &str = "monospace";

// --- GitHub light/dark palettes (from SPEC.md) ------------------------------
//
// Both palettes are picked at startup from the OS light/dark preference via the `dark-light`
// crate (see `Theme::detect`). In a headless environment with no desktop preference we default
// to light, per SPEC.md. The optional `GIT_REVIEW_THEME=light|dark` env var overrides detection.
#[derive(Clone, Copy)]
struct Theme {
    /// Whether the dark palette is active (kept for parity with the other apps / future use,
    /// e.g. tracking the syntect highlight theme to the scheme).
    #[allow(dead_code)]
    dark: bool,
    bg: u32,
    panel: u32,
    border: u32,
    text: u32,
    muted: u32,
    accent: u32,
    sel: u32,
    add_bg: u32,
    add_mark: u32,
    del_bg: u32,
    del_mark: u32,
    add_fg: u32,
    del_fg: u32,
    hunk_bg: u32,
}

impl Theme {
    const LIGHT: Theme = Theme {
        dark: false,
        bg: 0xffffff,
        panel: 0xf6f8fa,
        border: 0xd0d7de,
        text: 0x1f2328,
        muted: 0x656d76,
        accent: 0x0969da,
        sel: 0xddf4ff,
        add_bg: 0xe6ffec,
        add_mark: 0xabf2bc,
        del_bg: 0xffebe9,
        del_mark: 0xff8182,
        add_fg: 0x1a7f37,
        del_fg: 0xcf222e,
        hunk_bg: 0xddf4ff,
    };

    const DARK: Theme = Theme {
        dark: true,
        bg: 0x0d1117,
        panel: 0x161b22,
        border: 0x30363d,
        text: 0xe6edf3,
        muted: 0x8b949e,
        accent: 0x2f81f7,
        sel: 0x1f6feb,
        add_bg: 0x12261e,
        add_mark: 0x2ea043,
        del_bg: 0x25171c,
        del_mark: 0xf85149,
        add_fg: 0x3fb950,
        del_fg: 0xf85149,
        hunk_bg: 0x132e4d,
    };

    /// Detect the desktop light/dark preference at startup. `GIT_REVIEW_THEME` wins if set;
    /// otherwise `dark-light` is consulted, defaulting to light when unspecified (headless).
    fn detect() -> Theme {
        match std::env::var("GIT_REVIEW_THEME").ok().as_deref() {
            Some("dark") => return Theme::DARK,
            Some("light") => return Theme::LIGHT,
            _ => {}
        }
        match dark_light::detect() {
            Ok(dark_light::Mode::Dark) => Theme::DARK,
            _ => Theme::LIGHT,
        }
    }
}

// ----------------------------------------------------------------------------

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

/// One pre-flattened row of the scrollable diff body. Flattening lets us virtualize the whole
/// diff (commit message + all files + all hunks/lines) through a single `uniform_list`.
#[derive(Clone)]
enum DiffRow {
    Message(Rc<CommitMessage>),
    FileHeader(usize),
    HunkHeader(SharedString),
    Line {
        file: usize,
        kind: LineKind,
        old_no: Option<u32>,
        new_no: Option<u32>,
        text: SharedString,
    },
    Binary,
    Gap,
}

/// A node in the hierarchical file tree built from the current diff's file paths.
struct TreeNode {
    /// The single path component shown on this row (folders collapse single-child chains, so
    /// this may be a `a/b/c` segment).
    name: String,
    /// `Some(file_index)` for a leaf (index into `diff.files`); `None` for a folder.
    file: Option<usize>,
    /// Stable identity key (the full path so far) used for the expand-state set.
    key: String,
    children: Vec<TreeNode>,
}

/// One flattened, indented row of the file tree (built from the `TreeNode` forest honoring the
/// per-folder expand state). This is what `uniform_list` renders.
#[derive(Clone)]
struct TreeRow {
    depth: usize,
    name: SharedString,
    /// `Some(index)` for a file leaf, `None` for a folder.
    file: Option<usize>,
    /// Folder identity key (used to toggle expand state); empty for leaves.
    key: SharedString,
    expanded: bool,
}

pub struct ReviewApp {
    repo: Repo,
    hl: Rc<Highlighter>,
    theme: Theme,
    repo_name: String,
    commits: Vec<CommitInfo>,
    diff: DiffSet,
    showing: Showing,
    from: Option<Oid>,
    to: Option<Oid>,
    settings: Settings,
    error: Option<String>,

    // Flattened diff body + index of each file's header row (for click-to-scroll).
    rows: Vec<DiffRow>,
    file_header_row: Vec<usize>,

    // Hierarchical file tree: flattened (visible) rows + the set of collapsed folder keys.
    // Folders are expanded by default, so we track only the ones the user has collapsed.
    tree_rows: Vec<TreeRow>,
    collapsed: std::collections::HashSet<String>,

    // Layout state.
    side_width: Pixels,
    commits_height: Pixels,
    drag: Option<Drag>,

    diff_scroll: UniformListScrollHandle,
    commits_scroll: UniformListScrollHandle,
    files_scroll: UniformListScrollHandle,
}

#[derive(Clone, Copy)]
enum Drag {
    SidePanel,
    CommitsSplit,
}

impl ReviewApp {
    pub fn new(repo: Repo, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        let theme = Theme::detect();
        let hl = Rc::new(Highlighter::new());
        let repo_name = repo.workdir_name();
        let commits = repo.commits(500).unwrap_or_default();
        let showing = commits
            .iter()
            .find_map(|c| c.oid.map(Showing::Commit))
            .unwrap_or(Showing::Working);
        let mut app = Self {
            repo,
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
            rows: Vec::new(),
            file_header_row: Vec::new(),
            tree_rows: Vec::new(),
            collapsed: std::collections::HashSet::new(),
            side_width: px(320.0),
            commits_height: px(360.0),
            drag: None,
            diff_scroll: UniformListScrollHandle::new(),
            commits_scroll: UniformListScrollHandle::new(),
            files_scroll: UniformListScrollHandle::new(),
        };
        app.recompute();
        app
    }

    fn recompute(&mut self) {
        let ignore_ws = !self.settings.show_space;
        let res = match self.showing {
            Showing::Working => self.repo.working_tree(ignore_ws),
            Showing::Commit(oid) => self.repo.show(oid, ignore_ws),
            Showing::Range(a, b) => self.repo.range(a, b, ignore_ws),
        };
        match res {
            Ok(d) => {
                self.diff = d;
                self.error = None;
            }
            Err(e) => self.error = Some(e.message().to_string()),
        }
        self.flatten();
    }

    /// Flatten the current `DiffSet` into a vector of uniform-height rows.
    fn flatten(&mut self) {
        let mut rows = Vec::new();
        let mut header_rows = Vec::new();

        if let Some(msg) = &self.diff.message {
            rows.push(DiffRow::Message(Rc::new(msg.clone())));
            rows.push(DiffRow::Gap);
        }

        for (i, f) in self.diff.files.iter().enumerate() {
            header_rows.push(rows.len());
            rows.push(DiffRow::FileHeader(i));
            if f.binary {
                rows.push(DiffRow::Binary);
            } else {
                for hunk in &f.hunks {
                    rows.push(DiffRow::HunkHeader(hunk.header.clone().into()));
                    for line in &hunk.lines {
                        rows.push(DiffRow::Line {
                            file: i,
                            kind: line.kind,
                            old_no: line.old_no,
                            new_no: line.new_no,
                            text: line.text.clone().into(),
                        });
                    }
                }
            }
            rows.push(DiffRow::Gap);
        }

        self.rows = rows;
        self.file_header_row = header_rows;
        self.rebuild_tree();
    }

    /// Build the hierarchical file tree from the current diff's paths and flatten it (honoring
    /// the collapsed-folder set) into `tree_rows` for the `uniform_list`.
    fn rebuild_tree(&mut self) {
        // Build a forest of folder/leaf nodes from the file paths.
        let mut roots: Vec<TreeNode> = Vec::new();
        for (i, f) in self.diff.files.iter().enumerate() {
            insert_path(&mut roots, &f.path, i);
        }
        // GitHub-style: collapse single-child directory chains into one node.
        for r in &mut roots {
            collapse_chain(r);
        }
        sort_forest(&mut roots);

        let mut out = Vec::new();
        for r in &roots {
            self.flatten_tree(r, 0, &mut out);
        }
        self.tree_rows = out;
    }

    fn flatten_tree(&self, node: &TreeNode, depth: usize, out: &mut Vec<TreeRow>) {
        match node.file {
            Some(idx) => out.push(TreeRow {
                depth,
                name: node.name.clone().into(),
                file: Some(idx),
                key: SharedString::default(),
                expanded: false,
            }),
            None => {
                let expanded = !self.collapsed.contains(&node.key);
                out.push(TreeRow {
                    depth,
                    name: node.name.clone().into(),
                    file: None,
                    key: node.key.clone().into(),
                    expanded,
                });
                if expanded {
                    for c in &node.children {
                        self.flatten_tree(c, depth + 1, out);
                    }
                }
            }
        }
    }

    fn toggle_folder(&mut self, key: &str) {
        if !self.collapsed.remove(key) {
            self.collapsed.insert(key.to_string());
        }
        self.rebuild_tree();
    }

    fn select(&mut self, showing: Showing, cx: &mut Context<Self>) {
        if self.showing != showing {
            self.showing = showing;
            self.recompute();
            self.diff_scroll.scroll_to_item(0, ScrollStrategy::Top);
            cx.notify();
        }
    }

    fn maybe_range(&mut self, cx: &mut Context<Self>) {
        if let (Some(a), Some(b)) = (self.from, self.to) {
            self.select(Showing::Range(a, b), cx);
        }
    }

    fn scroll_to_file(&mut self, idx: usize) {
        if let Some(&row) = self.file_header_row.get(idx) {
            self.diff_scroll.scroll_to_item(row, ScrollStrategy::Top);
        }
    }

    // === toolbar ============================================================
    fn render_toolbar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let s = &self.settings;
        div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(40.0))
            .px_2()
            .gap_2()
            .bg(rgb(t.bg))
            .border_b_1()
            .border_color(rgb(t.border))
            .child(
                div()
                    .font_family(MONO)
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(t.text))
                    .child(SharedString::from(self.diff.summary.clone())),
            )
            .child(
                div()
                    .text_color(rgb(t.add_fg))
                    .child(format!("+{}", self.diff.added)),
            )
            .child(
                div()
                    .text_color(rgb(t.del_fg))
                    .child(format!("\u{2212}{}", self.diff.removed)),
            )
            .child(div().flex_1())
            .child(self.tool_button("wrap", "\u{21b5}", "Word wrap", s.word_wrap, cx))
            .child(self.tool_button(
                "space",
                "\u{2423}",
                "Show space changes",
                s.show_space,
                cx,
            ))
            .child(self.tool_button("dec", "A-", "Decrease font size", false, cx))
            .child(self.tool_button("inc", "A+", "Increase font size", false, cx))
            .child(self.tool_button("num", "#", "Show line numbers", s.line_numbers, cx))
    }

    fn tool_button(
        &self,
        id: &'static str,
        label: &'static str,
        tip: &'static str,
        active: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = self.theme;
        let tip: SharedString = tip.into();
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .min_w(px(28.0))
            .h(px(26.0))
            .px_1()
            .rounded_md()
            .border_1()
            .border_color(rgb(t.border))
            .font_family(MONO)
            .when(active, |d| {
                d.bg(rgb(t.sel))
                    .text_color(rgb(t.accent))
                    .font_weight(FontWeight::BOLD)
            })
            .when(!active, |d| d.bg(rgb(t.bg)).text_color(rgb(t.text)))
            .hover(|d| d.bg(rgb(t.panel)))
            .cursor_pointer()
            .tooltip(move |window, cx| Tip::view(tip.clone(), window, cx))
            .on_click(cx.listener(move |this, _ev, _window, cx| {
                this.on_tool(id, cx);
            }))
            .child(label)
    }

    fn on_tool(&mut self, id: &str, cx: &mut Context<Self>) {
        let mut recompute = false;
        match id {
            "wrap" => self.settings.word_wrap = !self.settings.word_wrap,
            "space" => {
                self.settings.show_space = !self.settings.show_space;
                recompute = true;
            }
            "dec" => self.settings.font_size = (self.settings.font_size - 1.0).max(8.0),
            "inc" => self.settings.font_size = (self.settings.font_size + 1.0).min(28.0),
            "num" => self.settings.line_numbers = !self.settings.line_numbers,
            _ => {}
        }
        if recompute {
            self.recompute();
        }
        cx.notify();
    }

    // === commit list ========================================================
    fn render_commit_list(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let header = div()
            .px_2()
            .py_1()
            .text_xs()
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(t.muted))
            .child(format!("COMMITS \u{b7} {}", self.repo_name));

        let count = self.commits.len();
        let list = gpui::uniform_list(
            "commits",
            count,
            cx.processor(|this, range: Range<usize>, _window, cx| {
                let mut items = Vec::new();
                for ix in range {
                    items.push(this.commit_row(ix, cx));
                }
                items
            }),
        )
        .track_scroll(self.commits_scroll.clone())
        .flex_grow();

        div()
            .flex()
            .flex_col()
            .h(self.commits_height)
            .min_h(px(80.0))
            .overflow_hidden()
            .bg(rgb(t.panel))
            .child(header)
            .child(list)
    }

    fn commit_row(&self, ix: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let c = self.commits[ix].clone();
        let is_current = match (self.showing, c.oid) {
            (Showing::Working, _) if c.is_working_tree() => true,
            (Showing::Commit(o), Some(oid)) => o == oid,
            _ => false,
        };
        let is_from = c.oid.is_some() && c.oid == self.from;
        let is_to = c.oid.is_some() && c.oid == self.to;
        let oid = c.oid;

        let mut line1 = div().flex().flex_row().items_center().gap_1();

        if let Some(oid) = oid {
            line1 = line1
                .child(self.endpoint(ix, "from", "\u{25c0}", "Compare from this commit", is_from, cx))
                .child(self.endpoint(ix, "to", "\u{25b6}", "Compare to this commit", is_to, cx));
            let _ = oid;
        } else {
            line1 = line1.child(div().w(px(44.0)));
        }

        line1 = line1
            .child(
                div()
                    .font_family(MONO)
                    .text_color(rgb(t.accent))
                    .text_size(px(11.0))
                    .child(c.short.clone()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(t.muted))
                    .child(c.date.clone()),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(t.muted))
                    .max_w(px(110.0))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .truncate()
                    .child(c.author.clone()),
            );

        let title = div()
            .text_color(rgb(t.text))
            .text_size(px(12.5))
            .whitespace_nowrap()
            .overflow_hidden()
            .truncate()
            .child(c.title.clone());

        div()
            .id(("commit", ix))
            .flex()
            .flex_col()
            .gap_0p5()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(rgb(t.border))
            .when(is_current, |d| d.bg(rgb(t.sel)))
            .when(!is_current, |d| d.hover(|h| h.bg(rgba(0x0969da14))))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _ev, _window, cx| match oid {
                Some(oid) => this.select(Showing::Commit(oid), cx),
                None => this.select(Showing::Working, cx),
            }))
            .child(line1)
            .child(title)
    }

    fn endpoint(
        &self,
        ix: usize,
        which: &'static str,
        glyph: &'static str,
        tip: &'static str,
        active: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = self.theme;
        let tip: SharedString = tip.into();
        let oid = self.commits[ix].oid;
        div()
            .id(("ep", ix * 2 + (which == "to") as usize))
            .flex()
            .items_center()
            .justify_center()
            .w(px(20.0))
            .h(px(18.0))
            .rounded_sm()
            .text_size(px(10.0))
            .when(active, |d| d.bg(rgb(t.accent)).text_color(rgb(0xffffff)))
            .when(!active, |d| d.bg(rgb(0xe6e6e6)).text_color(rgb(t.text)))
            .hover(|d| d.opacity(0.8))
            .cursor_pointer()
            .tooltip(move |window, cx| Tip::view(tip.clone(), window, cx))
            .on_click(cx.listener(move |this, _ev, _window, cx| {
                if let Some(oid) = oid {
                    if which == "from" {
                        this.from = Some(oid);
                    } else {
                        this.to = Some(oid);
                    }
                    this.maybe_range(cx);
                    cx.notify();
                }
            }))
            .child(glyph)
    }

    // === file tree ==========================================================
    fn render_file_tree(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let header = div()
            .px_2()
            .py_1()
            .text_xs()
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(t.muted))
            .child(format!("FILES ({})", self.diff.files.len()));

        let count = self.tree_rows.len();
        let list = gpui::uniform_list(
            "files",
            count,
            cx.processor(|this, range: Range<usize>, _window, cx| {
                let mut items = Vec::new();
                for ix in range {
                    items.push(this.tree_row_el(ix, cx));
                }
                items
            }),
        )
        .track_scroll(self.files_scroll.clone())
        .flex_grow();

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(60.0))
            .overflow_hidden()
            .bg(rgb(t.panel))
            .border_t_1()
            .border_color(rgb(t.border))
            .child(header)
            .child(list)
    }

    /// One row of the hierarchical file tree: either a folder (disclosure arrow + folder icon,
    /// click toggles) or a file leaf (file icon + name + `+a −r`, click scrolls the diff to it).
    fn tree_row_el(&self, ix: usize, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let t = self.theme;
        let row = self.tree_rows[ix].clone();
        // Indent by depth; folders nudge left so their arrow lines up with their children.
        let indent = px(8.0 + row.depth as f32 * 14.0);

        let el = div()
            .id(("tree", ix))
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .pr_2()
            .pl(indent)
            .py_0p5()
            .cursor_pointer()
            .hover(|h| h.bg(rgba(0x0969da14)));

        match row.file {
            None => {
                // Folder row.
                let key = row.key.to_string();
                let arrow = if row.expanded { "\u{25be}" } else { "\u{25b8}" };
                el.child(
                    div()
                        .w(px(12.0))
                        .text_size(px(10.0))
                        .text_color(rgb(t.muted))
                        .child(arrow),
                )
                .child(div().child(SharedString::from("\u{1f4c1}")))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(t.text))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .truncate()
                        .child(row.name.clone()),
                )
                .on_click(cx.listener(move |this, _ev, _window, cx| {
                    this.toggle_folder(&key);
                    cx.notify();
                }))
            }
            Some(fidx) => {
                let f = &self.diff.files[fidx];
                // Leaf row: a spacer to align under the disclosure arrow, then icon/name/counts.
                el.child(div().w(px(12.0)))
                    .child(div().child(file_icon(&f.path)))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.0))
                            .text_color(rgb(t.text))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .truncate()
                            .child(row.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(t.add_fg))
                            .child(format!("+{}", f.added)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(t.del_fg))
                            .child(format!("\u{2212}{}", f.removed)),
                    )
                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                        this.scroll_to_file(fidx);
                        cx.notify();
                    }))
            }
        }
    }

    // === diff view ==========================================================
    fn render_diff(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        if let Some(err) = self.error.clone() {
            return div()
                .size_full()
                .p_4()
                .bg(rgb(t.bg))
                .text_color(rgb(t.del_fg))
                .child(format!("Error: {err}"));
        }

        let count = self.rows.len();
        let list = gpui::uniform_list(
            "diff",
            count,
            cx.processor(|this, range: Range<usize>, _window, _cx| {
                let mut items = Vec::new();
                for ix in range {
                    items.push(this.diff_row(ix));
                }
                items
            }),
        )
        .track_scroll(self.diff_scroll.clone())
        .size_full();

        div().size_full().bg(rgb(t.bg)).child(list)
    }

    fn diff_row(&self, ix: usize) -> gpui::Div {
        let t = self.theme;
        let fs = self.settings.font_size;
        let row = self.rows[ix].clone();
        match row {
            DiffRow::Gap => div().h(px(8.0)).bg(rgb(t.bg)),
            DiffRow::Message(msg) => self.commit_message_row(&msg),
            DiffRow::Binary => div()
                .px_4()
                .py_1()
                .bg(rgb(t.bg))
                .text_color(rgb(t.muted))
                .italic()
                .child("Binary file not shown"),
            DiffRow::FileHeader(i) => self.file_header_row_el(i),
            DiffRow::HunkHeader(header) => div()
                .w_full()
                .px_2()
                .bg(rgb(t.hunk_bg))
                .font_family(MONO)
                .text_size(px(fs))
                .text_color(rgb(t.muted))
                .whitespace_nowrap()
                .child(header),
            DiffRow::Line {
                file,
                kind,
                old_no,
                new_no,
                text,
            } => self.code_line(file, kind, old_no, new_no, text),
        }
    }

    fn commit_message_row(&self, msg: &CommitMessage) -> gpui::Div {
        let t = self.theme;
        let mut inner = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_size(px(18.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(t.text))
                    .child(msg.title.clone()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(t.muted))
                    .child(format!("{}  \u{b7}  {}  \u{b7}  {}", msg.author, msg.date, msg.short)),
            );
        if !msg.body.is_empty() {
            inner = inner.child(
                div()
                    .mt_2()
                    .font_family(MONO)
                    .text_color(rgb(t.text))
                    .child(msg.body.clone()),
            );
        }
        div().px_2().pt_2().bg(rgb(t.bg)).child(
            div()
                .p_3()
                .bg(rgb(t.panel))
                .border_1()
                .border_color(rgb(t.border))
                .rounded_md()
                .child(inner),
        )
    }

    fn file_header_row_el(&self, i: usize) -> gpui::Div {
        let t = self.theme;
        let f = &self.diff.files[i];
        let label = match (&f.old_path, f.kind) {
            (Some(old), ChangeKind::Renamed) => format!("{old}  \u{2192}  {}", f.path),
            _ => f.path.clone(),
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .w_full()
            .px_2()
            .py_1()
            .bg(rgb(t.panel))
            .border_1()
            .border_color(rgb(t.border))
            .child(div().child(file_icon(&f.path)))
            .child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(12.5))
                    .text_color(rgb(t.text))
                    .child(label),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(t.muted))
                    .child(format!("[{}]", f.kind.letter())),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_color(rgb(t.add_fg))
                    .child(format!("+{}", f.added)),
            )
            .child(
                div()
                    .text_color(rgb(t.del_fg))
                    .child(format!("\u{2212}{}", f.removed)),
            )
    }

    fn code_line(
        &self,
        file: usize,
        kind: LineKind,
        old_no: Option<u32>,
        new_no: Option<u32>,
        text: SharedString,
    ) -> gpui::Div {
        let t = self.theme;
        let fs = self.settings.font_size;
        let (bg, mark, sign, sign_fg) = match kind {
            LineKind::Added => (Some(rgb(t.add_bg)), Some(rgb(t.add_mark)), "+", rgb(t.add_fg)),
            LineKind::Removed => (Some(rgb(t.del_bg)), Some(rgb(t.del_mark)), "-", rgb(t.del_fg)),
            LineKind::Context => (None, None, " ", rgb(t.muted)),
        };

        let num_w = px(fs * 0.62 * 4.0 + 6.0);

        let mut row = div()
            .flex()
            .flex_row()
            .items_start()
            .w_full()
            .font_family(MONO)
            .text_size(px(fs));
        if let Some(bg) = bg {
            row = row.bg(bg);
        }

        // gutter: optional old/new line numbers + the +/-/space marker column.
        if self.settings.line_numbers {
            row = row
                .child(
                    div()
                        .w(num_w)
                        .px_1()
                        .text_color(rgb(t.muted))
                        .text_right()
                        .child(opt_num(old_no)),
                )
                .child(
                    div()
                        .w(num_w)
                        .px_1()
                        .text_color(rgb(t.muted))
                        .text_right()
                        .child(opt_num(new_no)),
                );
        }
        let mut marker = div()
            .w(px(fs * 0.62 * 2.0))
            .flex_shrink_0()
            .text_center()
            .text_color(sign_fg)
            .child(sign);
        if let Some(mark) = mark {
            marker = marker.bg(mark);
        }
        row = row.child(marker);

        // code: syntax-highlighted spans rendered as a single StyledText with runs.
        let code = self.styled_code(file, &text, fs);
        let code_cell = if self.settings.word_wrap {
            div().flex_1().px_1().child(code)
        } else {
            div().px_1().whitespace_nowrap().child(code)
        };
        row.child(code_cell)
    }

    /// Build a `StyledText` for a code line using syntect highlight spans -> `TextRun`s.
    fn styled_code(&self, file: usize, text: &SharedString, fs: f32) -> StyledText {
        if text.is_empty() {
            return StyledText::new(SharedString::from(" "));
        }
        let path = &self.diff.files[file].path;
        let syntax = self.hl.syntax_for(path);
        let spans = self.hl.line(syntax, text);
        if spans.is_empty() {
            return StyledText::new(text.clone());
        }
        let mut runs: Vec<TextRun> = Vec::with_capacity(spans.len());
        let mut acc = String::new();
        for s in &spans {
            acc.push_str(&s.text);
            runs.push(TextRun {
                len: s.text.len(),
                font: gpui::font(MONO).into_styled(s.bold, s.italic),
                color: rgb_tuple(s.color),
                background_color: None,
                underline: None,
                strikethrough: None,
            });
        }
        // Guard: the run lengths must sum to exactly the string length.
        if acc.as_str() != text.as_ref() {
            return StyledText::new(text.clone());
        }
        let _ = fs;
        StyledText::new(SharedString::from(acc)).with_runs(runs)
    }

    // === resizable splitters ================================================
    fn vertical_splitter(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        // Between the side panel and the main view: drag to resize horizontally.
        div()
            .id("side-split")
            .w(px(5.0))
            .flex_shrink_0()
            .h_full()
            .bg(rgb(t.border))
            .hover(|h| h.bg(rgb(t.accent)))
            .cursor(CursorStyle::ResizeLeftRight)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _ev, _window, cx| {
                    this.drag = Some(Drag::SidePanel);
                    cx.notify();
                }),
            )
    }

    fn horizontal_splitter(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        // Between the commit list and the file tree: drag to resize vertically.
        div()
            .id("commits-split")
            .h(px(5.0))
            .flex_shrink_0()
            .w_full()
            .bg(rgb(t.border))
            .hover(|h| h.bg(rgb(t.accent)))
            .cursor(CursorStyle::ResizeUpDown)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _ev, _window, cx| {
                    this.drag = Some(Drag::CommitsSplit);
                    cx.notify();
                }),
            )
    }
}

impl Render for ReviewApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let toolbar = self.render_toolbar(cx);
        let commit_list = self.render_commit_list(cx);
        let splitter_h = self.horizontal_splitter(cx);
        let file_tree = self.render_file_tree(cx);
        let splitter_v = self.vertical_splitter(cx);
        let diff = self.render_diff(cx);

        let side = div()
            .flex()
            .flex_col()
            .w(self.side_width)
            .min_w(px(220.0))
            .max_w(px(640.0))
            .h_full()
            .bg(rgb(t.panel))
            .child(commit_list)
            .child(splitter_h)
            .child(file_tree);

        let main = div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .overflow_hidden()
            .child(div().flex_1().overflow_hidden().child(diff));

        // Below the full-width toolbar: side panel | splitter | main view.
        let body = div()
            .flex()
            .flex_row()
            .flex_1()
            .w_full()
            .overflow_hidden()
            .child(side)
            .child(splitter_v)
            .child(main);

        // Root: a vertical stack — the full-width toolbar pinned on top, then the body.
        let mut root = div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(t.bg))
            .text_color(rgb(t.text))
            .text_size(px(13.0))
            .child(toolbar)
            .child(body);

        // While a splitter is held, listen on the window for moves/release to resize.
        if self.drag.is_some() {
            root = root
                .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _window, cx| {
                    match this.drag {
                        Some(Drag::SidePanel) => {
                            let w = ev.position.x.clamp(px(220.0), px(640.0));
                            this.side_width = w;
                            cx.notify();
                        }
                        Some(Drag::CommitsSplit) => {
                            // Position is relative to the window; the body starts below the
                            // 40px-tall toolbar, so subtract it to get the commit-list height.
                            let h = (ev.position.y - px(40.0)).clamp(px(80.0), px(700.0));
                            this.commits_height = h;
                            cx.notify();
                        }
                        None => {}
                    }
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _ev: &MouseUpEvent, _window, cx| {
                        this.drag = None;
                        cx.notify();
                    }),
                );
        }

        root
    }
}

// --- file-tree construction -------------------------------------------------

/// Insert a file path into the folder/leaf forest, creating intermediate folder nodes.
fn insert_path(forest: &mut Vec<TreeNode>, path: &str, file_idx: usize) {
    let comps: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
    if comps.is_empty() {
        return;
    }
    let mut level = forest;
    let mut prefix = String::new();
    for (i, comp) in comps.iter().enumerate() {
        let is_leaf = i + 1 == comps.len();
        if prefix.is_empty() {
            prefix = comp.to_string();
        } else {
            prefix.push('/');
            prefix.push_str(comp);
        }
        if is_leaf {
            level.push(TreeNode {
                name: comp.to_string(),
                file: Some(file_idx),
                key: prefix.clone(),
                children: Vec::new(),
            });
            return;
        }
        // Find or create the folder node for this component.
        let pos = level
            .iter()
            .position(|n| n.file.is_none() && n.name == *comp);
        let idx = match pos {
            Some(p) => p,
            None => {
                level.push(TreeNode {
                    name: comp.to_string(),
                    file: None,
                    key: prefix.clone(),
                    children: Vec::new(),
                });
                level.len() - 1
            }
        };
        level = &mut level[idx].children;
    }
}

/// GitHub-style: fold a folder that has exactly one child folder into a single `a/b` node.
/// A folder with a single *file* child is left alone (so the file stays addressable).
fn collapse_chain(node: &mut TreeNode) {
    if node.file.is_none() {
        while node.children.len() == 1 && node.children[0].file.is_none() {
            let child = node.children.remove(0);
            node.name = format!("{}/{}", node.name, child.name);
            node.key = child.key;
            node.children = child.children;
        }
        for c in &mut node.children {
            collapse_chain(c);
        }
    }
}

/// Sort each level: folders first, then files, each alphabetically.
fn sort_forest(forest: &mut [TreeNode]) {
    forest.sort_by(|a, b| match (a.file.is_none(), b.file.is_none()) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    for n in forest.iter_mut() {
        sort_forest(&mut n.children);
    }
}

// --- small helpers ----------------------------------------------------------

/// A minimal tooltip view (gpui 0.2.2 has no built-in tooltip widget; Zed's lives in its `ui`
/// crate). We render a small labelled box as an `AnyView`.
struct Tip {
    text: SharedString,
}

impl Tip {
    fn view(text: SharedString, _window: &mut Window, cx: &mut App) -> gpui::AnyView {
        cx.new(|_| Tip { text }).into()
    }
}

impl Render for Tip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .bg(rgb(0x1f2328))
            .text_color(rgb(0xffffff))
            .text_xs()
            .rounded_md()
            .shadow_md()
            .child(self.text.clone())
    }
}

trait StyledFont {
    fn into_styled(self, bold: bool, italic: bool) -> Self;
}

impl StyledFont for gpui::Font {
    fn into_styled(mut self, bold: bool, italic: bool) -> Self {
        if bold {
            self.weight = FontWeight::BOLD;
        }
        if italic {
            self.style = gpui::FontStyle::Italic;
        }
        self
    }
}

fn rgb_tuple(c: (u8, u8, u8)) -> Hsla {
    rgb(((c.0 as u32) << 16) | ((c.1 as u32) << 8) | c.2 as u32).into()
}

fn opt_num(n: Option<u32>) -> SharedString {
    match n {
        Some(n) => n.to_string().into(),
        None => SharedString::default(),
    }
}

fn file_icon(path: &str) -> SharedString {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let glyph = match ext {
        "rs" => "\u{1f980}",
        "py" => "\u{1f40d}",
        "js" | "ts" | "tsx" | "jsx" => "\u{1f4dc}",
        "md" => "\u{1f4dd}",
        "toml" | "yaml" | "yml" | "json" | "ini" | "cfg" => "\u{2699}",
        "png" | "jpg" | "jpeg" | "gif" | "svg" => "\u{1f5bc}",
        "sh" | "bash" => "\u{1f4b2}",
        "html" | "css" => "\u{1f310}",
        _ => "\u{1f4c4}",
    };
    glyph.into()
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
