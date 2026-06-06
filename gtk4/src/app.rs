//! The GTK4 UI for git-review.
//!
//! Architecture: plain gtk4-rs (no relm4). Shared mutable state lives in an `Rc<RefCell<State>>`;
//! the widget handles we need to mutate live in an `Rc<Ui>`. A single `refresh()` rebuilds the
//! diff view, re-selects the active commit row, and repopulates the file tree whenever the
//! displayed diff or a toolbar setting changes.
//!
//! Layout uses GTK's native `GtkPaned`: an outer horizontal Paned (side panel | main) and an
//! inner vertical Paned in the side panel (commit list | file tree).
//!
//! The diff body is a vertical `gtk::Box` inside a `ScrolledWindow`. Each file contributes a
//! header widget plus a per-file `TextView` carrying the unified diff. Per-line backgrounds use
//! `TextTag::paragraph_background` (full-width green/red), and syntax colours come from the
//! framework-agnostic `Highlighter` applied as foreground `TextTag`s. A floating header inside a
//! `GtkOverlay` approximates the sticky file header.

use std::cell::RefCell;
use std::rc::Rc;

use git2::Oid;
use gtk::prelude::*;
use gtk::{
    gdk, glib, pango, Align, Application, ApplicationWindow, Box as GtkBox, Button,
    CssProvider, Label, ListBox, ListBoxRow, Orientation, Overlay, Paned, PolicyType,
    ScrolledWindow, SelectionMode, TextBuffer, TextTag, TextView, ToggleButton, WrapMode,
};

use crate::git::{ChangeKind, CommitInfo, DiffSet, FileDiff, LineKind, Repo};
use crate::highlight::Highlighter;

// --- GitHub-ish palette (hex strings used for CSS + TextTags) ----------------
const BG: &str = "#ffffff";
const PANEL: &str = "#f6f8fa";
const BORDER: &str = "#d0d7de";
const TEXT: &str = "#1f2328";
const MUTED: &str = "#656d76";
const ACCENT: &str = "#0969da";
const SEL: &str = "#ddf4ff";
const ADD_BG: &str = "#e6ffec";
const ADD_MARK: &str = "#abf2bc";
const DEL_BG: &str = "#ffebe9";
const DEL_MARK: &str = "#ff8182";
const ADD_FG: &str = "#1a7f37";
const DEL_FG: &str = "#cf222e";
const HUNK_BG: &str = "#ddf4ff";

#[derive(Clone, Copy, PartialEq)]
enum Showing {
    Working,
    Commit(Oid),
    Range(Oid, Oid),
}

struct Settings {
    word_wrap: bool,
    show_space: bool,
    font_size: i32,
    line_numbers: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            word_wrap: false,
            show_space: true,
            font_size: 12,
            line_numbers: true,
        }
    }
}

struct State {
    repo: Repo,
    hl: Highlighter,
    commits: Vec<CommitInfo>,
    diff: DiffSet,
    showing: Showing,
    from: Option<Oid>,
    to: Option<Oid>,
    settings: Settings,
    error: Option<String>,
}

impl State {
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
}

/// Widget handles the refresh routine needs to touch.
struct Ui {
    summary: Label,
    counts: Label,
    commit_list: ListBox,
    file_list: ListBox,
    diff_box: GtkBox,
    diff_scroll: ScrolledWindow,
    sticky: GtkBox,
    sticky_label: Label,
    // Per-file top widget (header) within diff_box, used for scroll-to and sticky tracking.
    file_anchors: RefCell<Vec<GtkBox>>,
}

type Shared = Rc<RefCell<State>>;

