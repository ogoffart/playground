//! The iced UI for git-review.
//!
//! Elm architecture: `App` state, a `Message` enum, `update`, and `view`. The data layer
//! (`git`, `highlight`) is shared verbatim with the other framework apps in this repo.

mod git;
mod highlight;

use std::collections::HashMap;

use iced::widget::pane_grid::{self, PaneGrid};
use iced::widget::operation::{self, AbsoluteOffset};
use iced::widget::{
    button, column, container, mouse_area, rich_text, row, scrollable, span, text,
    tooltip, Column, Id, Space,
};
use iced::{
    Background, Border, Color, Element, Fill, Font, Length, Padding, Shrink, Task, Theme,
};
use git2::Oid;

use git::{ChangeKind, CommitInfo, CommitMessage, DiffSet, FileDiff, LineKind, Repo};
use highlight::Highlighter;

// --- GitHub-ish palette -----------------------------------------------------
const BG: Color = Color::from_rgb(1.0, 1.0, 1.0);
fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}
fn panel() -> Color {
    rgb(0xf6, 0xf8, 0xfa)
}
fn border() -> Color {
    rgb(0xd0, 0xd7, 0xde)
}
fn text_c() -> Color {
    rgb(0x1f, 0x23, 0x28)
}
fn muted() -> Color {
    rgb(0x65, 0x6d, 0x76)
}
fn accent() -> Color {
    rgb(0x09, 0x69, 0xda)
}
fn sel() -> Color {
    rgb(0xdd, 0xf4, 0xff)
}
fn add_bg() -> Color {
    rgb(0xe6, 0xff, 0xec)
}
fn add_mark() -> Color {
    rgb(0xab, 0xf2, 0xbc)
}
fn del_bg() -> Color {
    rgb(0xff, 0xeb, 0xe9)
}
fn del_mark() -> Color {
    rgb(0xff, 0x81, 0x82)
}
fn add_fg() -> Color {
    rgb(0x1a, 0x7f, 0x37)
}
fn del_fg() -> Color {
    rgb(0xcf, 0x22, 0x2e)
}

