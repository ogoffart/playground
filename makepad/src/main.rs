//! Makepad implementation of the `git-review` GitHub-style diff viewer.
//!
//! Architecture notes (how Makepad's idioms map to the SPEC):
//!
//! * UI is declared in a single `live_design!{}` DSL block. The chrome (window, toolbar,
//!   side panel, splitters) is composed from stock `makepad-widgets` (`View`, `Button`,
//!   `Label`, `Splitter`, `PortalList`).
//! * The three scrollable lists (commits, file tree, diff body) are virtualized
//!   `PortalList`s. We flatten the diff into a `Vec<Row>` so the diff body — commit
//!   message, per-file headers, hunk headers and code lines — is a single uniform list
//!   that PortalList can recycle widgets across.
//! * Per-line backgrounds and per-span syntax colors are not expressible with a stock
//!   `Label` (single color). So `DiffRow` is a small custom `Widget` that draws a
//!   background quad (`DrawColor`) plus the gutter and each highlight span with its own
//!   color via a `DrawText` instance. This is the GitHub red/green line + colored token
//!   look from the SPEC.
//! * Resizable panels use the real `Splitter` widget (it owns its own drag handling),
//!   giving the horizontally-resizable side panel and the vertically-split
//!   commits/files panes from the SPEC.

use makepad_widgets::*;

mod git;
mod highlight;

use git::{ChangeKind, CommitInfo, DiffSet, LineKind, Repo};
use git2::Oid;
use highlight::Highlighter;

