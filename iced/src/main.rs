//! The iced UI for git-review.
//!
//! Elm architecture: `App` state, a `Message` enum, `update`, and `view`. The data layer
//! (`git`, `highlight`) is shared verbatim with the other framework apps in this repo.

mod git;
mod highlight;

use std::collections::{HashMap, HashSet};

use iced::widget::operation::{self, AbsoluteOffset};
use iced::widget::pane_grid::{self, PaneGrid};
use iced::widget::{
    button, column, container, mouse_area, rich_text, row, scrollable, span, text, tooltip,
    Column, Id, Space,
};
use iced::{
    Background, Border, Color, Element, Fill, Font, Length, Padding, Shrink, Task, Theme,
};
use git2::Oid;

use git::{ChangeKind, CommitInfo, CommitMessage, DiffSet, FileDiff, LineKind, Repo};
use highlight::Highlighter;

// --- GitHub-ish palette (light + dark), selected from the system scheme ------

#[derive(Clone, Copy)]
struct Palette {
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

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}

impl Palette {
    fn light() -> Self {
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
            btn_bg: rgb(0xff, 0xff, 0xff),
        }
    }

    fn dark() -> Self {
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
}

const MONO: Font = Font::MONOSPACE;

// --- file tree model --------------------------------------------------------

/// A node in the rendered, hierarchical file tree. Built from the flat list of
/// `FileDiff.path`s. Folder labels may collapse single-child chains GitHub-style.
enum TreeNode {
    /// A folder: a (possibly collapsed) path segment label, a stable key for
    /// expand/collapse state, and its children.
    Dir {
        label: String,
        key: String,
        children: Vec<TreeNode>,
    },
    /// A file leaf, with the index into `diff.files`.
    File { name: String, idx: usize },
}

/// Intermediate builder node.
struct BuildDir {
    children: Vec<BuildEntry>,
}
enum BuildEntry {
    Dir(String, BuildDir),
    File(String, usize),
}

impl BuildDir {
    fn new() -> Self {
        Self { children: Vec::new() }
    }
    fn dir_mut(&mut self, name: &str) -> &mut BuildDir {
        // Find existing dir child or create one.
        if let Some(pos) = self.children.iter().position(|e| matches!(e, BuildEntry::Dir(n, _) if n == name)) {
            if let BuildEntry::Dir(_, d) = &mut self.children[pos] {
                return d;
            }
            unreachable!()
        }
        self.children.push(BuildEntry::Dir(name.to_string(), BuildDir::new()));
        match self.children.last_mut().unwrap() {
            BuildEntry::Dir(_, d) => d,
            _ => unreachable!(),
        }
    }
    fn add_file(&mut self, name: &str, idx: usize) {
        self.children.push(BuildEntry::File(name.to_string(), idx));
    }
}

fn build_tree(files: &[FileDiff]) -> Vec<TreeNode> {
    let mut root = BuildDir::new();
    for (idx, f) in files.iter().enumerate() {
        let comps: Vec<&str> = f.path.split('/').filter(|s| !s.is_empty()).collect();
        if comps.is_empty() {
            continue;
        }
        let mut cur = &mut root;
        for comp in &comps[..comps.len() - 1] {
            cur = cur.dir_mut(comp);
        }
        cur.add_file(comps[comps.len() - 1], idx);
    }
    finalize(&root.children, String::new())
}

