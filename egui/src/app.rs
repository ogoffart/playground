//! The egui UI for git-review.

use eframe::egui;
use egui::{
    text::LayoutJob, Align, Align2, Color32, FontId, Id, Pos2, Rect, Sense, Stroke, TextFormat,
    Vec2,
};
use git2::Oid;

use crate::git::{ChangeKind, CommitInfo, DiffSet, FileDiff, LineKind, Repo};
use crate::highlight::Highlighter;

// --- GitHub-ish palettes (light + dark), selected to match the OS colour scheme ---------------
#[derive(Clone, Copy)]
struct Palette {
    dark: bool,
    bg: Color32,
    panel: Color32,
    border: Color32,
    text: Color32,
    muted: Color32,
    accent: Color32,
    sel: Color32,
    add_bg: Color32,
    add_mark: Color32,
    del_bg: Color32,
    del_mark: Color32,
    add_fg: Color32,
    del_fg: Color32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

impl Palette {
    const LIGHT: Palette = Palette {
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
        del_bg: rgb(0xff, 0xeb, 0xe9),
        del_mark: rgb(0xff, 0x81, 0x82),
        add_fg: rgb(0x1a, 0x7f, 0x37),
        del_fg: rgb(0xcf, 0x22, 0x2e),
    };

    const DARK: Palette = Palette {
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
        del_bg: rgb(0x25, 0x17, 0x1c),
        del_mark: rgb(0xf8, 0x51, 0x49),
        add_fg: rgb(0x3f, 0xb9, 0x50),
        del_fg: rgb(0xf8, 0x51, 0x49),
    };
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

/// A node in the hierarchical file tree built from `FileDiff.path`.
enum TreeNode {
    /// A folder: a (possibly collapsed `a/b/c`) directory label and its children.
    Dir {
        name: String,
        id: Id,
        children: Vec<TreeNode>,
    },
    /// A file leaf, with the index into `diff.files`.
    File { name: String, file_idx: usize },
}

pub struct App {
    repo: Repo,
    hl: Highlighter,
    repo_name: String,
    commits: Vec<CommitInfo>,
    diff: DiffSet,
    showing: Showing,
    from: Option<Oid>,
    to: Option<Oid>,
    settings: Settings,
    scroll_to_file: Option<usize>,
    error: Option<String>,
    pal: Palette,
    style_dark: Option<bool>,
}

impl App {
    pub fn new(repo: Repo) -> Self {
        let hl = Highlighter::new();
        let repo_name = repo.workdir_name();
        let commits = repo.commits(500).unwrap_or_default();
        // Default to showing the most recent commit, else the working tree.
        let showing = commits
            .iter()
            .find_map(|c| c.oid.map(Showing::Commit))
            .unwrap_or(Showing::Working);
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
            scroll_to_file: None,
            error: None,
            pal: Palette::LIGHT,
            style_dark: None,
        };
        app.recompute();
        app
    }

    /// Follow the OS colour scheme: `ctx.system_theme()` is `Some` when the desktop reports a
    /// preference, and `None` headless — in which case we default to light (per the spec).
    fn sync_theme(&mut self, ctx: &egui::Context) {
        let dark = matches!(ctx.system_theme(), Some(egui::Theme::Dark));
        self.pal = if dark { Palette::DARK } else { Palette::LIGHT };
        self.hl.set_dark(dark);
        if self.style_dark != Some(dark) {
            install_style(ctx, &self.pal);
            self.style_dark = Some(dark);
        }
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
    }

    fn select(&mut self, showing: Showing) {
        if self.showing != showing {
            self.showing = showing;
            self.recompute();
        }
    }

    // --- toolbar ------------------------------------------------------------
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal;
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            // left: summary
            ui.label(
                egui::RichText::new(&self.diff.summary)
                    .monospace()
                    .strong()
                    .color(pal.text),
            );
            ui.label(egui::RichText::new(format!("+{}", self.diff.added)).color(pal.add_fg));
            ui.label(egui::RichText::new(format!("−{}", self.diff.removed)).color(pal.del_fg));