const MONO: Font = Font::MONOSPACE;

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
    repo_name: String,
    commits: Vec<CommitInfo>,
    diff: DiffSet,
    showing: Showing,
    from: Option<Oid>,
    to: Option<Oid>,
    settings: Settings,
    error: Option<String>,

    panes: pane_grid::State<PaneKind>,
    diff_scroll: Id,
    /// Per-file vertical offset within the diff scroll content (px from top).
    file_tops: HashMap<usize, f32>,
    /// Heights of each file section (header + body), for sticky-header detection.
    /// Recomputed lazily from layout estimates.
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
        let hl = Highlighter::new();
        let repo_name = repo.workdir_name();
        let commits = repo.commits(500).unwrap_or_default();
        let showing = commits
            .iter()
            .find_map(|c| c.oid.map(Showing::Commit))
            .unwrap_or(Showing::Working);

        // Layout: horizontal split -> (left vertical split: commits/files) | main.
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
            repo_name,
            commits,
            diff: empty_diff(),
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
    /// iced lays widgets out itself, so we mirror the layout math used in `view`
    /// to know where to scroll and which header is currently topmost (sticky).
    fn compute_file_tops(&mut self) {
        self.file_tops.clear();
        let row_h = self.settings.font_size * ROW_LINE_HEIGHT;
        let header_h = 30.0;
        let file_gap = 12.0;
        let mut y = 0.0f32;
        if let Some(msg) = &self.diff.message {
            // commit message block: title + meta + body lines + paddings + bottom gap
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
        .style(|_theme| pane_grid::Style {
            hovered_region: pane_grid::Highlight {
                background: Background::Color(sel()),
                border: Border::default(),
            },
            picked_split: pane_grid::Line {
                color: accent(),
                width: 2.0,
            },
            hovered_split: pane_grid::Line {
                color: accent(),
                width: 2.0,
            },
        });

        container(grid)
            .width(Fill)
            .height(Fill)
            .style(|_t| container::Style {
                background: Some(Background::Color(border())),
                ..Default::default()
            })
            .into()
    }

    // --- commit list --------------------------------------------------------
    fn commit_list(&self) -> Element<'_, Message> {
        let header = section_header(&format!("COMMITS · {}", self.repo_name));

        let mut list = Column::new().width(Fill);
        for (idx, c) in self.commits.iter().enumerate() {
            list = list.push(self.commit_row(idx, c));
            list = list.push(thin_rule());
        }

        let scroll = scrollable(list).width(Fill).height(Fill);

        container(column![header, scroll].spacing(0))
            .width(Fill)
            .height(Fill)
            .style(panel_style)
            .into()
    }

    fn commit_row<'a>(&'a self, _idx: usize, c: &'a CommitInfo) -> Element<'a, Message> {
        let is_current = match (self.showing, c.oid) {
            (Showing::Working, _) if c.is_working_tree() => true,
            (Showing::Commit(o), Some(oid)) => o == oid,
            _ => false,
        };
        let is_from = c.oid.is_some() && c.oid == self.from;
        let is_to = c.oid.is_some() && c.oid == self.to;

        // line 1: endpoint buttons + sha + date + author
        let mut line1 = row![].spacing(4).align_y(iced::Alignment::Center);
        if let Some(oid) = c.oid {
            line1 = line1
                .push(endpoint_btn("◀", "Compare from this commit", is_from, Message::SetFrom(oid)))
                .push(endpoint_btn("▶", "Compare to this commit", is_to, Message::SetTo(oid)));
        } else {
            line1 = line1.push(Space::new().width(Length::Fixed(40.0)));
        }
        line1 = line1
            .push(text(c.short.clone()).font(MONO).size(11).color(accent()))
            .push(text(c.date.clone()).size(10).color(muted()))
            .push(Space::new().width(Fill))
            .push(
                text(c.author.clone())
                    .size(10)
                    .color(muted())
                    .wrapping(text::Wrapping::None),
            );

        let line2 = text(c.title.clone())
            .size(12)
            .color(text_c())
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
                background: Some(Background::Color(if is_current {
                    sel()
                } else {
                    panel()
                })),
                ..Default::default()
            });

        mouse_area(cell)
            .on_press(msg)
            .interaction(iced::mouse::Interaction::Pointer)
            .into()
    }

    // --- file tree ----------------------------------------------------------
    fn file_tree(&self) -> Element<'_, Message> {
        let header = section_header(&format!("FILES ({})", self.diff.files.len()));

        let mut list = Column::new().width(Fill);
        for (i, f) in self.diff.files.iter().enumerate() {
            let line = row![
                text(file_icon(&f.path)).size(13),
                text(f.path.clone())
                    .size(12)
                    .color(text_c())
                    .wrapping(text::Wrapping::None),
                Space::new().width(Fill),
                text(format!("+{}", f.added)).size(10).color(add_fg()),
                text(format!("−{}", f.removed)).size(10).color(del_fg()),
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center);

            let cell = container(line)
                .padding(Padding::from([3.0, 8.0]))
                .width(Fill)
                .style(|_t| container::Style {
                    ..Default::default()
                });

            list = list.push(
                mouse_area(cell)
                    .on_press(Message::OpenFile(i))
                    .interaction(iced::mouse::Interaction::Pointer),
            );
        }

        let scroll = scrollable(list).width(Fill).height(Fill);

        container(column![header, scroll].spacing(0))
            .width(Fill)
            .height(Fill)
            .style(panel_style)
            .into()
    }

    // --- main view (toolbar + diff) ----------------------------------------
    fn main_view(&self) -> Element<'_, Message> {
        let toolbar = self.toolbar();
        let diff = self.diff_view();
        container(column![toolbar, diff].spacing(0))
            .width(Fill)
            .height(Fill)
            .style(|_t| container::Style {
                background: Some(Background::Color(BG)),
                ..Default::default()
            })
            .into()
    }

    fn toolbar(&self) -> Element<'_, Message> {
        let summary = row![
            text(self.diff.summary.clone())
                .font(Font {
                    weight: iced::font::Weight::Bold,
                    ..MONO
                })
                .size(13)
                .color(text_c()),
            text(format!("+{}", self.diff.added)).size(13).color(add_fg()),
            text(format!("−{}", self.diff.removed)).size(13).color(del_fg()),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

        let buttons = row![
            tool_btn("⤶", "Word wrap", self.settings.word_wrap, Message::ToggleWordWrap),
            tool_btn("␣", "Show space changes", self.settings.show_space, Message::ToggleShowSpace),
            tool_btn("A-", "Decrease font size", false, Message::FontDec),
            tool_btn("A+", "Increase font size", false, Message::FontInc),
            tool_btn("#", "Show line numbers", self.settings.line_numbers, Message::ToggleLineNumbers),
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center);

        let bar = row![summary, Space::new().width(Fill), buttons]
            .spacing(8)
            .align_y(iced::Alignment::Center)
            .width(Fill);

        container(bar)
            .padding(Padding::from([6.0, 10.0]))
            .width(Fill)
            .style(|_t| container::Style {
                background: Some(Background::Color(BG)),
                border: Border {
                    color: border(),
                    width: 1.0,
                    radius: 0.0.into(),
                },
                ..Default::default()
            })
            .into()
    }

    fn diff_view(&self) -> Element<'_, Message> {
        if let Some(err) = &self.error {
            return container(text(format!("Error: {err}")).color(del_fg()))
                .padding(12)
                .into();
        }

        let mut body = Column::new().width(Fill).spacing(0);

        if let Some(msg) = &self.diff.message {
            body = body.push(commit_message_ui(msg, self.settings.font_size));
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

        let scroll = scrollable(container(body).padding(Padding::from([0.0, 0.0])))
            .direction(direction)
            .id(self.diff_scroll.clone())
            .on_scroll(Message::DiffScrolled)
            .width(Fill)
            .height(Fill);

        // Sticky header overlay: the topmost file whose section start is above the
        // current scroll offset gets its header pinned at the top of the viewport.
        let sticky = self.sticky_header();

        let stacked: Element<Message> = if let Some(h) = sticky {
            iced::widget::stack![scroll, h].into()
        } else {
            scroll.into()
        };

        container(stacked).width(Fill).height(Fill).into()
    }

    /// Which file's header should be pinned, given the current scroll offset.
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
        // Pin the header at the top of the viewport.
        Some(
            container(self.file_header_inner(f, true))
                .width(Fill)
                .into(),
        )
    }

    fn file_header(&self, f: &FileDiff) -> Element<'_, Message> {
        self.file_header_inner(f, false)
    }

    fn file_header_inner(&self, f: &FileDiff, sticky: bool) -> Element<'_, Message> {
        let label = match (&f.old_path, f.kind) {
            (Some(old), ChangeKind::Renamed) => format!("{old}  →  {}", f.path),
            _ => f.path.clone(),
        };
        let bar = row![
            text(file_icon(&f.path)).size(13),
            text(label).font(Font {
                weight: iced::font::Weight::Semibold,
                ..Font::DEFAULT
            }).size(13).color(text_c()),
            text(format!("[{}]", f.kind.letter())).size(10).color(muted()),
            Space::new().width(Fill),
            text(format!("+{}", f.added)).size(12).color(add_fg()),
            text(format!("−{}", f.removed)).size(12).color(del_fg()),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center)
        .width(Fill);

        container(bar)
            .padding(Padding::from([6.0, 10.0]))
            .width(Fill)
            .style(move |_t| container::Style {
                background: Some(Background::Color(panel())),
                border: Border {
                    color: border(),
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
        if f.binary {
            return container(text("Binary file not shown").size(12).color(muted()))
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
        // In word-wrap mode the rows must fill width; otherwise let them be as wide
        // as the content so horizontal scroll works.
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
        let fs = self.settings.font_size;
        let char_w = fs * 0.62;
        let num_w = char_w * 4.0;

        let (bg, mark, sign, sign_color) = match kind {
            LineKind::Added => (Some(add_bg()), Some(add_mark()), '+', add_fg()),
            LineKind::Removed => (Some(del_bg()), Some(del_mark()), '-', del_fg()),
            LineKind::Context => (None, None, ' ', muted()),
        };

        let mut left = row![].align_y(iced::Alignment::Center);

        if self.settings.line_numbers && !hunk_header {
            let old_s = old_no.map(|n| n.to_string()).unwrap_or_default();
            let new_s = new_no.map(|n| n.to_string()).unwrap_or_default();
            left = left.push(
                container(text(old_s).font(MONO).size(fs).color(muted()))
                    .width(Length::Fixed(num_w))
                    .align_x(iced::Alignment::End),
            );
            left = left.push(
                container(text(new_s).font(MONO).size(fs).color(muted()))
                    .width(Length::Fixed(num_w))
                    .align_x(iced::Alignment::End),
            );
        }

        // sign marker column
        let sign_cell = container(
            text(sign.to_string())
                .font(MONO)
                .size(fs)
                .color(sign_color),
        )
        .width(Length::Fixed(char_w * 1.6))
        .align_x(iced::Alignment::Center)
        .style(move |_t| container::Style {
            background: mark.map(Background::Color),
            ..Default::default()
        });
        if !hunk_header {
            left = left.push(sign_cell);
        }

        // code
        let code: Element<Message> = if hunk_header {
            text(line_text.to_string())
                .font(MONO)
                .size(fs)
                .color(accent())
                .into()
        } else if let Some(syntax) = syntax {
            let spans = self.hl.line(syntax, line_text);
            if spans.is_empty() {
                text(" ").font(MONO).size(fs).into()
            } else {
                let spans: Vec<text::Span<'_, ()>> = spans
                    .into_iter()
                    .map(|s| {
                        span(s.text)
                            .font(MONO)
                            .size(fs)
                            .color(rgb(s.color.0, s.color.1, s.color.2))
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
            text(line_text.to_string()).font(MONO).size(fs).into()
        };

        let code_cell = container(code)
            .padding(Padding::from([0.0, 6.0]))
            .width(if self.settings.word_wrap { Fill } else { Shrink });

        let line = row![left, code_cell].align_y(iced::Alignment::Center);

        let row_bg = if hunk_header { Some(sel()) } else { bg };
        let width = if self.settings.word_wrap { Fill } else { Shrink };

        container(line)
            .width(width)
            .style(move |_t| container::Style {
                background: row_bg.map(Background::Color),
                ..Default::default()
            })
            .into()
    }
}

// row line-height multiplier used both for layout and offset estimation.
const ROW_LINE_HEIGHT: f32 = 1.45;

// --- small widgets ----------------------------------------------------------

fn panel_style(_t: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(panel())),
        ..Default::default()
    }
}

fn section_header(label: &str) -> Element<'static, Message> {
    container(
        text(label.to_string())
            .size(11)
            .color(muted())
            .font(Font {
                weight: iced::font::Weight::Bold,
                ..Font::DEFAULT
            }),
    )
    .padding(Padding::from([6.0, 8.0]))
    .width(Fill)
    .style(panel_style)
    .into()
}

fn thin_rule() -> Element<'static, Message> {
    container(Space::new().height(Length::Fixed(1.0)))
        .width(Fill)
        .style(|_t| container::Style {
            background: Some(Background::Color(border())),
            ..Default::default()
        })
        .into()
}

fn tool_btn(label: &str, tip: &str, active: bool, msg: Message) -> Element<'static, Message> {
    let txt = text(label.to_string())
        .font(MONO)
        .size(13)
        .color(if active { accent() } else { text_c() });

    let b = button(txt)
        .padding(Padding::from([3.0, 7.0]))
        .on_press(msg)
        .style(move |_t, _status| button::Style {
            background: Some(Background::Color(if active {
                sel()
            } else {
                Color::TRANSPARENT
            })),
            text_color: if active { accent() } else { text_c() },
            border: Border {
                color: border(),
                width: 1.0,
                radius: 4.0.into(),
            },
            ..Default::default()
        });

    tooltip(b, container(text(tip.to_string()).size(11).color(BG))
        .padding(Padding::from([3.0, 6.0]))
        .style(|_t| container::Style {
            background: Some(Background::Color(text_c())),
            border: Border { color: border(), width: 0.0, radius: 4.0.into() },
            ..Default::default()
        }), tooltip::Position::Bottom)
        .into()
}