/// Convert builder entries into render nodes, collapsing single-child dir chains.
fn finalize(entries: &[BuildEntry], prefix: String) -> Vec<TreeNode> {
    // Sort: directories first, then files, each alphabetically — GitHub-style.
    let mut dirs: Vec<&BuildEntry> = entries
        .iter()
        .filter(|e| matches!(e, BuildEntry::Dir(..)))
        .collect();
    let mut leaves: Vec<&BuildEntry> = entries
        .iter()
        .filter(|e| matches!(e, BuildEntry::File(..)))
        .collect();
    dirs.sort_by(|a, b| name_of(a).cmp(name_of(b)));
    leaves.sort_by(|a, b| name_of(a).cmp(name_of(b)));

    let mut out = Vec::new();
    for e in dirs {
        if let BuildEntry::Dir(name, d) = e {
            // Collapse single-child dir chains: a/b/c -> one node "a/b/c".
            let mut label = name.clone();
            let mut key = format!("{prefix}{name}");
            let mut cur = d;
            while cur.children.len() == 1 {
                if let BuildEntry::Dir(child_name, child_dir) = &cur.children[0] {
                    label = format!("{label}/{child_name}");
                    key = format!("{key}/{child_name}");
                    cur = child_dir;
                } else {
                    break;
                }
            }
            let children = finalize(&cur.children, format!("{key}/"));
            out.push(TreeNode::Dir { label, key, children });
        }
    }
    for e in leaves {
        if let BuildEntry::File(name, idx) = e {
            out.push(TreeNode::File { name: name.clone(), idx: *idx });
        }
    }
    out
}

fn name_of(e: &BuildEntry) -> &str {
    match e {
        BuildEntry::Dir(n, _) => n,
        BuildEntry::File(n, _) => n,
    }
}

// --- state ------------------------------------------------------------------

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

/// Which logical pane lives in each `pane_grid` slot.
#[derive(Clone, Copy)]
enum PaneKind {
    Commits,
    Files,
    Main,
}

struct App {
    repo: Repo,
    hl: Highlighter,
    pal: Palette,
    repo_name: String,
    commits: Vec<CommitInfo>,
    diff: DiffSet,
    tree: Vec<TreeNode>,
    /// Collapsed folder keys (default = expanded, so absence means open).
    collapsed: HashSet<String>,
    showing: Showing,
    from: Option<Oid>,
    to: Option<Oid>,
    settings: Settings,
    error: Option<String>,

    panes: pane_grid::State<PaneKind>,
    diff_scroll: Id,
    /// Per-file vertical offset within the diff scroll content (px from top).
    file_tops: HashMap<usize, f32>,
    diff_offset: f32,
    diff_viewport_h: f32,
}

#[derive(Debug, Clone)]
enum Message {
    SelectWorking,
    SelectCommit(Oid),
    SetFrom(Oid),
    SetTo(Oid),
    OpenFile(usize),
    ToggleFolder(String),
    ToggleWordWrap,
    ToggleShowSpace,
    FontDec,
    FontInc,
    ToggleLineNumbers,
    PaneResized(pane_grid::ResizeEvent),
    DiffScrolled(scrollable::Viewport),
}

impl App {
    fn new(repo: Repo) -> (Self, Task<Message>) {
        // Detect the desktop colour scheme; headless -> light.
        // Follow the desktop scheme; `GIT_REVIEW_THEME=dark|light` can force it
        // (useful in a headless environment, which otherwise defaults to light).
        let dark = match std::env::var("GIT_REVIEW_THEME").ok().as_deref() {
            Some("dark") => true,
            Some("light") => false,
            _ => matches!(dark_light::detect(), Ok(dark_light::Mode::Dark)),
        };
        let pal = if dark { Palette::dark() } else { Palette::light() };
        let hl = Highlighter::new(dark);
        let repo_name = repo.workdir_name();
        let commits = repo.commits(500).unwrap_or_default();
        let showing = commits
            .iter()
            .find_map(|c| c.oid.map(Showing::Commit))
            .unwrap_or(Showing::Working);

        // Layout: left side panel split vertically (commits / files) | main.
        let config = pane_grid::Configuration::Split {
            axis: pane_grid::Axis::Vertical,
            ratio: 0.28,
            a: Box::new(pane_grid::Configuration::Split {
                axis: pane_grid::Axis::Horizontal,
                ratio: 0.55,
                a: Box::new(pane_grid::Configuration::Pane(PaneKind::Commits)),
                b: Box::new(pane_grid::Configuration::Pane(PaneKind::Files)),
            }),
            b: Box::new(pane_grid::Configuration::Pane(PaneKind::Main)),
        };

        let mut app = Self {
            repo,
            hl,
            pal,
            repo_name,
            commits,
            diff: empty_diff(),
            tree: Vec::new(),
            collapsed: HashSet::new(),
            showing,
            from: None,
            to: None,
            settings: Settings::default(),
            error: None,
            panes: pane_grid::State::with_configuration(config),
            diff_scroll: Id::unique(),
            file_tops: HashMap::new(),
            diff_offset: 0.0,
            diff_viewport_h: 600.0,
        };
        app.recompute();
        (app, Task::none())
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
        self.tree = build_tree(&self.diff.files);
        self.compute_file_tops();
    }