            // right: tool buttons
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                let mut changed = false;
                if tool(ui, pal, "#", "Show line numbers", self.settings.line_numbers).clicked() {
                    self.settings.line_numbers = !self.settings.line_numbers;
                }
                if tool(ui, pal, "A+", "Increase font size", false).clicked() {
                    self.settings.font_size = (self.settings.font_size + 1.0).min(28.0);
                }
                if tool(ui, pal, "A-", "Decrease font size", false).clicked() {
                    self.settings.font_size = (self.settings.font_size - 1.0).max(8.0);
                }
                if tool(ui, pal, "␣", "Show space changes", self.settings.show_space).clicked() {
                    self.settings.show_space = !self.settings.show_space;
                    changed = true;
                }
                if tool(ui, pal, "⤶", "Word wrap", self.settings.word_wrap).clicked() {
                    self.settings.word_wrap = !self.settings.word_wrap;
                }
                if changed {
                    self.recompute();
                }
            });
        });
    }

    // --- commit list --------------------------------------------------------
    fn commit_list(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(format!("COMMITS · {}", self.repo_name))
                    .small()
                    .strong()
                    .color(pal.muted),
            );
        });
        egui::ScrollArea::vertical()
            .id_salt("commits")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let commits = self.commits.clone();
                for c in &commits {
                    self.commit_row(ui, c);
                }
            });
    }

    fn commit_row(&mut self, ui: &mut egui::Ui, c: &CommitInfo) {
        let pal = self.pal;
        let is_current = match (self.showing, c.oid) {
            (Showing::Working, _) if c.is_working_tree() => true,
            (Showing::Commit(o), Some(oid)) => o == oid,
            _ => false,
        };
        let is_from = c.oid.is_some() && c.oid == self.from;
        let is_to = c.oid.is_some() && c.oid == self.to;

        let frame = egui::Frame::new()
            .fill(if is_current {
                pal.sel
            } else {
                Color32::TRANSPARENT
            })
            .inner_margin(egui::Margin::symmetric(8, 5));
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            // line 1: endpoint buttons + sha + date + author
            ui.horizontal(|ui| {
                if let Some(oid) = c.oid {
                    if endpoint(ui, pal, "◀", "Compare from this commit", is_from).clicked() {
                        self.from = Some(oid);
                        self.maybe_range();
                    }
                    if endpoint(ui, pal, "▶", "Compare to this commit", is_to).clicked() {
                        self.to = Some(oid);
                        self.maybe_range();
                    }
                } else {
                    ui.add_space(38.0);
                }
                ui.label(
                    egui::RichText::new(&c.short)
                        .monospace()
                        .color(pal.accent)
                        .size(11.0),
                );
                ui.label(egui::RichText::new(&c.date).small().color(pal.muted));
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(&c.author).small().color(pal.muted))
                            .truncate(),
                    );
                });
            });
            // line 2: title (clickable)
            let title = ui.add(
                egui::Label::new(egui::RichText::new(&c.title).color(pal.text))
                    .truncate()
                    .sense(Sense::click()),
            );
            if title.clicked() {
                match c.oid {
                    Some(oid) => self.select(Showing::Commit(oid)),
                    None => self.select(Showing::Working),
                }
            }
        });
        ui.separator();
    }

    fn maybe_range(&mut self) {
        if let (Some(a), Some(b)) = (self.from, self.to) {
            self.select(Showing::Range(a, b));
        }
    }

    // --- file tree ----------------------------------------------------------
    fn file_tree(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(format!("FILES ({})", self.diff.files.len()))
                    .small()
                    .strong()
                    .color(pal.muted),
            );
        });
        let roots = build_tree(&self.diff.files);
        egui::ScrollArea::vertical()
            .id_salt("files")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                for node in &roots {
                    self.tree_node(ui, node, 0);
                }
            });
    }

    /// Render one tree node (folder or file leaf) at the given indentation depth.
    fn tree_node(&mut self, ui: &mut egui::Ui, node: &TreeNode, depth: usize) {
        let pal = self.pal;
        let indent = 8.0 + depth as f32 * 14.0;
        match node {
            TreeNode::Dir { name, id, children } => {
                // Collapsing state, expanded by default.
                let mut open = ui
                    .data_mut(|d| d.get_temp::<bool>(*id))
                    .unwrap_or(true);
                let resp = ui
                    .horizontal(|ui| {
                        ui.add_space(indent);
                        ui.label(
                            egui::RichText::new(if open { "▾" } else { "▸" })
                                .size(11.0)
                                .color(pal.muted),
                        );
                        ui.label(egui::RichText::new("📁").size(13.0));
                        ui.label(
                            egui::RichText::new(format!("{name}/"))
                                .color(pal.text)
                                .size(12.0)
                                .strong(),
                        );
                    })
                    .response
                    .interact(Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(resp.rect, 0.0, hover_tint(pal));
                }
                if resp.clicked() {
                    open = !open;
                    ui.data_mut(|d| d.insert_temp(*id, open));
                }
                if open {
                    for child in children {
                        self.tree_node(ui, child, depth + 1);
                    }
                }
            }
            TreeNode::File { name, file_idx } => {
                let f = &self.diff.files[*file_idx];
                let added = f.added;
                let removed = f.removed;
                let resp = ui
                    .horizontal(|ui| {
                        ui.add_space(indent + 14.0); // align past the disclosure column
                        ui.label(file_icon(name));
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(name).color(pal.text).size(12.0),
                            )
                            .truncate(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(format!("−{removed}"))
                                    .small()
                                    .color(pal.del_fg),
                            );
                            ui.label(
                                egui::RichText::new(format!("+{added}"))
                                    .small()
                                    .color(pal.add_fg),
                            );
                        });
                    })
                    .response
                    .interact(Sense::click());
                if resp.clicked() {
                    self.scroll_to_file = Some(*file_idx);
                }
                if resp.hovered() {
                    ui.painter().rect_filled(resp.rect, 0.0, hover_tint(pal));
                }
            }
        }
    }

    // --- diff view ----------------------------------------------------------
    fn diff_view(&mut self, ui: &mut egui::Ui) {
        if let Some(err) = &self.error {
            ui.colored_label(self.pal.del_fg, format!("Error: {err}"));
            return;
        }

        let wrap = self.settings.word_wrap;
        let scroll = if wrap {
            egui::ScrollArea::vertical()
        } else {
            egui::ScrollArea::both()
        };

        let out = scroll.auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut header_tops: Vec<(usize, f32)> = Vec::new();

            // commit message (single-commit view only)
            if let Some(msg) = &self.diff.message {
                commit_message_ui(ui, self.pal, msg);
            }

            let files = self.diff.files.clone();
            for (i, f) in files.iter().enumerate() {
                let header_rect = self.file_header(ui, f, false);
                header_tops.push((i, header_rect.top()));
                if self.scroll_to_file == Some(i) {
                    ui.scroll_to_rect(header_rect, Some(Align::TOP));
                }
                self.file_body(ui, f);
                ui.add_space(10.0);
            }
            header_tops
        });
        self.scroll_to_file = None;

        // sticky header overlay for the topmost visible file
        let header_tops = out.inner;
        let viewport_top = out.inner_rect.top();
        if let Some((idx, _)) = header_tops
            .iter()
            .rev()
            .find(|(_, top)| *top <= viewport_top + 0.5)
            .copied()
        {
            if let Some(f) = self.diff.files.get(idx).cloned() {
                let area = egui::Area::new(Id::new("sticky-header"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(Pos2::new(out.inner_rect.left(), viewport_top));
                area.show(ui.ctx(), |ui| {
                    ui.set_width(out.inner_rect.width());
                    self.file_header(ui, &f, true);
                });
            }
        }
    }

    /// Draw a file header bar; returns its rect. `sticky` tweaks the shadow/border.
    fn file_header(&self, ui: &mut egui::Ui, f: &FileDiff, sticky: bool) -> Rect {
        let pal = self.pal;
        let frame = egui::Frame::new()
            .fill(pal.panel)
            .stroke(Stroke::new(1.0, pal.border))
            .inner_margin(egui::Margin::symmetric(10, 6));
        let resp = frame
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(file_icon(&f.path));
                    let label = match (&f.old_path, f.kind) {
                        (Some(old), ChangeKind::Renamed) => format!("{old}  →  {}", f.path),
                        _ => f.path.clone(),
                    };
                    ui.label(
                        egui::RichText::new(label)
                            .strong()
                            .color(pal.text)
                            .size(12.5),
                    );
                    ui.label(
                        egui::RichText::new(format!("[{}]", f.kind.letter()))
                            .small()
                            .color(pal.muted),
                    );
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("−{}", f.removed)).color(pal.del_fg),
                        );
                        ui.label(
                            egui::RichText::new(format!("+{}", f.added)).color(pal.add_fg),
                        );
                    });
                });
            })
            .response;
        if sticky {
            ui.painter().rect_filled(
                Rect::from_min_size(resp.rect.left_bottom(), Vec2::new(resp.rect.width(), 4.0)),
                0.0,
                Color32::from_black_alpha(20),
            );
        }
        resp.rect
    }

    fn file_body(&self, ui: &mut egui::Ui, f: &FileDiff) {
        if f.binary {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new("Binary file not shown")
                        .italics()
                        .color(self.pal.muted),
                );
            });
            return;
        }
        let syntax = self.hl.syntax_for(&f.path);
        for hunk in &f.hunks {
            self.draw_line(ui, LineKind::Context, None, None, &hunk.header, None, true);
            for line in &hunk.lines {
                self.draw_line(
                    ui,
                    line.kind,
                    line.old_no,
                    line.new_no,
                    &line.text,
                    Some(syntax),
                    false,
                );
            }
        }
    }

    /// Draw one diff row with a full-width tinted background, a line-number/sign gutter, and
    /// syntax-highlighted code. `hunk_header` rows are rendered in the accent colour.
    #[allow(clippy::too_many_arguments)]
    fn draw_line(
        &self,
        ui: &mut egui::Ui,
        kind: LineKind,
        old_no: Option<u32>,
        new_no: Option<u32>,
        text: &str,
        syntax: Option<&syntect::parsing::SyntaxReference>,
        hunk_header: bool,
    ) {
        let pal = self.pal;
        let fs = self.settings.font_size;
        let char_w = fs * 0.6;
        let num_w = char_w * 4.0;
        let sign_w = char_w * 2.0;
        let gutter = if self.settings.line_numbers {
            num_w * 2.0 + sign_w
        } else {
            sign_w
        };
        let code_x_pad = 6.0;

        let (bg, mark, sign) = match kind {
            LineKind::Added => (pal.add_bg, pal.add_mark, '+'),
            LineKind::Removed => (pal.del_bg, pal.del_mark, '-'),
            LineKind::Context => (Color32::TRANSPARENT, Color32::TRANSPARENT, ' '),
        };

        // Build the code galley.
        let avail = ui.available_width();
        let wrap_width = if self.settings.word_wrap {
            (avail - gutter - code_x_pad).max(40.0)
        } else {
            f32::INFINITY
        };
        let job = if hunk_header {
            simple_job(text, fs, pal.muted, wrap_width)
        } else if let Some(syntax) = syntax {
            let spans = self.hl.line(syntax, text);
            spans_job(&spans, fs, pal.text, wrap_width)
        } else {
            simple_job(text, fs, pal.text, wrap_width)
        };
        let galley = ui.painter().layout_job(job);
        let row_h = galley.size().y.max(fs * 1.35);

        let needed = gutter + code_x_pad + galley.size().x + 8.0;
        let width = if self.settings.word_wrap {
            avail
        } else {
            needed.max(avail)
        };

        let (rect, _resp) = ui.allocate_exact_size(Vec2::new(width, row_h), Sense::hover());
        let p = ui.painter_at(rect);

        if hunk_header {
            p.rect_filled(rect, 0.0, pal.sel);
        } else if bg != Color32::TRANSPARENT {
            p.rect_filled(rect, 0.0, bg);
            // stronger marker strip on the sign column
            let mark_rect = Rect::from_min_size(
                Pos2::new(rect.left() + gutter - sign_w, rect.top()),
                Vec2::new(sign_w, rect.height()),
            );
            p.rect_filled(mark_rect, 0.0, mark);
        }

        let top = rect.top();
        let mono = FontId::monospace(fs);
        if self.settings.line_numbers && !hunk_header {
            if let Some(n) = old_no {
                p.text(
                    Pos2::new(rect.left() + num_w - 4.0, top),
                    Align2::RIGHT_TOP,
                    n.to_string(),
                    mono.clone(),
                    pal.muted,
                );
            }
            if let Some(n) = new_no {
                p.text(
                    Pos2::new(rect.left() + num_w * 2.0 - 4.0, top),
                    Align2::RIGHT_TOP,
                    n.to_string(),
                    mono.clone(),
                    pal.muted,
                );
            }
        }
        if !hunk_header && sign != ' ' {
            p.text(
                Pos2::new(rect.left() + gutter - sign_w + 4.0, top),
                Align2::LEFT_TOP,
                sign.to_string(),
                mono,
                if kind == LineKind::Added {
                    pal.add_fg
                } else {
                    pal.del_fg
                },
            );
        }
        p.galley(
            Pos2::new(rect.left() + gutter + code_x_pad, top),
            galley,
            pal.text,
        );
    }
}