fn endpoint_btn(
    label: &str,
    tip: &str,
    active: bool,
    msg: Message,
) -> Element<'static, Message> {
    let b = button(text(label.to_string()).size(9).color(if active {
        BG
    } else {
        text_c()
    }))
    .padding(Padding::from([1.0, 4.0]))
    .on_press(msg)
    .style(move |_t, _status| button::Style {
        background: Some(Background::Color(if active { accent() } else { rgb(0xe6, 0xe6, 0xe6) })),
        text_color: if active { BG } else { text_c() },
        border: Border { color: border(), width: 0.0, radius: 3.0.into() },
        ..Default::default()
    });
    tooltip(
        b,
        container(text(tip.to_string()).size(11).color(BG))
            .padding(Padding::from([3.0, 6.0]))
            .style(|_t| container::Style {
                background: Some(Background::Color(text_c())),
                border: Border { color: border(), width: 0.0, radius: 4.0.into() },
                ..Default::default()
            }),
        tooltip::Position::Bottom,
    )
    .into()
}

fn commit_message_ui(msg: &CommitMessage, fs: f32) -> Element<'_, Message> {
    let mut col = column![
        text(msg.title.clone()).size(18).color(text_c()).font(Font {
            weight: iced::font::Weight::Bold,
            ..Font::DEFAULT
        }),
        text(format!("{}  ·  {}  ·  {}", msg.author, msg.date, msg.short))
            .size(11)
            .color(muted()),
    ]
    .spacing(2)
    .width(Fill);

    if !msg.body.is_empty() {
        col = col.push(Space::new().height(Length::Fixed(8.0)));
        col = col.push(text(msg.body.clone()).font(MONO).size(fs).color(text_c()));
    }

    container(col)
        .padding(12)
        .width(Fill)
        .style(|_t| container::Style {
            background: Some(Background::Color(panel())),
            border: Border {
                color: border(),
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

fn theme(_state: &App) -> Theme {
    Theme::Light
}

fn main() -> iced::Result {
    let path = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("GIT_REVIEW_REPO").ok())
        .unwrap_or_else(|| ".".to_string());

    // Validate up front so we fail fast with a useful message.
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
        .window_size(iced::Size::new(1280.0, 860.0))
        .run()
}