    fn select(&mut self, showing: Showing) {
        if self.showing != showing {
            self.showing = showing;
            self.diff_offset = 0.0;
            self.recompute();
        }
    }

    fn maybe_range(&mut self) {
        if let (Some(a), Some(b)) = (self.from, self.to) {
            self.select(Showing::Range(a, b));
        }
    }

    /// Estimate the y-offset of each file's header inside the diff scroll content.
    fn compute_file_tops(&mut self) {
        self.file_tops.clear();
        let row_h = self.settings.font_size * ROW_LINE_HEIGHT;
        let header_h = 30.0;
        let file_gap = 12.0;
        let mut y = 0.0f32;
        if let Some(msg) = &self.diff.message {
            let body_lines = if msg.body.is_empty() {
                0
            } else {
                msg.body.lines().count().max(1)
            };
            y += 24.0 + 18.0 + (body_lines as f32) * row_h + 24.0 + 12.0;
        }
        for (i, f) in self.diff.files.iter().enumerate() {
            self.file_tops.insert(i, y);
            y += header_h;
            if f.binary {
                y += row_h;
            } else {
                for h in &f.hunks {
                    y += row_h; // hunk header
                    y += h.lines.len() as f32 * row_h;
                }
            }
            y += file_gap;
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SelectWorking => self.select(Showing::Working),
            Message::SelectCommit(oid) => self.select(Showing::Commit(oid)),
            Message::SetFrom(oid) => {
                self.from = Some(oid);
                self.maybe_range();
            }
            Message::SetTo(oid) => {
                self.to = Some(oid);
                self.maybe_range();
            }
            Message::OpenFile(i) => {
                if let Some(y) = self.file_tops.get(&i).copied() {
                    return operation::scroll_to(
                        self.diff_scroll.clone(),
                        AbsoluteOffset { x: 0.0, y },
                    );
                }
            }
            Message::ToggleFolder(key) => {
                if !self.collapsed.remove(&key) {
                    self.collapsed.insert(key);
                }
            }
            Message::ToggleWordWrap => self.settings.word_wrap = !self.settings.word_wrap,
            Message::ToggleShowSpace => {
                self.settings.show_space = !self.settings.show_space;
                self.recompute();
            }
            Message::FontDec => {
                self.settings.font_size = (self.settings.font_size - 1.0).max(8.0);
                self.compute_file_tops();
            }
            Message::FontInc => {
                self.settings.font_size = (self.settings.font_size + 1.0).min(28.0);
                self.compute_file_tops();
            }
            Message::ToggleLineNumbers => {
                self.settings.line_numbers = !self.settings.line_numbers
            }
            Message::PaneResized(pane_grid::ResizeEvent { split, ratio }) => {
                self.panes.resize(split, ratio);
            }
            Message::DiffScrolled(vp) => {
                self.diff_offset = vp.absolute_offset().y;
                self.diff_viewport_h = vp.bounds().height;
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let p = self.pal;
        let grid = PaneGrid::new(&self.panes, |_pane, kind, _focused| {
            let body: Element<Message> = match kind {
                PaneKind::Commits => self.commit_list(),
                PaneKind::Files => self.file_tree(),
                PaneKind::Main => self.main_view(),
            };
            pane_grid::Content::new(body)
        })
        .spacing(1)
        .on_resize(8, Message::PaneResized)
        .style(move |_theme| pane_grid::Style {
            hovered_region: pane_grid::Highlight {
                background: Background::Color(p.sel),
                border: Border::default(),
            },
            picked_split: pane_grid::Line {
                color: p.accent,
                width: 2.0,
            },
            hovered_split: pane_grid::Line {
                color: p.accent,
                width: 2.0,
            },
        });

        // Full-width toolbar pinned at the very top, above the side panel + main.
        let toolbar = self.toolbar();
        let rest = container(grid)
            .width(Fill)
            .height(Fill)
            .style(move |_t| container::Style {
                background: Some(Background::Color(p.border)),
                ..Default::default()
            });

        container(column![toolbar, rest].spacing(0))
            .width(Fill)
            .height(Fill)
            .style(move |_t| container::Style {
                background: Some(Background::Color(p.bg)),
                ..Default::default()
            })
            .into()
    }

    // --- commit list --------------------------------------------------------
    fn commit_list(&self) -> Element<'_, Message> {
        let header = self.section_header(&format!("COMMITS · {}", self.repo_name));

        let mut list = Column::new().width(Fill);
        for (idx, c) in self.commits.iter().enumerate() {
            list = list.push(self.commit_row(idx, c));
            list = list.push(self.thin_rule());
        }

        let scroll = scrollable(list).width(Fill).height(Fill);
        let p = self.pal;
        container(column![header, scroll].spacing(0))
            .width(Fill)
            .height(Fill)
            .style(move |_t| panel_style(p))
            .into()
    }

    fn commit_row<'a>(&'a self, _idx: usize, c: &'a CommitInfo) -> Element<'a, Message> {
        let p = self.pal;
        let is_current = match (self.showing, c.oid) {
            (Showing::Working, _) if c.is_working_tree() => true,
            (Showing::Commit(o), Some(oid)) => o == oid,
            _ => false,
        };
        let is_from = c.oid.is_some() && c.oid == self.from;
        let is_to = c.oid.is_some() && c.oid == self.to;

        let mut line1 = row![].spacing(4).align_y(iced::Alignment::Center);
        if let Some(oid) = c.oid {
            line1 = line1
                .push(endpoint_btn(p, "◀", "Compare from this commit", is_from, Message::SetFrom(oid)))
                .push(endpoint_btn(p, "▶", "Compare to this commit", is_to, Message::SetTo(oid)));
        } else {
            line1 = line1.push(Space::new().width(Length::Fixed(40.0)));
        }
        line1 = line1
            .push(text(c.short.clone()).font(MONO).size(11).color(p.accent))
            .push(text(c.date.clone()).size(10).color(p.muted))
            .push(Space::new().width(Fill))
            .push(
                text(c.author.clone())
                    .size(10)
                    .color(p.muted)
                    .wrapping(text::Wrapping::None),
            );

        let line2 = text(c.title.clone())
            .size(12)
            .color(p.text)
            .wrapping(text::Wrapping::None);

        let inner = column![line1, line2].spacing(2).width(Fill);

        let msg = match c.oid {
            Some(oid) => Message::SelectCommit(oid),
            None => Message::SelectWorking,
        };

        let cell = container(inner)
            .padding(Padding::from([5.0, 8.0]))
            .width(Fill)
            .style(move |_t| container::Style {
                background: Some(Background::Color(if is_current { p.sel } else { p.panel })),
                text_color: if is_current && p.dark { Some(rgb(0xff, 0xff, 0xff)) } else { None },
                ..Default::default()
            });

        mouse_area(cell)
            .on_press(msg)
            .interaction(iced::mouse::Interaction::Pointer)
            .into()
    }

    // --- file tree ----------------------------------------------------------
    fn file_tree(&self) -> Element<'_, Message> {
        let header = self.section_header(&format!("FILES ({})", self.diff.files.len()));

        let mut rows: Vec<Element<'_, Message>> = Vec::new();
        for node in &self.tree {
            self.render_node(node, 0, &mut rows);
        }
        let list = Column::with_children(rows).width(Fill);

        let scroll = scrollable(list).width(Fill).height(Fill);
        let p = self.pal;
        container(column![header, scroll].spacing(0))
            .width(Fill)
            .height(Fill)
            .style(move |_t| panel_style(p))
            .into()
    }