live_design! {
    use link::theme::*;
    use link::shaders::*;
    use link::widgets::*;

    // ---- palette (GitHub light, from SPEC.md) ------------------------------
    BG = #ffffff
    PANEL = #f6f8fa
    BORDER = #d0d7de
    TEXTCOL = #1f2328
    MUTED = #656d76
    ACCENT = #0969da
    SEL = #ddf4ff
    ADD_FG_DSL = #1a7f37
    DEL_FG_DSL = #cf222e

    // A flat tool/toggle button used in the toolbar and commit endpoints.
    ToolBtn = <Button> {
        width: Fit, height: 26,
        padding: { left: 8, right: 8, top: 4, bottom: 4 }
        margin: { left: 2 }
        draw_bg: {
            instance active: 0.0
            fn pixel(self) -> vec4 {
                let sdf = Sdf2d::viewport(self.pos * self.rect_size);
                sdf.box(0.5, 0.5, self.rect_size.x - 1.0, self.rect_size.y - 1.0, 4.0);
                let base = mix((BG), (SEL), self.active);
                let bg = mix(base, (SEL), self.hover * 0.6);
                sdf.fill_keep(bg);
                sdf.stroke(mix((BORDER), (ACCENT), self.active), 1.0);
                return sdf.result;
            }
        }
        draw_text: {
            instance active: 0.0
            text_style: <THEME_FONT_REGULAR> { font_size: 11.0 }
            fn get_color(self) -> vec4 {
                return mix((TEXTCOL), (ACCENT), self.active);
            }
        }
    }

    // ---- custom diff row widget -------------------------------------------
    DiffRow = {{DiffRow}} {
        width: Fill, height: Fit
    }

    // ---- commit list row --------------------------------------------------
    CommitRow = <View> {
        width: Fill, height: Fit,
        flow: Down,
        padding: { left: 8, right: 8, top: 5, bottom: 5 }
        show_bg: true,
        draw_bg: { color: (PANEL) }
        cursor: Hand,

        line1 = <View> {
            width: Fill, height: Fit, flow: Right, align: { y: 0.5 }, spacing: 4
            from_btn = <ToolBtn> { text: "◀" }
            to_btn = <ToolBtn> { text: "▶" }
            sha = <Label> { draw_text: { color: (ACCENT), text_style: { font_size: 11.0 } } text: "" }
            date = <Label> { draw_text: { color: (MUTED), text_style: { font_size: 10.0 } } text: "" }
            filler = <View> { width: Fill, height: 1 }
            author = <Label> { width: Fit, draw_text: { color: (MUTED), text_style: { font_size: 10.0 } } text: "" }
        }
        title = <Label> {
            width: Fill, height: Fit, margin: { top: 2 }
            draw_text: { color: (TEXTCOL), wrap: Ellipsis, text_style: { font_size: 12.0 } }
            text: ""
        }
        sep = <View> { width: Fill, height: 1, show_bg: true, draw_bg: { color: (BORDER) } margin: { top: 4 } }
    }

    // ---- file tree row ----------------------------------------------------
    FileRow = <View> {
        width: Fill, height: Fit, flow: Right, align: { y: 0.5 }, spacing: 4
        padding: { left: 10, right: 8, top: 3, bottom: 3 }
        show_bg: true,
        draw_bg: { color: (PANEL) }
        cursor: Hand,
        icon = <Label> { width: Fit, draw_text: { text_style: { font_size: 12.0 } } text: "" }
        name = <Label> { width: Fit, draw_text: { color: (TEXTCOL), wrap: Ellipsis, text_style: { font_size: 12.0 } } text: "" }
        ffiller = <View> { width: Fill, height: 1 }
        fadd = <Label> { width: Fit, draw_text: { color: (ADD_FG_DSL), text_style: { font_size: 10.0 } } text: "" }
        fdel = <Label> { width: Fit, draw_text: { color: (DEL_FG_DSL), text_style: { font_size: 10.0 } } text: "" }
    }

    SectionHeader = <View> {
        width: Fill, height: Fit, flow: Right, align: { y: 0.5 }
        padding: { left: 8, right: 8, top: 6, bottom: 6 }
        show_bg: true, draw_bg: { color: (PANEL) }
        lbl = <Label> { draw_text: { color: (MUTED), text_style: <THEME_FONT_BOLD> { font_size: 10.0 } } text: "" }
    }

    App = {{App}} {
        ui: <Root> {
            main_window = <Window> {
                window: { inner_size: vec2(1280, 860), title: "git-review (Makepad)" }
                body = <View> {
                    width: Fill, height: Fill, flow: Right,
                    show_bg: true, draw_bg: { color: (BG) }

                    // horizontally-resizable side panel | main view
                    outer_split = <Splitter> {
                        axis: Horizontal,
                        align: FromA(320.0),
                        a = <View> {
                            width: Fill, height: Fill, flow: Down,
                            show_bg: true, draw_bg: { color: (PANEL) }
                            // vertical split: commits (top) / files (bottom)
                            side_split = <Splitter> {
                                axis: Vertical,
                                align: FromA(380.0),
                                a = <View> {
                                    width: Fill, height: Fill, flow: Down,
                                    commits_hdr = <SectionHeader> { lbl = { text: "COMMITS" } }
                                    commit_list = <PortalList> {
                                        width: Fill, height: Fill,
                                        CommitRow = <CommitRow> {}
                                    }
                                }
                                b = <View> {
                                    width: Fill, height: Fill, flow: Down,
                                    files_hdr = <SectionHeader> { lbl = { text: "FILES" } }
                                    file_list = <PortalList> {
                                        width: Fill, height: Fill,
                                        FileRow = <FileRow> {}
                                    }
                                }
                            }
                        }
                        b = <View> {
                            width: Fill, height: Fill, flow: Down,
                            show_bg: true, draw_bg: { color: (BG) }
                            // toolbar
                            toolbar = <View> {
                                width: Fill, height: Fit, flow: Right, align: { y: 0.5 }
                                padding: { left: 10, right: 8, top: 6, bottom: 6 }
                                spacing: 6,
                                show_bg: true, draw_bg: { color: (BG) }
                                summary = <Label> {
                                    draw_text: { color: (TEXTCOL), text_style: <THEME_FONT_BOLD> { font_size: 12.0 } }
                                    text: ""
                                }
                                t_add = <Label> { draw_text: { color: (ADD_FG_DSL), text_style: { font_size: 12.0 } } text: "" }
                                t_del = <Label> { draw_text: { color: (DEL_FG_DSL), text_style: { font_size: 12.0 } } text: "" }
                                tfiller = <View> { width: Fill, height: 1 }
                                btn_wrap = <ToolBtn> { text: "⤶" }
                                btn_space = <ToolBtn> { text: "␣" }
                                btn_fdec = <ToolBtn> { text: "A-" }
                                btn_finc = <ToolBtn> { text: "A+" }
                                btn_lines = <ToolBtn> { text: "#" }
                            }
                            tsep = <View> { width: Fill, height: 1, show_bg: true, draw_bg: { color: (BORDER) } }
                            diff_list = <PortalList> {
                                width: Fill, height: Fill,
                                DiffRow = <DiffRow> {}
                            }
                        }
                    }
                }
            }
        }
    }
}