pub fn load_css() {
    let provider = CssProvider::new();
    provider.load_from_data(&css());
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn css() -> String {
    format!(
        r#"
        window {{ background: {BG}; color: {TEXT}; }}
        .panel {{ background: {PANEL}; }}
        .toolbar {{ background: {BG}; border-bottom: 1px solid {BORDER}; padding: 4px 8px; }}
        .section-title {{ color: {MUTED}; font-size: 10px; font-weight: bold; padding: 6px 8px 2px 8px; }}
        .summary {{ font-family: monospace; font-weight: bold; color: {TEXT}; }}
        .add-fg {{ color: {ADD_FG}; font-weight: bold; }}
        .del-fg {{ color: {DEL_FG}; font-weight: bold; }}
        .muted {{ color: {MUTED}; }}
        .accent {{ color: {ACCENT}; font-family: monospace; }}

        .tool {{ padding: 2px 8px; min-height: 0; font-family: monospace; }}
        .tool:checked {{ background: {SEL}; color: {ACCENT}; }}

        .commit-row {{ padding: 5px 8px; border-bottom: 1px solid {BORDER}; }}
        .commit-row.current {{ background: {SEL}; }}
        .endpoint {{ padding: 0 5px; min-height: 0; min-width: 0; font-size: 10px; }}
        .endpoint:checked {{ background: {ACCENT}; color: white; }}
        .title {{ color: {TEXT}; }}

        .file-row {{ padding: 4px 8px; }}
        .file-row:hover {{ background: {SEL}; }}

        .file-header {{
            background: {PANEL};
            border: 1px solid {BORDER};
            padding: 6px 10px;
        }}
        .file-header .path {{ font-weight: bold; color: {TEXT}; }}
        .sticky {{ background: {PANEL}; border: 1px solid {BORDER}; padding: 6px 10px; }}

        .commit-msg {{
            background: {PANEL};
            border: 1px solid {BORDER};
            padding: 12px;
        }}
        .commit-msg .heading {{ font-size: 16px; font-weight: bold; color: {TEXT}; }}
        .commit-msg .body {{ font-family: monospace; color: {TEXT}; }}

        textview {{ background: {BG}; color: {TEXT}; }}
        textview text {{ background: {BG}; }}
        .binary {{ color: {MUTED}; font-style: italic; padding: 6px 12px; }}
        "#
    )
}

pub fn build_ui(app: &Application, repo: Repo) {
    let hl = Highlighter::new();
    let repo_name = repo.workdir_name();
    let commits = repo.commits(500).unwrap_or_default();
    let showing = commits
        .iter()
        .find_map(|c| c.oid.map(Showing::Commit))
        .unwrap_or(Showing::Working);

    let mut state = State {
        repo,
        hl,
        commits,
        diff: empty_diff(),
        showing,
        from: None,
        to: None,
        settings: Settings::default(),
        error: None,
    };
    state.recompute();
    let shared: Shared = Rc::new(RefCell::new(state));

    // --- Toolbar ------------------------------------------------------------
    let toolbar = GtkBox::new(Orientation::Horizontal, 6);
    toolbar.add_css_class("toolbar");
    let summary = Label::new(None);
    summary.add_css_class("summary");
    let counts = Label::new(None);
    counts.set_use_markup(true);
    toolbar.append(&summary);
    toolbar.append(&counts);

    let spacer = GtkBox::new(Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    toolbar.append(&spacer);

    let wrap_btn = tool_toggle("⤶", "Word wrap", false);
    let space_btn = tool_toggle("␣", "Show space changes", true);
    let font_dec = tool_button("A-", "Decrease font size");
    let font_inc = tool_button("A+", "Increase font size");
    let lines_btn = tool_toggle("#", "Show line numbers", true);
    for w in [wrap_btn.upcast_ref::<gtk::Widget>()] {
        toolbar.append(w);
    }
    toolbar.append(&space_btn);
    toolbar.append(&font_dec);
    toolbar.append(&font_inc);
    toolbar.append(&lines_btn);

    // --- Side panel: commit list + file tree (vertical Paned) ---------------
    let commit_title = Label::new(Some(&format!("COMMITS · {repo_name}")));
    commit_title.set_xalign(0.0);
    commit_title.add_css_class("section-title");
    let commit_list = ListBox::new();
    commit_list.set_selection_mode(SelectionMode::None);
    let commit_scroll = ScrolledWindow::new();
    commit_scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    commit_scroll.set_child(Some(&commit_list));
    commit_scroll.set_vexpand(true);
    let commit_pane = GtkBox::new(Orientation::Vertical, 0);
    commit_pane.add_css_class("panel");
    commit_pane.append(&commit_title);
    commit_pane.append(&commit_scroll);

    let file_title = Label::new(Some("FILES"));
    file_title.set_xalign(0.0);
    file_title.add_css_class("section-title");
    let file_list = ListBox::new();
    file_list.set_selection_mode(SelectionMode::None);
    let file_scroll = ScrolledWindow::new();
    file_scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    file_scroll.set_child(Some(&file_list));
    file_scroll.set_vexpand(true);
    let file_pane = GtkBox::new(Orientation::Vertical, 0);
    file_pane.add_css_class("panel");
    file_pane.append(&file_title);
    file_pane.append(&file_scroll);

    let side_paned = Paned::new(Orientation::Vertical);
    side_paned.set_start_child(Some(&commit_pane));
    side_paned.set_end_child(Some(&file_pane));
    side_paned.set_position(420);
    side_paned.set_resize_start_child(true);
    side_paned.set_resize_end_child(true);
    side_paned.set_wide_handle(true);

    // --- Main view: toolbar + diff scroll (with sticky overlay) -------------
    let diff_box = GtkBox::new(Orientation::Vertical, 0);
    diff_box.set_hexpand(true);
    diff_box.set_vexpand(true);

    let diff_scroll = ScrolledWindow::new();
    diff_scroll.set_policy(PolicyType::Automatic, PolicyType::Automatic);
    diff_scroll.set_child(Some(&diff_box));
    diff_scroll.set_hexpand(true);
    diff_scroll.set_vexpand(true);

    // Sticky header overlay (floats over the top of the scrolled diff).
    let sticky = GtkBox::new(Orientation::Horizontal, 8);
    sticky.add_css_class("sticky");
    sticky.set_valign(Align::Start);
    sticky.set_halign(Align::Fill);
    sticky.set_visible(false);
    let sticky_label = Label::new(None);
    sticky_label.set_use_markup(true);
    sticky_label.set_xalign(0.0);
    sticky.append(&sticky_label);

    let overlay = Overlay::new();
    overlay.set_child(Some(&diff_scroll));
    overlay.add_overlay(&sticky);

    let main_box = GtkBox::new(Orientation::Vertical, 0);
    main_box.append(&toolbar);
    main_box.append(&overlay);

    // --- Outer horizontal Paned (side panel | main) ------------------------
    let outer = Paned::new(Orientation::Horizontal);
    outer.set_start_child(Some(&side_paned));
    outer.set_end_child(Some(&main_box));
    outer.set_position(330);
    outer.set_resize_start_child(false);
    outer.set_resize_end_child(true);
    outer.set_shrink_start_child(false);
    outer.set_wide_handle(true);

    let window = ApplicationWindow::builder()
        .application(app)
        .title("git-review · gtk4")
        .default_width(1200)
        .default_height(820)
        .child(&outer)
        .build();

    let ui = Rc::new(Ui {
        summary,
        counts,
        commit_list,
        file_list,
        diff_box,
        diff_scroll: diff_scroll.clone(),
        sticky,
        sticky_label,
        file_anchors: RefCell::new(Vec::new()),
    });

    // Build the (static) commit list once.
    build_commit_list(&ui, &shared);

    // Wire toolbar buttons.
    {
        let shared = shared.clone();
        let ui = ui.clone();
        wrap_btn.connect_toggled(move |b| {
            shared.borrow_mut().settings.word_wrap = b.is_active();
            refresh(&ui, &shared);
        });
    }
    {
        let shared = shared.clone();
        let ui = ui.clone();
        space_btn.connect_toggled(move |b| {
            shared.borrow_mut().settings.show_space = b.is_active();
            {
                let mut s = shared.borrow_mut();
                s.recompute();
            }
            refresh(&ui, &shared);
        });
    }
    {
        let shared = shared.clone();
        let ui = ui.clone();
        lines_btn.connect_toggled(move |b| {
            shared.borrow_mut().settings.line_numbers = b.is_active();
            refresh(&ui, &shared);
        });
    }
    {
        let shared = shared.clone();
        let ui = ui.clone();
        font_dec.connect_clicked(move |_| {
            {
                let mut s = shared.borrow_mut();
                s.settings.font_size = (s.settings.font_size - 1).max(8);
            }
            refresh(&ui, &shared);
        });
    }
    {
        let shared = shared.clone();
        let ui = ui.clone();
        font_inc.connect_clicked(move |_| {
            {
                let mut s = shared.borrow_mut();
                s.settings.font_size = (s.settings.font_size + 1).min(28);
            }
            refresh(&ui, &shared);
        });
    }

    // Update sticky header as the diff scrolls.
    {
        let ui = ui.clone();
        let shared = shared.clone();
        diff_scroll
            .vadjustment()
            .connect_value_changed(move |adj| update_sticky(&ui, &shared, adj.value()));
    }

    refresh(&ui, &shared);
    window.present();
}

// --- Commit list -------------------------------------------------------------

fn build_commit_list(ui: &Rc<Ui>, shared: &Shared) {
    let commits = shared.borrow().commits.clone();
    for c in &commits {
        let row = ListBoxRow::new();
        row.set_selectable(false);
        row.set_activatable(false);
        let vbox = GtkBox::new(Orientation::Vertical, 2);
        vbox.add_css_class("commit-row");

        // line 1: endpoint buttons + sha + date + author
        let line1 = GtkBox::new(Orientation::Horizontal, 6);
        if let Some(oid) = c.oid {
            let from_btn = ToggleButton::with_label("◀");
            from_btn.add_css_class("endpoint");
            from_btn.set_tooltip_text(Some("Compare from this commit"));
            let to_btn = ToggleButton::with_label("▶");
            to_btn.add_css_class("endpoint");
            to_btn.set_tooltip_text(Some("Compare to this commit"));
            {
                let shared = shared.clone();
                let ui = ui.clone();
                from_btn.connect_clicked(move |_| {
                    shared.borrow_mut().from = Some(oid);
                    maybe_range(&ui, &shared);
                });
            }
            {
                let shared = shared.clone();
                let ui = ui.clone();
                to_btn.connect_clicked(move |_| {
                    shared.borrow_mut().to = Some(oid);
                    maybe_range(&ui, &shared);
                });
            }
            line1.append(&from_btn);
            line1.append(&to_btn);
        } else {
            let pad = GtkBox::new(Orientation::Horizontal, 0);
            pad.set_size_request(56, -1);
            line1.append(&pad);
        }

        let sha = Label::new(Some(&c.short));
        sha.add_css_class("accent");
        line1.append(&sha);
        let date = Label::new(Some(&c.date));
        date.add_css_class("muted");
        line1.append(&date);
        let author = Label::new(Some(&c.author));
        author.add_css_class("muted");
        author.set_ellipsize(pango::EllipsizeMode::End);
        author.set_xalign(1.0);
        author.set_hexpand(true);
        line1.append(&author);
        vbox.append(&line1);

        // line 2: title (click to open)
        let title = Label::new(Some(&c.title));
        title.add_css_class("title");
        title.set_ellipsize(pango::EllipsizeMode::End);
        title.set_xalign(0.0);
        vbox.append(&title);

        // Click anywhere on the row body opens the commit/working tree.
        let gesture = gtk::GestureClick::new();
        {
            let shared = shared.clone();
            let ui = ui.clone();
            let oid = c.oid;
            gesture.connect_released(move |_, _, _, _| {
                let showing = match oid {
                    Some(o) => Showing::Commit(o),
                    None => Showing::Working,
                };
                let changed = {
                    let mut s = shared.borrow_mut();
                    if s.showing != showing {
                        s.showing = showing;
                        s.recompute();
                        true
                    } else {
                        false
                    }
                };
                if changed {
                    refresh(&ui, &shared);
                }
            });
        }
        vbox.add_controller(gesture);

        row.set_child(Some(&vbox));
        ui.commit_list.append(&row);
    }
}

fn maybe_range(ui: &Rc<Ui>, shared: &Shared) {
    let changed = {
        let mut s = shared.borrow_mut();
        if let (Some(a), Some(b)) = (s.from, s.to) {
            let showing = Showing::Range(a, b);
            if s.showing != showing {
                s.showing = showing;
                s.recompute();
                true
            } else {
                false
            }
        } else {
            false
        }
    };
    if changed {
        refresh(ui, shared);
    }
}

/// Re-apply the "current" CSS class to the row that matches the active view.
fn update_commit_selection(ui: &Rc<Ui>, shared: &Shared) {
    let (showing, commits) = {
        let s = shared.borrow();
        (s.showing, s.commits.clone())
    };
    let mut i = 0;
    while let Some(row) = ui.commit_list.row_at_index(i) {
        let c = &commits[i as usize];
        let is_current = match (showing, c.oid) {
            (Showing::Working, None) => true,
            (Showing::Commit(o), Some(oid)) => o == oid,
            _ => false,
        };
        if let Some(child) = row.child() {
            if is_current {
                child.add_css_class("current");
            } else {
                child.remove_css_class("current");
            }
        }
        i += 1;
    }
}

// --- Full refresh ------------------------------------------------------------

fn refresh(ui: &Rc<Ui>, shared: &Shared) {
    let s = shared.borrow();

    // Toolbar summary + counts.
    if let Some(err) = &s.error {
        ui.summary.set_text(&format!("Error: {err}"));
        ui.counts.set_markup("");
    } else {
        ui.summary.set_text(&s.diff.summary);
        ui.counts.set_markup(&format!(
            "<span foreground='{ADD_FG}'>+{}</span>  <span foreground='{DEL_FG}'>−{}</span>",
            s.diff.added, s.diff.removed
        ));
    }

    rebuild_file_list(ui, &s);
    rebuild_diff(ui, &s);
    drop(s);

    update_commit_selection(ui, shared);
    ui.sticky.set_visible(false);
}

fn rebuild_file_list(ui: &Rc<Ui>, s: &State) {
    while let Some(child) = ui.file_list.first_child() {
        ui.file_list.remove(&child);
    }
    for (i, f) in s.diff.files.iter().enumerate() {
        let row = ListBoxRow::new();
        row.set_selectable(false);
        let hbox = GtkBox::new(Orientation::Horizontal, 6);
        hbox.add_css_class("file-row");

        let icon = Label::new(Some(file_icon(&f.path)));
        hbox.append(&icon);
        let path = Label::new(Some(&f.path));
        path.set_ellipsize(pango::EllipsizeMode::Start);
        path.set_xalign(0.0);
        path.set_hexpand(true);
        hbox.append(&path);

        let plus = Label::new(None);
        plus.set_markup(&format!("<span foreground='{ADD_FG}'>+{}</span>", f.added));
        hbox.append(&plus);
        let minus = Label::new(None);
        minus.set_markup(&format!("<span foreground='{DEL_FG}'>−{}</span>", f.removed));
        hbox.append(&minus);

        row.set_child(Some(&hbox));

        // Click scrolls the diff to this file.
        let gesture = gtk::GestureClick::new();
        {
            let ui = ui.clone();
            gesture.connect_released(move |_, _, _, _| scroll_to_file(&ui, i));
        }
        hbox.add_controller(gesture);

        ui.file_list.append(&row);
    }
}

fn rebuild_diff(ui: &Rc<Ui>, s: &State) {
    // Clear.
    while let Some(child) = ui.diff_box.first_child() {
        ui.diff_box.remove(&child);
    }
    ui.file_anchors.borrow_mut().clear();

    if let Some(err) = &s.error {
        let lbl = Label::new(Some(&format!("Error: {err}")));
        lbl.add_css_class("del-fg");
        lbl.set_xalign(0.0);
        lbl.set_margin_top(12);
        lbl.set_margin_start(12);
        ui.diff_box.append(&lbl);
        return;
    }

    // Commit message (single-commit view).
    if let Some(msg) = &s.diff.message {
        let card = GtkBox::new(Orientation::Vertical, 2);
        card.add_css_class("commit-msg");
        card.set_margin_bottom(10);
        let title = Label::new(Some(&msg.title));
        title.add_css_class("heading");
        title.set_xalign(0.0);
        title.set_wrap(true);
        card.append(&title);
        let meta = Label::new(Some(&format!(
            "{}  ·  {}  ·  {}",
            msg.author, msg.date, msg.short
        )));
        meta.add_css_class("muted");
        meta.set_xalign(0.0);
        card.append(&meta);
        if !msg.body.is_empty() {
            let body = Label::new(Some(&msg.body));
            body.add_css_class("body");
            body.set_xalign(0.0);
            body.set_wrap(true);
            body.set_margin_top(8);
            card.append(&body);
        }
        ui.diff_box.append(&card);
    }

    for f in &s.diff.files {
        // Header widget (also the scroll-to / sticky anchor).
        let header = file_header_widget(f);
        ui.diff_box.append(&header);
        ui.file_anchors.borrow_mut().push(header);

        // Body.
        if f.binary {
            let lbl = Label::new(Some("Binary file not shown"));
            lbl.add_css_class("binary");
            lbl.set_xalign(0.0);
            ui.diff_box.append(&lbl);
        } else {
            let tv = build_file_textview(s, f);
            ui.diff_box.append(&tv);
        }

        let pad = GtkBox::new(Orientation::Vertical, 0);
        pad.set_size_request(-1, 10);
        ui.diff_box.append(&pad);
    }
}

fn file_header_widget(f: &FileDiff) -> GtkBox {
    let header = GtkBox::new(Orientation::Horizontal, 8);
    header.add_css_class("file-header");

    let icon = Label::new(Some(file_icon(&f.path)));
    header.append(&icon);

    let label = match (&f.old_path, f.kind) {
        (Some(old), ChangeKind::Renamed) => format!("{old}  →  {}", f.path),
        _ => f.path.clone(),
    };
    let path = Label::new(Some(&label));
    path.add_css_class("path");
    path.set_xalign(0.0);
    path.set_ellipsize(pango::EllipsizeMode::Start);
    header.append(&path);

    let kind = Label::new(Some(&format!("[{}]", f.kind.letter())));
    kind.add_css_class("muted");
    header.append(&kind);

    let spacer = GtkBox::new(Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    header.append(&spacer);

    let plus = Label::new(None);
    plus.set_markup(&format!("<span foreground='{ADD_FG}'>+{}</span>", f.added));
    header.append(&plus);
    let minus = Label::new(None);
    minus.set_markup(&format!("<span foreground='{DEL_FG}'>−{}</span>", f.removed));
    header.append(&minus);

    header
}

/// Build a `TextView` carrying the full unified diff for one file, with a line-number/sign
/// gutter, syntax-highlighted code, and full-width per-line backgrounds.
fn build_file_textview(s: &State, f: &FileDiff) -> TextView {
    let buffer = TextBuffer::new(None);
    let tags = make_tags(&buffer);
    let syntax = s.hl.syntax_for(&f.path);
    let line_numbers = s.settings.line_numbers;

    let mut iter = buffer.start_iter();
    let mut first = true;

    for hunk in &f.hunks {
        if !first {
            buffer.insert(&mut iter, "\n");
        }
        first = false;
        // Hunk header line.
        let gutter = gutter_text(line_numbers, None, None, ' ');
        let start_off = iter.offset();
        buffer.insert(&mut iter, &gutter);
        buffer.insert(&mut iter, &hunk.header);
        apply_para(&buffer, &tags.hunk, start_off, iter.offset());

        for line in &hunk.lines {
            buffer.insert(&mut iter, "\n");
            let sign = match line.kind {
                LineKind::Added => '+',
                LineKind::Removed => '-',
                LineKind::Context => ' ',
            };
            let line_start = iter.offset();
            let g = gutter_text(line_numbers, line.old_no, line.new_no, sign);
            buffer.insert(&mut iter, &g);
            // Tag the gutter muted.
            apply_range(&buffer, &tags.muted, line_start, iter.offset());

            // Code with syntax spans.
            let code_start = iter.offset();
            let spans = s.hl.line(syntax, &line.text);
            if spans.is_empty() {
                buffer.insert(&mut iter, &line.text);
            } else {
                for span in &spans {
                    let span_start = iter.offset();
                    buffer.insert(&mut iter, &span.text);
                    let tag = color_tag(&buffer, &tags, span.color, span.bold, span.italic);
                    apply_range(&buffer, &tag, span_start, iter.offset());
                }
            }
            let _ = code_start;

            // Full-width paragraph background + marker for added/removed.
            let bg_tag = match line.kind {
                LineKind::Added => Some(&tags.add_bg),
                LineKind::Removed => Some(&tags.del_bg),
                LineKind::Context => None,
            };
            if let Some(tag) = bg_tag {
                apply_para(&buffer, tag, line_start, iter.offset());
            }
            // Sign marker colour.
            if sign != ' ' {
                let marker_tag = if sign == '+' { &tags.add_fg } else { &tags.del_fg };
                apply_range(&buffer, marker_tag, line_start, line_start + gutter_sign_offset(line_numbers) + 1);
            }
        }
    }

    // Apply the chosen font size across the whole buffer (monospace family from set_monospace).
    {
        let font_tag = TextTag::new(None);
        font_tag.set_size_points(s.settings.font_size as f64);
        buffer.tag_table().add(&font_tag);
        let start = buffer.start_iter();
        let end = buffer.end_iter();
        buffer.apply_tag(&font_tag, &start, &end);
    }

    let tv = TextView::with_buffer(&buffer);
    tv.set_editable(false);
    tv.set_cursor_visible(false);
    tv.set_monospace(true);
    tv.set_left_margin(6);
    tv.set_right_margin(6);
    tv.set_top_margin(2);
    tv.set_bottom_margin(2);
    if s.settings.word_wrap {
        tv.set_wrap_mode(WrapMode::WordChar);
    } else {
        tv.set_wrap_mode(WrapMode::None);
    }
    tv
}

/// Holds the static tags used across a buffer. Color tags for syntax are created lazily.
struct Tags {
    hunk: TextTag,
    add_bg: TextTag,
    del_bg: TextTag,
    add_fg: TextTag,
    del_fg: TextTag,
    muted: TextTag,
    color_cache: RefCell<std::collections::HashMap<(u8, u8, u8, bool, bool), TextTag>>,
}

fn make_tags(buffer: &TextBuffer) -> Tags {
    let table = buffer.tag_table();
    let mk = |name: &str| {
        let t = TextTag::new(Some(name));
        table.add(&t);
        t
    };
    let hunk = mk("hunk");
    hunk.set_paragraph_background(Some(HUNK_BG));
    hunk.set_foreground(Some(MUTED));

    let add_bg = mk("add_bg");
    add_bg.set_paragraph_background(Some(ADD_BG));
    let del_bg = mk("del_bg");
    del_bg.set_paragraph_background(Some(DEL_BG));

    let add_fg = mk("add_fg");
    add_fg.set_foreground(Some(ADD_FG));
    let del_fg = mk("del_fg");
    del_fg.set_foreground(Some(DEL_FG));

    let muted = mk("muted");
    muted.set_foreground(Some(MUTED));

    Tags {
        hunk,
        add_bg,
        del_bg,
        add_fg,
        del_fg,
        muted,
        color_cache: RefCell::new(std::collections::HashMap::new()),
    }
}

fn color_tag(
    buffer: &TextBuffer,
    tags: &Tags,
    rgb: (u8, u8, u8),
    bold: bool,
    italic: bool,
) -> TextTag {
    let key = (rgb.0, rgb.1, rgb.2, bold, italic);
    if let Some(t) = tags.color_cache.borrow().get(&key) {
        return t.clone();
    }
    let t = TextTag::new(None);
    t.set_foreground(Some(&format!("#{:02x}{:02x}{:02x}", rgb.0, rgb.1, rgb.2)));
    if bold {
        t.set_weight(700); // pango bold
    }
    if italic {
        t.set_style(pango::Style::Italic);
    }
    buffer.tag_table().add(&t);
    tags.color_cache.borrow_mut().insert(key, t.clone());
    t
}

fn apply_range(buffer: &TextBuffer, tag: &TextTag, start: i32, end: i32) {
    let s = buffer.iter_at_offset(start);
    let e = buffer.iter_at_offset(end);
    buffer.apply_tag(tag, &s, &e);
}

fn apply_para(buffer: &TextBuffer, tag: &TextTag, start: i32, end: i32) {
    apply_range(buffer, tag, start, end);
}

/// Width of the gutter prefix before the sign char, in characters.
fn gutter_sign_offset(line_numbers: bool) -> i32 {
    if line_numbers {
        // "%4 %4 " -> two 4-wide numbers + two spaces = 10 chars before sign
        10
    } else {
        0
    }
}

/// Build the textual gutter prefix: old/new line numbers (optional) + the sign char + a space.
fn gutter_text(line_numbers: bool, old: Option<u32>, new: Option<u32>, sign: char) -> String {
    if line_numbers {
        let o = old.map(|n| n.to_string()).unwrap_or_default();
        let n = new.map(|n| n.to_string()).unwrap_or_default();
        format!("{o:>4} {n:>4} {sign} ")
    } else {
        format!("{sign} ")
    }
}

// --- Scroll-to-file + sticky -------------------------------------------------

fn scroll_to_file(ui: &Rc<Ui>, idx: usize) {
    let anchor = {
        let anchors = ui.file_anchors.borrow();
        anchors.get(idx).cloned()
    };
    let Some(anchor) = anchor else { return };
    // Compute the anchor's top relative to the diff_box (the scrolled child).
    if let Some(bounds) = anchor.compute_bounds(&ui.diff_box) {
        let y = bounds.y() as f64;
        let adj = ui.diff_scroll.vadjustment();
        let max = (adj.upper() - adj.page_size()).max(0.0);
        adj.set_value(y.min(max));
    }
}

/// Find the topmost file header at or above the viewport top, and pin its label in the overlay.
fn update_sticky(ui: &Rc<Ui>, shared: &Shared, scroll_y: f64) {
    let anchors = ui.file_anchors.borrow();
    if anchors.is_empty() {
        ui.sticky.set_visible(false);
        return;
    }
    let mut current: Option<usize> = None;
    for (i, a) in anchors.iter().enumerate() {
        if let Some(b) = a.compute_bounds(&ui.diff_box) {
            // header top relative to viewport
            if (b.y() as f64) <= scroll_y + 0.5 {
                current = Some(i);
            } else {
                break;
            }
        }
    }
    let s = shared.borrow();
    match current.and_then(|i| s.diff.files.get(i)) {
        Some(f) => {
            let label = match (&f.old_path, f.kind) {
                (Some(old), ChangeKind::Renamed) => format!("{old}  →  {}", f.path),
                _ => f.path.clone(),
            };
            ui.sticky_label.set_markup(&format!(
                "{}  <span weight='bold'>{}</span>   <span foreground='{ADD_FG}'>+{}</span> <span foreground='{DEL_FG}'>−{}</span>",
                file_icon(&f.path),
                glib::markup_escape_text(&label),
                f.added,
                f.removed
            ));
            ui.sticky.set_visible(true);
        }
        None => ui.sticky.set_visible(false),
    }
}

// --- small widgets -----------------------------------------------------------

fn tool_toggle(label: &str, tip: &str, active: bool) -> ToggleButton {
    let b = ToggleButton::with_label(label);
    b.add_css_class("tool");
    b.set_tooltip_text(Some(tip));
    b.set_active(active);
    b
}

fn tool_button(label: &str, tip: &str) -> Button {
    let b = Button::with_label(label);
    b.add_css_class("tool");
    b.set_tooltip_text(Some(tip));
    b
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

fn empty_diff() -> DiffSet {
    DiffSet {
        message: None,
        files: Vec::new(),
        added: 0,
        removed: 0,
        summary: String::new(),
    }
}