impl eframe::App for App {
    // eframe 0.34 makes `ui` the required method: the whole app is driven from a root `Ui`, and
    // panels are carved out of it with `show_inside`.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.sync_theme(ui.ctx());
        let pal = self.pal;

        // The toolbar is added FIRST at the top level, so it spans the FULL window width above
        // both the side panel and the diff. The left panel is carved out of the remaining area.
        egui::Panel::top("toolbar")
            .frame(
                egui::Frame::new()
                    .fill(pal.panel)
                    .stroke(Stroke::new(1.0, pal.border))
                    .inner_margin(egui::Margin::symmetric(6, 6)),
            )
            .show_inside(ui, |ui| self.toolbar(ui));

        egui::Panel::left("side")
            .resizable(true)
            .default_size(320.0)
            .size_range(220.0..=600.0)
            .frame(egui::Frame::new().fill(pal.panel))
            .show_inside(ui, |ui| {
                egui::Panel::top("commits_panel")
                    .resizable(true)
                    .default_size(360.0)
                    .frame(egui::Frame::new().fill(pal.panel))
                    .show_inside(ui, |ui| self.commit_list(ui));
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(pal.panel))
                    .show_inside(ui, |ui| self.file_tree(ui));
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(pal.bg))
            .show_inside(ui, |ui| self.diff_view(ui));
    }
}