app_main!(App);

fn main() {
    app_main();
}

// ---------------------------------------------------------------------------
// Flattened diff rows for the virtualized diff PortalList.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Span {
    color: Vec4,
    text: String,
}

#[derive(Clone)]
enum RowKind {
    CommitMsg,
    FileHeader,
    HunkHeader,
    Code,
    Binary,
}

#[derive(Clone)]
struct Row {
    kind: RowKind,
    /// index into `diff.files`, for click-to-scroll mapping
    file_index: Option<usize>,
    bg: Vec4,
    mark: Vec4,
    line_kind: LineKind,
    old_no: Option<u32>,
    new_no: Option<u32>,
    sign: char,
    spans: Vec<Span>,
    // pre-formatted single-string content (headers / commit message)
    title: String,
    subtitle: String,
}

// ---------------------------------------------------------------------------
// Custom diff-row widget: draws background, gutter and colored spans.
// ---------------------------------------------------------------------------

#[derive(Live, Widget)]
struct DiffRow {
    #[redraw] #[rust] area: Area,
    #[walk] walk: Walk,
    #[live] draw_bg: DrawColor,
    #[live] draw_mark: DrawColor,
    #[live] draw_text: DrawText,

    #[rust] row: Option<Row>,
    #[rust(11.0f64)] font_size: f64,
    #[rust(true)] line_numbers: bool,
}

impl LiveHook for DiffRow {}

impl Widget for DiffRow {
    fn handle_event(&mut self, _cx: &mut Cx, _event: &Event, _scope: &mut Scope) {}