    /// Recursively render a tree node into a flat list of rows, indented by depth.
    fn render_node<'a>(&'a self, node: &'a TreeNode, depth: usize, out: &mut Vec<Element<'a, Message>>) {
        let p = self.pal;
        let indent = 8.0 + depth as f32 * 14.0;
        match node {
            TreeNode::Dir { label, key, children } => {
                let open = !self.collapsed.contains(key);
                let arrow = if open { "▾" } else { "▸" };
                let line = row![
                    Space::new().width(Length::Fixed(indent)),
                    text(arrow.to_string()).size(10).color(p.muted),
                    text("📁").size(13),
                    text(label.clone())
                        .size(12)
                        .color(p.text)
                        .font(Font { weight: iced::font::Weight::Semibold, ..Font::DEFAULT })
                        .wrapping(text::Wrapping::None),
                ]
                .spacing(5)
                .align_y(iced::Alignment::Center);

                let cell = container(line)
                    .padding(Padding::from([3.0, 6.0]))
                    .width(Fill);

                out.push(
                    mouse_area(cell)
                        .on_press(Message::ToggleFolder(key.clone()))
                        .interaction(iced::mouse::Interaction::Pointer)
                        .into(),
                );

                if open {
                    for child in children {
                        self.render_node(child, depth + 1, out);
                    }
                }
            }
            TreeNode::File { name, idx } => {
                let f = &self.diff.files[*idx];
                let line = row![
                    Space::new().width(Length::Fixed(indent + 14.0)),
                    text(file_icon(&f.path)).size(13),
                    text(name.clone())
                        .size(12)
                        .color(p.text)
                        .wrapping(text::Wrapping::None),
                    Space::new().width(Fill),
                    text(format!("+{}", f.added)).size(10).color(p.add_fg),
                    text(format!("−{}", f.removed)).size(10).color(p.del_fg),
                ]
                .spacing(5)
                .align_y(iced::Alignment::Center);

                let cell = container(line)
                    .padding(Padding::from([3.0, 6.0]))
                    .width(Fill);

                out.push(
                    mouse_area(cell)
                        .on_press(Message::OpenFile(*idx))
                        .interaction(iced::mouse::Interaction::Pointer)
                        .into(),
                );
            }
        }
    }

    // --- main view (diff only; toolbar is now full-width on top) ------------
    fn main_view(&self) -> Element<'_, Message> {
        let p = self.pal;
        container(self.diff_view())
            .width(Fill)
            .height(Fill)
            .style(move |_t| container::Style {
                background: Some(Background::Color(p.bg)),
                ..Default::default()
            })
            .into()
    }

    fn toolbar(&self) -> Element<'_, Message> {
        let p = self.pal;
        let summary = row![
            text(self.diff.summary.clone())
                .font(Font { weight: iced::font::Weight::Bold, ..MONO })
                .size(13)
                .color(p.text),
            text(format!("+{}", self.diff.added)).size(13).color(p.add_fg),
            text(format!("−{}", self.diff.removed)).size(13).color(p.del_fg),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

        let buttons = row![
            tool_btn(p, "⤶", "Word wrap", self.settings.word_wrap, Message::ToggleWordWrap),
            tool_btn(p, "␣", "Show space changes", self.settings.show_space, Message::ToggleShowSpace),
            tool_btn(p, "A-", "Decrease font size", false, Message::FontDec),
            tool_btn(p, "A+", "Increase font size", false, Message::FontInc),
            tool_btn(p, "#", "Show line numbers", self.settings.line_numbers, Message::ToggleLineNumbers),
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center);

        let bar = row![summary, Space::new().width(Fill), buttons]
            .spacing(8)
            .align_y(iced::Alignment::Center)
            .width(Fill);

        container(bar)
            .padding(Padding::from([7.0, 12.0]))
            .width(Fill)
            .style(move |_t| container::Style {
                background: Some(Background::Color(p.panel)),
                border: Border {
                    color: p.border,
                    width: 1.0,
                    radius: 0.0.into(),
                },
                ..Default::default()
            })
            .into()
    }

    fn diff_view(&self) -> Element<'_, Message> {
        let p = self.pal;
        if let Some(err) = &self.error {
            return container(text(format!("Error: {err}")).color(p.del_fg))
                .padding(12)
                .into();
        }

        let mut body = Column::new().width(Fill).spacing(0);

        if let Some(msg) = &self.diff.message {
            body = body.push(commit_message_ui(p, msg, self.settings.font_size));
        }

        for (i, f) in self.diff.files.iter().enumerate() {
            body = body.push(self.file_header(f));
            body = body.push(self.file_body(f, i));
            body = body.push(Space::new().height(Length::Fixed(12.0)));
        }

        let direction = if self.settings.word_wrap {
            scrollable::Direction::Vertical(scrollable::Scrollbar::new())
        } else {
            scrollable::Direction::Both {
                vertical: scrollable::Scrollbar::new(),
                horizontal: scrollable::Scrollbar::new(),
            }
        };

        let scroll = scrollable(container(body))
            .direction(direction)
            .id(self.diff_scroll.clone())
            .on_scroll(Message::DiffScrolled)
            .width(Fill)
            .height(Fill);

        let sticky = self.sticky_header();
        let stacked: Element<Message> = if let Some(h) = sticky {
            iced::widget::stack![scroll, h].into()
        } else {
            scroll.into()
        };

        container(stacked).width(Fill).height(Fill).into()
    }

    fn sticky_header(&self) -> Option<Element<'_, Message>> {
        if self.diff.files.is_empty() {
            return None;
        }
        let off = self.diff_offset;
        let mut current: Option<usize> = None;
        for (i, _f) in self.diff.files.iter().enumerate() {
            let top = *self.file_tops.get(&i)?;
            if top <= off + 0.5 {
                current = Some(i);
            } else {
                break;
            }
        }
        let i = current?;
        let f = &self.diff.files[i];
        Some(container(self.file_header_inner(f, true)).width(Fill).into())
    }

    fn file_header(&self, f: &FileDiff) -> Element<'_, Message> {
        self.file_header_inner(f, false)
    }

    fn file_header_inner(&self, f: &FileDiff, sticky: bool) -> Element<'_, Message> {
        let p = self.pal;
        let label = match (&f.old_path, f.kind) {
            (Some(old), ChangeKind::Renamed) => format!("{old}  →  {}", f.path),
            _ => f.path.clone(),
        };
        let bar = row![
            text(file_icon(&f.path)).size(13),
            text(label)
                .font(Font { weight: iced::font::Weight::Semibold, ..Font::DEFAULT })
                .size(13)
                .color(p.text),
            text(format!("[{}]", f.kind.letter())).size(10).color(p.muted),
            Space::new().width(Fill),
            text(format!("+{}", f.added)).size(12).color(p.add_fg),
            text(format!("−{}", f.removed)).size(12).color(p.del_fg),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center)
        .width(Fill);

        container(bar)
            .padding(Padding::from([6.0, 10.0]))
            .width(Fill)
            .style(move |_t| container::Style {
                background: Some(Background::Color(p.panel)),
                border: Border {
                    color: p.border,
                    width: 1.0,
                    radius: 0.0.into(),
                },
                shadow: if sticky {
                    iced::Shadow {
                        color: Color::from_rgba8(0, 0, 0, 0.18),
                        offset: iced::Vector::new(0.0, 2.0),
                        blur_radius: 3.0,
                    }
                } else {
                    iced::Shadow::default()
                },
                ..Default::default()
            })
            .into()
    }

    fn file_body(&self, f: &FileDiff, _idx: usize) -> Element<'_, Message> {
        let p = self.pal;
        if f.binary {
            return container(text("Binary file not shown").size(12).color(p.muted))
                .padding(Padding::from([4.0, 12.0]))
                .into();
        }
        let syntax = self.hl.syntax_for(&f.path);
        let mut col = Column::new().width(Shrink);
        for hunk in &f.hunks {
            col = col.push(self.diff_row(LineKind::Context, None, None, &hunk.header, None, true));
            for line in &hunk.lines {
                col = col.push(self.diff_row(
                    line.kind,
                    line.old_no,
                    line.new_no,
                    &line.text,
                    Some(syntax),
                    false,
                ));
            }
        }
        let width = if self.settings.word_wrap { Fill } else { Shrink };
        container(col).width(width).into()
    }

    #[allow(clippy::too_many_arguments)]
    fn diff_row(
        &self,
        kind: LineKind,
        old_no: Option<u32>,
        new_no: Option<u32>,
        line_text: &str,
        syntax: Option<&syntect::parsing::SyntaxReference>,
        hunk_header: bool,
    ) -> Element<'_, Message> {
        let p = self.pal;
        let fs = self.settings.font_size;
        let char_w = fs * 0.62;
        let num_w = char_w * 4.0;

        let (bg, mark, sign, sign_color) = match kind {
            LineKind::Added => (Some(p.add_bg), Some(p.add_mark), '+', p.add_fg),
            LineKind::Removed => (Some(p.del_bg), Some(p.del_mark), '-', p.del_fg),
            LineKind::Context => (None, None, ' ', p.muted),
        };

        let mut left = row![].align_y(iced::Alignment::Center);

        if self.settings.line_numbers && !hunk_header {
            let old_s = old_no.map(|n| n.to_string()).unwrap_or_default();
            let new_s = new_no.map(|n| n.to_string()).unwrap_or_default();
            left = left.push(
                container(text(old_s).font(MONO).size(fs).color(p.muted))
                    .width(Length::Fixed(num_w))
                    .align_x(iced::Alignment::End),
            );
            left = left.push(
                container(text(new_s).font(MONO).size(fs).color(p.muted))
                    .width(Length::Fixed(num_w))
                    .align_x(iced::Alignment::End),
            );
        }

        let sign_cell = container(text(sign.to_string()).font(MONO).size(fs).color(sign_color))
            .width(Length::Fixed(char_w * 1.6))
            .align_x(iced::Alignment::Center)
            .style(move |_t| container::Style {
                background: mark.map(Background::Color),
                ..Default::default()
            });
        if !hunk_header {
            left = left.push(sign_cell);
        }

        let code: Element<Message> = if hunk_header {
            text(line_text.to_string()).font(MONO).size(fs).color(p.accent).into()
        } else if let Some(syntax) = syntax {
            let spans = self.hl.line(syntax, line_text);
            if spans.is_empty() {
                text(" ").font(MONO).size(fs).into()
            } else {
                let spans: Vec<text::Span<'_, ()>> = spans
                    .into_iter()
                    .map(|s| {
                        span(s.text).font(MONO).size(fs).color(rgb(s.color.0, s.color.1, s.color.2))
                    })
                    .collect();
                let rt = rich_text(spans);
                let rt = if self.settings.word_wrap {
                    rt.wrapping(text::Wrapping::Word)
                } else {
                    rt.wrapping(text::Wrapping::None)
                };
                rt.into()
            }
        } else {
            text(line_text.to_string()).font(MONO).size(fs).color(p.text).into()
        };

        let code_cell = container(code)
            .padding(Padding::from([0.0, 6.0]))
            .width(if self.settings.word_wrap { Fill } else { Shrink });

        let line = row![left, code_cell].align_y(iced::Alignment::Center);

        let row_bg = if hunk_header { Some(p.sel) } else { bg };
        let width = if self.settings.word_wrap { Fill } else { Shrink };

        container(line)
            .width(width)
            .style(move |_t| container::Style {
                background: row_bg.map(Background::Color),
                ..Default::default()
            })
            .into()
    }

    // --- small widgets that need the palette --------------------------------

    fn section_header(&self, label: &str) -> Element<'_, Message> {
        let p = self.pal;
        container(
            text(label.to_string())
                .size(11)
                .color(p.muted)
                .font(Font { weight: iced::font::Weight::Bold, ..Font::DEFAULT }),
        )
        .padding(Padding::from([6.0, 8.0]))
        .width(Fill)
        .style(move |_t| panel_style(p))
        .into()
    }

    fn thin_rule(&self) -> Element<'_, Message> {
        let p = self.pal;
        container(Space::new().height(Length::Fixed(1.0)))
            .width(Fill)
            .style(move |_t| container::Style {
                background: Some(Background::Color(p.border)),
                ..Default::default()
            })
            .into()
    }
}