// --- file tree construction -------------------------------------------------

/// Build a hierarchical tree from the flat list of file paths. Directory components become
/// folder nodes; a single-child directory chain is collapsed GitHub-style into one node
/// (`a/b/c`); distinct subtrees branch.
fn build_tree(files: &[FileDiff]) -> Vec<TreeNode> {
    // Intermediate mutable tree keyed by component name, preserving insertion order.
    #[derive(Default)]
    struct Builder {
        dirs: Vec<(String, Builder)>,
        files: Vec<(String, usize)>,
    }
    impl Builder {
        fn child(&mut self, name: &str) -> &mut Builder {
            if let Some(pos) = self.dirs.iter().position(|(n, _)| n == name) {
                &mut self.dirs[pos].1
            } else {
                self.dirs.push((name.to_string(), Builder::default()));
                &mut self.dirs.last_mut().unwrap().1
            }
        }
    }

    let mut root = Builder::default();
    for (idx, f) in files.iter().enumerate() {
        let parts: Vec<&str> = f.path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            continue;
        }
        let mut cur = &mut root;
        for comp in &parts[..parts.len() - 1] {
            cur = cur.child(comp);
        }
        cur.files.push((parts[parts.len() - 1].to_string(), idx));
    }

    fn convert(name_prefix: &str, b: Builder, path: &str) -> Vec<TreeNode> {
        let mut out = Vec::new();
        let _ = name_prefix;
        for (name, mut sub) in b.dirs {
            // Collapse single-child dir chains: a dir with exactly one child dir and no files
            // folds into "name/childname".
            let mut label = name;
            let mut full = format!("{path}/{label}");
            while sub.files.is_empty() && sub.dirs.len() == 1 {
                let (cn, cb) = sub.dirs.pop().unwrap();
                label = format!("{label}/{cn}");
                full = format!("{path}/{label}");
                sub = cb;
            }
            let children = convert(&label, sub, &full);
            out.push(TreeNode::Dir {
                name: label,
                id: Id::new(("tree-dir", full)),
                children,
            });
        }
        for (name, idx) in b.files {
            out.push(TreeNode::File {
                name,
                file_idx: idx,
            });
        }
        out
    }

    convert("", root, "")
}