    fn draw_walk(&mut self, cx: &mut Cx2d, _scope: &mut Scope, walk: Walk) -> DrawStep {
        let Some(row) = self.row.clone() else {
            return DrawStep::done();
        };

        let fs = self.font_size;
        let char_w = fs * 0.62;
        let line_h = fs * 1.5;

        // Configure mono text style + size.
        self.draw_text.text_style.font_size = fs as f32;

        match row.kind {
            RowKind::CommitMsg => {
                let height = line_h * 2.2 + 16.0;
                let walk = Walk { width: Size::Fill, height: Size::Fixed(height), ..walk };
                let rect = cx.walk_turtle(walk);
                self.draw_bg.color = color_panel();
                self.draw_bg.draw_abs(cx, rect);
                self.draw_text.color = color_text();
                self.draw_text.draw_abs(cx, dvec2(rect.pos.x + 12.0, rect.pos.y + 8.0), &row.title);
                self.draw_text.color = color_muted();
                self.draw_text.draw_abs(cx, dvec2(rect.pos.x + 12.0, rect.pos.y + 8.0 + line_h), &row.subtitle);
                if !row.spans.is_empty() {
                    self.draw_text.color = color_text();
                    self.draw_text.draw_abs(cx, dvec2(rect.pos.x + 12.0, rect.pos.y + 8.0 + line_h * 2.2), &row.spans[0].text);
                }
                cx.add_aligned_rect_area(&mut self.area, rect);
            }
            RowKind::FileHeader => {
                let height = line_h + 12.0;
                let walk = Walk { width: Size::Fill, height: Size::Fixed(height), ..walk };
                let rect = cx.walk_turtle(walk);
                self.draw_bg.color = color_panel();
                self.draw_bg.draw_abs(cx, rect);
                self.draw_mark.color = color_border();
                self.draw_mark.draw_abs(cx, Rect { pos: rect.pos, size: dvec2(rect.size.x, 1.0) });
                self.draw_text.color = color_text();
                self.draw_text.draw_abs(cx, dvec2(rect.pos.x + 10.0, rect.pos.y + 6.0), &row.title);
                let counts = &row.subtitle;
                let cw = counts.chars().count() as f64 * char_w;
                self.draw_text.color = color_muted();
                self.draw_text.draw_abs(cx, dvec2(rect.pos.x + rect.size.x - cw - 12.0, rect.pos.y + 6.0), counts);
                cx.add_aligned_rect_area(&mut self.area, rect);
            }
            RowKind::Binary => {
                let height = line_h + 6.0;
                let walk = Walk { width: Size::Fill, height: Size::Fixed(height), ..walk };
                let rect = cx.walk_turtle(walk);
                self.draw_text.color = color_muted();
                self.draw_text.draw_abs(cx, dvec2(rect.pos.x + 14.0, rect.pos.y + 3.0), "Binary file not shown");
                cx.add_aligned_rect_area(&mut self.area, rect);
            }
            RowKind::HunkHeader | RowKind::Code => {
                let num_w = char_w * 4.0;
                let sign_w = char_w * 2.0;
                let gutter = if self.line_numbers && matches!(row.kind, RowKind::Code) {
                    num_w * 2.0 + sign_w
                } else {
                    sign_w
                };
                let walk = Walk { width: Size::Fill, height: Size::Fixed(line_h), ..walk };
                let rect = cx.walk_turtle(walk);

                if matches!(row.kind, RowKind::HunkHeader) {
                    self.draw_bg.color = color_hunk();
                    self.draw_bg.draw_abs(cx, rect);
                    self.draw_text.color = color_muted();
                    self.draw_text.draw_abs(cx, dvec2(rect.pos.x + 8.0, rect.pos.y + 2.0), &row.title);
                    cx.add_aligned_rect_area(&mut self.area, rect);
                    return DrawStep::done();
                }

                if row.bg.w > 0.0 {
                    self.draw_bg.color = row.bg;
                    self.draw_bg.draw_abs(cx, rect);
                    self.draw_mark.color = row.mark;
                    let mark_rect = Rect {
                        pos: dvec2(rect.pos.x + gutter - sign_w, rect.pos.y),
                        size: dvec2(sign_w, rect.size.y),
                    };
                    self.draw_mark.draw_abs(cx, mark_rect);
                }

                let top = rect.pos.y + 2.0;
                if self.line_numbers {
                    self.draw_text.color = color_muted();
                    if let Some(n) = row.old_no {
                        let s = n.to_string();
                        let x = rect.pos.x + num_w - 4.0 - s.chars().count() as f64 * char_w;
                        self.draw_text.draw_abs(cx, dvec2(x, top), &s);
                    }
                    if let Some(n) = row.new_no {
                        let s = n.to_string();
                        let x = rect.pos.x + num_w * 2.0 - 4.0 - s.chars().count() as f64 * char_w;
                        self.draw_text.draw_abs(cx, dvec2(x, top), &s);
                    }
                }
                if row.sign != ' ' {
                    self.draw_text.color = if row.line_kind == LineKind::Added { color_add_fg() } else { color_del_fg() };
                    self.draw_text.draw_abs(cx, dvec2(rect.pos.x + gutter - sign_w + 4.0, top), &row.sign.to_string());
                }

                // syntax-highlighted spans laid out left to right
                let mut x = rect.pos.x + gutter + 6.0;
                for sp in &row.spans {
                    self.draw_text.color = sp.color;
                    self.draw_text.draw_abs(cx, dvec2(x, top), &sp.text);
                    x += sp.text.chars().count() as f64 * char_w;
                }
                cx.add_aligned_rect_area(&mut self.area, rect);
            }
        }
        DrawStep::done()
    }
}

impl DiffRow {
    fn set_row(&mut self, row: Row, font_size: f64, line_numbers: bool) {
        self.row = Some(row);
        self.font_size = font_size;
        self.line_numbers = line_numbers;
    }
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Showing {
    Working,
    Commit(Oid),
    Range(Oid, Oid),
}

struct Settings {
    word_wrap: bool,
    show_space: bool,
    font_size: f64,
    line_numbers: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { word_wrap: false, show_space: true, font_size: 12.0, line_numbers: true }
    }
}

#[derive(Live)]
struct App {
    #[live]
    ui: WidgetRef,

    #[rust]
    model: Option<Model>,
}