// row line-height multiplier used both for layout and offset estimation.
const ROW_LINE_HEIGHT: f32 = 1.45;

// --- free helper widgets ----------------------------------------------------

fn panel_style(p: Palette) -> container::Style {
    container::Style {
        background: Some(Background::Color(p.panel)),
        ..Default::default()
    }
}

fn tool_btn(p: Palette, label: &str, tip: &str, active: bool, msg: Message) -> Element<'static, Message> {
    let txt = text(label.to_string())
        .font(MONO)
        .size(13)
        .color(if active { p.accent } else { p.text });

    let b = button(txt)
        .padding(Padding::from([3.0, 7.0]))
        .on_press(msg)
        .style(move |_t, _status| button::Style {
            background: Some(Background::Color(if active { p.sel } else { p.btn_bg })),
            text_color: if active { p.accent } else { p.text },
            border: Border {
                color: p.border,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..Default::default()
        });

    tooltip(
        b,
        container(text(tip.to_string()).size(11).color(p.bg))
            .padding(Padding::from([3.0, 6.0]))
            .style(move |_t| container::Style {
                background: Some(Background::Color(p.text)),
                border: Border { color: p.border, width: 0.0, radius: 4.0.into() },
                ..Default::default()
            }),
        tooltip::Position::Bottom,
    )
    .into()
}