// --- small widgets ----------------------------------------------------------

fn hover_tint(pal: Palette) -> Color32 {
    if pal.dark {
        Color32::from_rgba_unmultiplied(0x2f, 0x81, 0xf7, 28)
    } else {
        Color32::from_rgba_unmultiplied(9, 105, 218, 18)
    }
}

fn tool(ui: &mut egui::Ui, pal: Palette, label: &str, tip: &str, active: bool) -> egui::Response {
    let mut text = egui::RichText::new(label).monospace();
    if active {
        text = text.color(pal.accent).strong();
    } else {
        text = text.color(pal.text);
    }
    let fill = if active {
        pal.sel
    } else if pal.dark {
        Color32::from_rgb(0x21, 0x26, 0x2d)
    } else {
        Color32::from_rgb(0xf6, 0xf8, 0xfa)
    };
    ui.add(
        egui::Button::new(text)
            .fill(fill)
            .stroke(Stroke::new(1.0, pal.border)),
    )
    .on_hover_text(tip)
}

fn endpoint(ui: &mut egui::Ui, pal: Palette, label: &str, tip: &str, active: bool) -> egui::Response {
    let inactive = if pal.dark {
        Color32::from_gray(60)
    } else {
        Color32::from_gray(230)
    };
    ui.add(
        egui::Button::new(egui::RichText::new(label).size(10.0))
            .small()
            .fill(if active { pal.accent } else { inactive }),
    )
    .on_hover_text(tip)
}