struct Model {
    repo: Repo,
    hl: Highlighter,
    repo_name: String,
    commits: Vec<CommitInfo>,
    diff: DiffSet,
    rows: Vec<Row>,
    showing: Showing,
    from: Option<Oid>,
    to: Option<Oid>,
    settings: Settings,
    #[allow(dead_code)]
    error: Option<String>,
}

impl LiveHook for App {
    fn after_new_from_doc(&mut self, _cx: &mut Cx) {
        let path = std::env::args()
            .nth(1)
            .or_else(|| std::env::var("GIT_REVIEW_REPO").ok())
            .unwrap_or_else(|| ".".to_string());

        match Repo::open(&path) {
            Ok(repo) => {
                let hl = Highlighter::new();
                let repo_name = repo.workdir_name();
                let commits = repo.commits(500).unwrap_or_default();
                let showing = commits
                    .iter()
                    .find_map(|c| c.oid.map(Showing::Commit))
                    .unwrap_or(Showing::Working);
                let mut model = Model {
                    repo,
                    hl,
                    repo_name,
                    commits,
                    diff: empty_diff(),
                    rows: Vec::new(),
                    showing,
                    from: None,
                    to: None,
                    settings: Settings::default(),
                    error: None,
                };
                model.recompute();
                self.model = Some(model);
            }
            Err(e) => {
                error!("Failed to open repo at {path}: {}", e.message());
            }
        }
    }
}

impl LiveRegister for App {
    fn live_register(cx: &mut Cx) {
        makepad_widgets::live_design(cx);
    }
}

impl AppMain for App {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        // `match_event_with_draw_2d` routes Draw events to `handle_draw_2d` (our custom
        // PortalList draw loop) and everything else to the normal `match_event` handlers.
        if let Err(()) = self.match_event_with_draw_2d(cx, event) {
            let mut scope = Scope::empty();
            self.ui.handle_event(cx, event, &mut scope);
        }
    }
}

impl MatchEvent for App {
    fn handle_startup(&mut self, cx: &mut Cx) {
        self.refresh_chrome(cx);
        self.ui.redraw(cx);
    }

    fn handle_actions(&mut self, cx: &mut Cx, actions: &Actions) {
        let mut dirty_recompute = false;
        let mut dirty_chrome = false;

        // --- toolbar buttons ---
        if self.ui.button(id!(btn_wrap)).clicked(actions) {
            if let Some(m) = &mut self.model { m.settings.word_wrap = !m.settings.word_wrap; }
            dirty_chrome = true;
        }
        if self.ui.button(id!(btn_space)).clicked(actions) {
            if let Some(m) = &mut self.model { m.settings.show_space = !m.settings.show_space; }
            dirty_recompute = true;
        }
        if self.ui.button(id!(btn_fdec)).clicked(actions) {
            if let Some(m) = &mut self.model { m.settings.font_size = (m.settings.font_size - 1.0).max(8.0); }
            dirty_recompute = true;
        }
        if self.ui.button(id!(btn_finc)).clicked(actions) {
            if let Some(m) = &mut self.model { m.settings.font_size = (m.settings.font_size + 1.0).min(28.0); }
            dirty_recompute = true;
        }
        if self.ui.button(id!(btn_lines)).clicked(actions) {
            if let Some(m) = &mut self.model { m.settings.line_numbers = !m.settings.line_numbers; }
            dirty_chrome = true;
        }

        // --- commit list interactions ---
        let commit_list = self.ui.portal_list(id!(commit_list));
        let commits_len = self.model.as_ref().map(|m| m.commits.len()).unwrap_or(0);
        for i in 0..commits_len {
            if let Some((_, item)) = commit_list.get_item(i) {
                if item.button(id!(from_btn)).clicked(actions) {
                    if let Some(m) = &mut self.model {
                        if let Some(oid) = m.commits[i].oid { m.from = Some(oid); }
                    }
                    dirty_recompute |= self.maybe_range();
                    dirty_chrome = true;
                }
                if item.button(id!(to_btn)).clicked(actions) {
                    if let Some(m) = &mut self.model {
                        if let Some(oid) = m.commits[i].oid { m.to = Some(oid); }
                    }
                    dirty_recompute |= self.maybe_range();
                    dirty_chrome = true;
                }
                if item.as_view().finger_down(actions).is_some() {
                    self.open_commit(i);
                    dirty_recompute = true;
                }
            }
        }

        // --- file tree interactions ---
        let file_list = self.ui.portal_list(id!(file_list));
        let files_len = self.model.as_ref().map(|m| m.diff.files.len()).unwrap_or(0);
        for i in 0..files_len {
            if let Some((_, item)) = file_list.get_item(i) {
                if item.as_view().finger_down(actions).is_some() {
                    self.scroll_to_file(cx, i);
                }
            }
        }

        if dirty_recompute {
            if let Some(m) = &mut self.model { m.recompute(); }
            dirty_chrome = true;
        }
        if dirty_chrome {
            self.refresh_chrome(cx);
            self.ui.redraw(cx);
        }
    }