fn endpoint_btn(p: Palette, label: &str, tip: &str, active: bool, msg: Message) -> Element<'static, Message> {
    let b = button(text(label.to_string()).size(9).color(if active { p.bg } else { p.text }))
        .padding(Padding::from([1.0, 4.0]))
        .on_press(msg)
        .style(move |_t, _status| button::Style {
            background: Some(Background::Color(if active { p.accent } else { p.btn_bg })),
            text_color: if active { p.bg } else { p.text },
            border: Border { color: p.border, width: 1.0, radius: 3.0.into() },
            ..Default::default()
        });
    tooltip(
        b,
        container(text(tip.to_string()).size(11).color(p.bg))
            .padding(Padding::from([3.0, 6.0]))
            .style(move |_t| container::Style {
                background: Some(Background::Color(p.text)),
                border: Border { color: p.border, width: 0.0, radius: 4.0.into() },
                ..Default::default()
            }),
        tooltip::Position::Bottom,
    )
    .into()
}

fn commit_message_ui(p: Palette, msg: &CommitMessage, fs: f32) -> Element<'_, Message> {
    let mut col = column![
        text(msg.title.clone()).size(18).color(p.text).font(Font {
            weight: iced::font::Weight::Bold,
            ..Font::DEFAULT
        }),
        text(format!("{}  ·  {}  ·  {}", msg.author, msg.date, msg.short))
            .size(11)
            .color(p.muted),
    ]
    .spacing(2)
    .width(Fill);

    if !msg.body.is_empty() {
        col = col.push(Space::new().height(Length::Fixed(8.0)));
        col = col.push(text(msg.body.clone()).font(MONO).size(fs).color(p.text));
    }

    container(col)
        .padding(12)
        .width(Fill)
        .style(move |_t| container::Style {
            background: Some(Background::Color(p.panel)),
            border: Border {
                color: p.border,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn file_icon(path: &str) -> String {
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
    .to_string()
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

fn theme(state: &App) -> Theme {
    if state.pal.dark {
        Theme::Dark
    } else {
        Theme::Light
    }
}

fn main() -> iced::Result {
    let path = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("GIT_REVIEW_REPO").ok())
        .unwrap_or_else(|| ".".to_string());

    if let Err(e) = Repo::open(&path) {
        eprintln!("failed to open git repository at '{path}': {e}");
        std::process::exit(1);
    }

    let boot = move || {
        let repo = Repo::open(&path).expect("repository (validated)");
        App::new(repo)
    };

    iced::application(boot, App::update, App::view)
        .title("git-review · iced")
        .theme(theme)
        .window_size(iced::Size::new(1280.0, 800.0))
        .run()
}