fn commit_message_ui(ui: &mut egui::Ui, pal: Palette, msg: &crate::git::CommitMessage) {
    egui::Frame::new()
        .fill(pal.panel)
        .stroke(Stroke::new(1.0, pal.border))
        .inner_margin(12)
        .outer_margin(egui::Margin {
            left: 0,
            right: 0,
            top: 0,
            bottom: 10,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(&msg.title).heading().color(pal.text));
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(format!("{}  ·  {}  ·  {}", msg.author, msg.date, msg.short))
                    .small()
                    .color(pal.muted),
            );
            if !msg.body.is_empty() {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(&msg.body).monospace().color(pal.text));
            }
        });
}

fn spans_job(spans: &[crate::highlight::Span], fs: f32, fallback: Color32, wrap_width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    if spans.is_empty() {
        job.append(
            " ",
            0.0,
            TextFormat {
                font_id: FontId::monospace(fs),
                color: fallback,
                ..Default::default()
            },
        );
        return job;
    }
    for s in spans {
        job.append(&s.text, 0.0, fmt(s.color, fs, s.bold, s.italic));
    }
    job
}

fn simple_job(text: &str, fs: f32, color: Color32, wrap_width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    let t = if text.is_empty() { " " } else { text };
    job.append(
        t,
        0.0,
        TextFormat {
            font_id: FontId::monospace(fs),
            color,
            ..Default::default()
        },
    );
    job
}

fn fmt(rgb: (u8, u8, u8), fs: f32, _bold: bool, italic: bool) -> TextFormat {
    // The default egui monospace font has no bold variant, so colour/italics convey emphasis.
    TextFormat {
        font_id: FontId::monospace(fs),
        color: Color32::from_rgb(rgb.0, rgb.1, rgb.2),
        italics: italic,
        ..Default::default()
    }
}

fn file_icon(path: &str) -> egui::RichText {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let glyph = match ext {
        "rs" => "🦀",
        "py" => "🐍",
        "js" | "ts" | "tsx" | "jsx" => "📜",
        "md" => "📝",
        "toml" | "yaml" | "yml" | "json" | "ini" | "cfg" => "⚙",
        "png" | "jpg" | "jpeg" | "gif" | "svg" => "🖼",
        "sh" | "bash" => "💲",
        "html" | "css" => "🌐",
        _ => "📄",
    };
    egui::RichText::new(glyph).size(13.0)
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

fn install_style(ctx: &egui::Context, pal: &Palette) {
    use egui::FontFamily::Proportional;
    use egui::TextStyle::*;
    let mut style = (*ctx.global_style()).clone();
    style.visuals = if pal.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    style.visuals.panel_fill = pal.bg;
    style.visuals.window_fill = pal.bg;
    style.visuals.override_text_color = Some(pal.text);
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, pal.border);
    style.text_styles.insert(Heading, FontId::new(18.0, Proportional));
    style.spacing.scroll = egui::style::ScrollStyle::solid();
    ctx.set_global_style(style);
}