    fn handle_draw_2d(&mut self, cx: &mut Cx2d) {
        // Capture the list UIDs *before* the draw loop. `WidgetRef::widget_uid()`
        // returns 0 while the widget is borrowed, so we must not query it on the
        // currently-borrowed list during the loop.
        let commit_uid = self.ui.portal_list(id!(commit_list)).widget_uid();
        let file_uid = self.ui.portal_list(id!(file_list)).widget_uid();
        let diff_uid = self.ui.portal_list(id!(diff_list)).widget_uid();

        let mut scope = Scope::empty();
        while let Some(next) = self.ui.draw(cx, &mut scope).step() {
            if let Some(mut list) = next.as_portal_list().borrow_mut() {
                let uid = list.widget_uid();
                if uid == commit_uid {
                    self.draw_commit_list(cx, &mut list);
                } else if uid == file_uid {
                    self.draw_file_list(cx, &mut list);
                } else if uid == diff_uid {
                    self.draw_diff_list(cx, &mut list);
                }
            }
        }
    }
}

impl App {
    fn maybe_range(&mut self) -> bool {
        if let Some(m) = &mut self.model {
            if let (Some(a), Some(b)) = (m.from, m.to) {
                if m.showing != Showing::Range(a, b) {
                    m.showing = Showing::Range(a, b);
                    return true;
                }
            }
        }
        false
    }

    fn open_commit(&mut self, index: usize) {
        if let Some(m) = &mut self.model {
            let showing = match m.commits.get(index).and_then(|c| c.oid) {
                Some(oid) => Showing::Commit(oid),
                None => Showing::Working,
            };
            m.showing = showing;
        }
    }

    fn scroll_to_file(&mut self, cx: &mut Cx, file_index: usize) {
        if let Some(m) = &self.model {
            if let Some(row_idx) = m.rows.iter().position(|r| {
                matches!(r.kind, RowKind::FileHeader) && r.file_index == Some(file_index)
            }) {
                let list = self.ui.portal_list(id!(diff_list));
                list.smooth_scroll_to(cx, row_idx, 80.0, None);
                self.ui.redraw(cx);
            }
        }
    }

    fn refresh_chrome(&mut self, cx: &mut Cx) {
        let (summary, added, removed, repo_name, files_len, wrap, space, lines) = {
            let Some(m) = &self.model else { return };
            (
                m.diff.summary.clone(),
                m.diff.added,
                m.diff.removed,
                m.repo_name.clone(),
                m.diff.files.len(),
                m.settings.word_wrap,
                m.settings.show_space,
                m.settings.line_numbers,
            )
        };
        self.ui.label(id!(summary)).set_text(cx, &summary);
        self.ui.label(id!(t_add)).set_text(cx, &format!("+{}", added));
        self.ui.label(id!(t_del)).set_text(cx, &format!("−{}", removed));
        self.ui.label(id!(commits_hdr.lbl)).set_text(cx, &format!("COMMITS · {}", repo_name));
        self.ui.label(id!(files_hdr.lbl)).set_text(cx, &format!("FILES ({})", files_len));

        set_active(cx, &self.ui.button(id!(btn_wrap)), wrap);
        set_active(cx, &self.ui.button(id!(btn_space)), space);
        set_active(cx, &self.ui.button(id!(btn_lines)), lines);
    }

    fn draw_commit_list(&mut self, cx: &mut Cx2d, list: &mut PortalList) {
        let Some(m) = &self.model else { return };
        list.set_item_range(cx, 0, m.commits.len());
        while let Some(i) = list.next_visible_item(cx) {
            if i >= m.commits.len() { continue; }
            let c = &m.commits[i];
            let item = list.item(cx, i, live_id!(CommitRow));

            let is_current = match (m.showing, c.oid) {
                (Showing::Working, _) if c.is_working_tree() => true,
                (Showing::Commit(o), Some(oid)) => o == oid,
                _ => false,
            };
            let is_from = c.oid.is_some() && c.oid == m.from;
            let is_to = c.oid.is_some() && c.oid == m.to;

            item.label(id!(sha)).set_text(cx, &c.short);
            item.label(id!(date)).set_text(cx, &c.date);
            item.label(id!(author)).set_text(cx, &c.author);
            item.label(id!(title)).set_text(cx, &c.title);

            let bg = if is_current { color_sel() } else { color_panel() };
            item.apply_over(cx, live! { draw_bg: { color: (bg) } });
            set_active(cx, &item.button(id!(from_btn)), is_from);
            set_active(cx, &item.button(id!(to_btn)), is_to);

            item.button(id!(from_btn)).set_visible(cx, c.oid.is_some());
            item.button(id!(to_btn)).set_visible(cx, c.oid.is_some());

            item.draw_all(cx, &mut Scope::empty());
        }
    }

    fn draw_file_list(&mut self, cx: &mut Cx2d, list: &mut PortalList) {
        let Some(m) = &self.model else { return };
        list.set_item_range(cx, 0, m.diff.files.len());
        while let Some(i) = list.next_visible_item(cx) {
            if i >= m.diff.files.len() { continue; }
            let f = &m.diff.files[i];
            let item = list.item(cx, i, live_id!(FileRow));
            item.label(id!(icon)).set_text(cx, file_icon(&f.path));
            item.label(id!(name)).set_text(cx, &f.path);
            item.label(id!(fadd)).set_text(cx, &format!("+{}", f.added));
            item.label(id!(fdel)).set_text(cx, &format!("−{}", f.removed));
            item.draw_all(cx, &mut Scope::empty());
        }
    }

    fn draw_diff_list(&mut self, cx: &mut Cx2d, list: &mut PortalList) {
        let Some(m) = &self.model else { return };
        let (fs, ln) = (m.settings.font_size, m.settings.line_numbers);
        list.set_item_range(cx, 0, m.rows.len());
        while let Some(i) = list.next_visible_item(cx) {
            if i >= m.rows.len() { continue; }
            let row = m.rows[i].clone();
            let item = list.item(cx, i, live_id!(DiffRow));
            if let Some(mut dr) = item.borrow_mut::<DiffRow>() {
                dr.set_row(row, fs, ln);
            }
            item.draw_all(cx, &mut Scope::empty());
        }
    }
}

impl Model {
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
        self.rebuild_rows();
    }

    fn rebuild_rows(&mut self) {
        let mut rows = Vec::new();

        if let Some(msg) = &self.diff.message {
            let mut spans = Vec::new();
            if !msg.body.is_empty() {
                spans.push(Span { color: color_text(), text: msg.body.replace('\n', "  ") });
            }
            rows.push(Row {
                kind: RowKind::CommitMsg,
                file_index: None,
                bg: color_panel(),
                mark: vec4(0.0, 0.0, 0.0, 0.0),
                line_kind: LineKind::Context,
                old_no: None,
                new_no: None,
                sign: ' ',
                spans,
                title: msg.title.clone(),
                subtitle: format!("{}  ·  {}  ·  {}", msg.author, msg.date, msg.short),
            });
        }

        let files = self.diff.files.clone();
        for (fi, f) in files.iter().enumerate() {
            let label = match (&f.old_path, f.kind) {
                (Some(old), ChangeKind::Renamed) => format!("{old}  →  {}  [{}]", f.path, f.kind.letter()),
                _ => format!("{}  [{}]", f.path, f.kind.letter()),
            };
            rows.push(Row {
                kind: RowKind::FileHeader,
                file_index: Some(fi),
                bg: color_panel(),
                mark: vec4(0.0, 0.0, 0.0, 0.0),
                line_kind: LineKind::Context,
                old_no: None,
                new_no: None,
                sign: ' ',
                spans: Vec::new(),
                title: label,
                subtitle: format!("+{}  −{}", f.added, f.removed),
            });

            if f.binary {
                rows.push(blank_row(RowKind::Binary, Some(fi)));
                continue;
            }

            let syntax = self.hl.syntax_for(&f.path).clone();
            for hunk in &f.hunks {
                rows.push(Row {
                    kind: RowKind::HunkHeader,
                    file_index: Some(fi),
                    bg: color_hunk(),
                    mark: vec4(0.0, 0.0, 0.0, 0.0),
                    line_kind: LineKind::Context,
                    old_no: None,
                    new_no: None,
                    sign: ' ',
                    spans: Vec::new(),
                    title: hunk.header.clone(),
                    subtitle: String::new(),
                });
                for line in &hunk.lines {
                    let (bg, mark, sign) = match line.kind {
                        LineKind::Added => (color_add_bg(), color_add_mark(), '+'),
                        LineKind::Removed => (color_del_bg(), color_del_mark(), '-'),
                        LineKind::Context => (vec4(0.0, 0.0, 0.0, 0.0), vec4(0.0, 0.0, 0.0, 0.0), ' '),
                    };
                    let hspans = self.hl.line(&syntax, &line.text);
                    let spans = hspans
                        .into_iter()
                        .map(|s| Span { color: rgb_to_vec4(s.color), text: s.text })
                        .collect();
                    rows.push(Row {
                        kind: RowKind::Code,
                        file_index: Some(fi),
                        bg,
                        mark,
                        line_kind: line.kind,
                        old_no: line.old_no,
                        new_no: line.new_no,
                        sign,
                        spans,
                        title: String::new(),
                        subtitle: String::new(),
                    });
                }
            }
        }

        self.rows = rows;
    }
}

fn blank_row(kind: RowKind, file_index: Option<usize>) -> Row {
    Row {
        kind,
        file_index,
        bg: vec4(0.0, 0.0, 0.0, 0.0),
        mark: vec4(0.0, 0.0, 0.0, 0.0),
        line_kind: LineKind::Context,
        old_no: None,
        new_no: None,
        sign: ' ',
        spans: Vec::new(),
        title: String::new(),
        subtitle: String::new(),
    }
}

fn set_active(cx: &mut Cx, btn: &ButtonRef, active: bool) {
    let v = if active { 1.0f64 } else { 0.0f64 };
    btn.apply_over(cx, live! { draw_bg: { active: (v) }, draw_text: { active: (v) } });
}

// ---- color helpers (match the DSL palette) --------------------------------
fn rgb_to_vec4(c: (u8, u8, u8)) -> Vec4 {
    vec4(c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0, 1.0)
}
fn color_text() -> Vec4 { rgb_to_vec4((0x1f, 0x23, 0x28)) }
fn color_muted() -> Vec4 { rgb_to_vec4((0x65, 0x6d, 0x76)) }
fn color_border() -> Vec4 { rgb_to_vec4((0xd0, 0xd7, 0xde)) }
fn color_panel() -> Vec4 { rgb_to_vec4((0xf6, 0xf8, 0xfa)) }
fn color_sel() -> Vec4 { rgb_to_vec4((0xdd, 0xf4, 0xff)) }
fn color_hunk() -> Vec4 { rgb_to_vec4((0xdd, 0xf4, 0xff)) }
fn color_add_bg() -> Vec4 { rgb_to_vec4((0xe6, 0xff, 0xec)) }
fn color_add_mark() -> Vec4 { rgb_to_vec4((0xab, 0xf2, 0xbc)) }
fn color_del_bg() -> Vec4 { rgb_to_vec4((0xff, 0xeb, 0xe9)) }
fn color_del_mark() -> Vec4 { rgb_to_vec4((0xff, 0x81, 0x82)) }
fn color_add_fg() -> Vec4 { rgb_to_vec4((0x1a, 0x7f, 0x37)) }
fn color_del_fg() -> Vec4 { rgb_to_vec4((0xcf, 0x22, 0x2e)) }

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

fn empty_diff() -> DiffSet {
    DiffSet {
        message: None,
        files: Vec::new(),
        added: 0,
        removed: 0,
        summary: String::new(),
    }
}
