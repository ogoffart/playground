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
    gdk, gio, glib, pango, Align, Application, ApplicationWindow, Box as GtkBox, Button,
    CssProvider, Label, ListBox, ListBoxRow, ListView, Orientation, Overlay, Paned, PolicyType,
    ScrolledWindow, SelectionMode, SignalListItemFactory, SingleSelection, TextBuffer, TextTag,
    TextView, ToggleButton, TreeExpander, TreeListModel, TreeListRow, WrapMode,
};

use crate::git::{ChangeKind, CommitInfo, DiffSet, FileDiff, LineKind, Repo};
use crate::highlight::Highlighter;

// --- GitHub palette (light + dark); the active one is chosen at startup ------
#[derive(Clone, Copy)]
struct Palette {
    bg: &'static str,
    panel: &'static str,
    border: &'static str,
    text: &'static str,
    muted: &'static str,
    accent: &'static str,
    sel: &'static str,
    add_bg: &'static str,
    del_bg: &'static str,
    add_fg: &'static str,
    del_fg: &'static str,
    hunk_bg: &'static str,
}

const LIGHT: Palette = Palette {
    bg: "#ffffff",
    panel: "#f6f8fa",
    border: "#d0d7de",
    text: "#1f2328",
    muted: "#656d76",
    accent: "#0969da",
    sel: "#ddf4ff",
    add_bg: "#e6ffec",
    del_bg: "#ffebe9",
    add_fg: "#1a7f37",
    del_fg: "#cf222e",
    hunk_bg: "#ddf4ff",
};

const DARK: Palette = Palette {
    bg: "#0d1117",
    panel: "#161b22",
    border: "#30363d",
    text: "#e6edf3",
    muted: "#8b949e",
    accent: "#2f81f7",
    sel: "#1f6feb",
    add_bg: "#12261e",
    del_bg: "#25171c",
    add_fg: "#3fb950",
    del_fg: "#f85149",
    hunk_bg: "#1c2a3a",
};

thread_local! {
    static PALETTE: std::cell::Cell<Palette> = const { std::cell::Cell::new(LIGHT) };
}

fn pal() -> Palette {
    PALETTE.with(|p| p.get())
}

/// Resolve the desktop colour scheme. Honors GTK's `gtk-application-prefer-dark-theme`
/// (which GTK sets from the XDG `org.freedesktop.appearance color-scheme` portal /
/// `prefers-color-scheme`). Headless / no preference -> light.
fn detect_dark() -> bool {
    gtk::Settings::default()
        .map(|s| s.is_gtk_application_prefer_dark_theme())
        .unwrap_or(false)
}

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
    file_tree: ListView,
    diff_box: GtkBox,
    diff_scroll: ScrolledWindow,
    sticky: GtkBox,
    sticky_label: Label,
    // Per-file top widget (header) within diff_box, used for scroll-to and sticky tracking.
    file_anchors: RefCell<Vec<GtkBox>>,
}

type Shared = Rc<RefCell<State>>;

pub fn load_css() {
    // Decide the scheme once and lock the active palette.
    let dark = detect_dark();
    PALETTE.with(|p| p.set(if dark { DARK } else { LIGHT }));

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
    let Palette {
        bg,
        panel,
        border,
        text,
        muted,
        accent,
        sel,
        add_fg,
        del_fg,
        ..
    } = pal();
    format!(
        r#"
        window {{ background: {bg}; color: {text}; }}
        .panel {{ background: {panel}; }}
        .toolbar {{ background: {panel}; border-bottom: 1px solid {border}; padding: 5px 8px; }}
        .section-title {{ color: {muted}; font-size: 10px; font-weight: bold; padding: 6px 8px 2px 8px; }}
        .summary {{ font-family: monospace; font-weight: bold; color: {text}; }}
        .add-fg {{ color: {add_fg}; font-weight: bold; }}
        .del-fg {{ color: {del_fg}; font-weight: bold; }}
        .muted {{ color: {muted}; }}
        .accent {{ color: {accent}; font-family: monospace; }}

        .tool {{ padding: 2px 8px; min-height: 0; font-family: monospace; border: 1px solid {border}; background: {bg}; }}
        .tool:checked {{ background: {sel}; color: {accent}; }}

        .commit-row {{ padding: 5px 8px; border-bottom: 1px solid {border}; }}
        .commit-row.current {{ background: {sel}; }}
        .endpoint {{ padding: 0 5px; min-height: 0; min-width: 0; font-size: 10px; }}
        .endpoint:checked {{ background: {accent}; color: white; }}
        .title {{ color: {text}; }}

        .tree-row {{ padding: 2px 4px; }}
        .tree-row:hover {{ background: {sel}; }}

        .file-header {{
            background: {panel};
            border: 1px solid {border};
            padding: 6px 10px;
        }}
        .file-header .path {{ font-weight: bold; color: {text}; }}
        .sticky {{ background: {panel}; border: 1px solid {border}; padding: 6px 10px; }}

        .commit-msg {{
            background: {panel};
            border: 1px solid {border};
            padding: 12px;
        }}
        .commit-msg .heading {{ font-size: 16px; font-weight: bold; color: {text}; }}
        .commit-msg .body {{ font-family: monospace; color: {text}; }}

        textview {{ background: {bg}; color: {text}; }}
        textview text {{ background: {bg}; }}
        listview {{ background: {panel}; }}
        listview > row {{ background: transparent; }}
        .binary {{ color: {muted}; font-style: italic; padding: 6px 12px; }}
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

    // Real hierarchical file tree: GtkTreeListModel + GtkListView with expanders.
    let file_tree = build_file_tree_view();
    let file_scroll = ScrolledWindow::new();
    file_scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    file_scroll.set_child(Some(&file_tree));
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
    outer.set_vexpand(true);

    // --- Window: full-width toolbar on TOP, then the [side | main] paned ----
    let root = GtkBox::new(Orientation::Vertical, 0);
    root.append(&toolbar);
    root.append(&outer);

    let window = ApplicationWindow::builder()
        .application(app)
        .title("git-review · gtk4")
        .default_width(1280)
        .default_height(800)
        .child(&root)
        .build();
    window.set_default_size(1280, 800);

    let ui = Rc::new(Ui {
        summary,
        counts,
        commit_list,
        file_tree: file_tree.clone(),
        diff_box,
        diff_scroll: diff_scroll.clone(),
        sticky,
        sticky_label,
        file_anchors: RefCell::new(Vec::new()),
    });

    // Wire the tree-view factory now that we have the `Ui` (leaf clicks scroll the diff).
    setup_tree_factory(&file_tree, &ui);

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
        let p = pal();
        ui.counts.set_markup(&format!(
            "<span foreground='{}'>+{}</span>  <span foreground='{}'>−{}</span>",
            p.add_fg, s.diff.added, p.del_fg, s.diff.removed
        ));
    }

    rebuild_file_tree(ui, &s);
    rebuild_diff(ui, &s);
    drop(s);

    update_commit_selection(ui, shared);
    ui.sticky.set_visible(false);
}

// --- File tree (real hierarchy via GtkTreeListModel + GtkListView) ----------

/// A plain in-memory tree built from the flat `FileDiff` list. Directory chains with a single
/// child are collapsed GitHub-style (e.g. `a/b/c.rs` becomes one folder `a/b`).
#[derive(Default)]
struct TreeBuild {
    /// children of this node, keyed by the next path segment
    dirs: std::collections::BTreeMap<String, TreeBuild>,
    /// (segment label, file index) for leaf files directly under this node
    files: Vec<(String, usize)>,
}

fn build_tree(files: &[FileDiff]) -> TreeBuild {
    let mut root = TreeBuild::default();
    for (i, f) in files.iter().enumerate() {
        let parts: Vec<&str> = f.path.split('/').collect();
        let mut node = &mut root;
        for seg in &parts[..parts.len() - 1] {
            node = node.dirs.entry(seg.to_string()).or_default();
        }
        let leaf = parts.last().copied().unwrap_or(&f.path);
        node.files.push((leaf.to_string(), i));
    }
    root
}

/// Flatten one tree node into `FileNode` GObjects, collapsing single-child dir chains.
fn node_to_objects(prefix: &str, build: &TreeBuild, files: &[FileDiff]) -> Vec<node::FileNode> {
    let mut out = Vec::new();
    for (name, sub) in &build.dirs {
        // Collapse single-child directory chains: keep descending while this dir has exactly
        // one directory child and no file leaves.
        let mut label = name.clone();
        let mut cur = sub;
        while cur.files.is_empty() && cur.dirs.len() == 1 {
            let (n, s) = cur.dirs.iter().next().unwrap();
            label = format!("{label}/{n}");
            cur = s;
        }
        let full = if prefix.is_empty() {
            label.clone()
        } else {
            format!("{prefix}/{label}")
        };
        let dir_node = node::FileNode::new_dir(&label);
        let children = node_to_objects(&full, cur, files);
        dir_node.set_children(children);
        out.push(dir_node);
    }
    for (name, idx) in &build.files {
        let f = &files[*idx];
        out.push(node::FileNode::new_file(name, *idx, f.added, f.removed, file_icon(&f.path)));
    }
    out
}

/// Create the empty ListView; the model is (re)attached in `rebuild_file_tree`.
fn build_file_tree_view() -> ListView {
    let view = ListView::new(None::<SingleSelection>, None::<SignalListItemFactory>);
    view.add_css_class("panel");
    view.set_vexpand(true);
    view
}

/// Install the item factory (rows: expander + icon + name + counts). Separated so it can
/// capture the `Ui` for leaf-click scroll-to-file.
fn setup_tree_factory(view: &ListView, ui: &Rc<Ui>) {
    let factory = SignalListItemFactory::new();
    factory.connect_setup(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let expander = TreeExpander::new();
        let row = GtkBox::new(Orientation::Horizontal, 6);
        row.add_css_class("tree-row");
        let icon = Label::new(None);
        let name = Label::new(None);
        name.set_xalign(0.0);
        name.set_ellipsize(pango::EllipsizeMode::Middle);
        name.set_hexpand(true);
        let counts = Label::new(None);
        counts.set_use_markup(true);
        row.append(&icon);
        row.append(&name);
        row.append(&counts);
        expander.set_child(Some(&row));
        item.set_child(Some(&expander));
    });
    {
        factory.connect_bind(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let row = item.item().and_downcast::<TreeListRow>().unwrap();
            let node = row.item().and_downcast::<node::FileNode>().unwrap();
            let expander = item.child().and_downcast::<TreeExpander>().unwrap();
            expander.set_list_row(Some(&row));
            let hbox = expander.child().and_downcast::<GtkBox>().unwrap();
            let icon = hbox.first_child().and_downcast::<Label>().unwrap();
            let name = icon.next_sibling().and_downcast::<Label>().unwrap();
            let counts = name.next_sibling().and_downcast::<Label>().unwrap();

            if node.is_dir() {
                icon.set_text("📁");
                name.set_text(&node.name());
                counts.set_markup("");
            } else {
                icon.set_text(&node.icon());
                name.set_text(&node.name());
                let p = pal();
                counts.set_markup(&format!(
                    "<span foreground='{}'>+{}</span> <span foreground='{}'>−{}</span>",
                    p.add_fg,
                    node.added(),
                    p.del_fg,
                    node.removed()
                ));
            }
        });
    }
    view.set_factory(Some(&factory));

    // Activating a row: leaf -> scroll diff; folder -> toggle expansion.
    let ui = ui.clone();
    view.connect_activate(move |view, pos| {
        let model = view.model().unwrap();
        if let Some(row) = model.item(pos).and_downcast::<TreeListRow>() {
            if let Some(node) = row.item().and_downcast::<node::FileNode>() {
                if node.is_dir() {
                    row.set_expanded(!row.is_expanded());
                } else {
                    scroll_to_file(&ui, node.index() as usize);
                }
            }
        }
    });
}

fn rebuild_file_tree(ui: &Rc<Ui>, s: &State) {
    let build = build_tree(&s.diff.files);
    let roots = node_to_objects("", &build, &s.diff.files);

    let root_model = gio::ListStore::new::<node::FileNode>();
    for n in roots {
        root_model.append(&n);
    }

    let tree_model = TreeListModel::new(root_model, false, true, |obj| {
        let node = obj.downcast_ref::<node::FileNode>().unwrap();
        if node.is_dir() {
            let store = gio::ListStore::new::<node::FileNode>();
            for c in node.children() {
                store.append(&c);
            }
            Some(store.upcast())
        } else {
            None
        }
    });

    let selection = SingleSelection::new(Some(tree_model));
    selection.set_autoselect(false);
    selection.set_can_unselect(true);
    ui.file_tree.set_model(Some(&selection));
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
    plus.set_markup(&format!("<span foreground='{}'>+{}</span>", pal().add_fg, f.added));
    header.append(&plus);
    let minus = Label::new(None);
    minus.set_markup(&format!("<span foreground='{}'>−{}</span>", pal().del_fg, f.removed));
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
    let p = pal();
    let hunk = mk("hunk");
    hunk.set_paragraph_background(Some(p.hunk_bg));
    hunk.set_foreground(Some(p.muted));

    let add_bg = mk("add_bg");
    add_bg.set_paragraph_background(Some(p.add_bg));
    let del_bg = mk("del_bg");
    del_bg.set_paragraph_background(Some(p.del_bg));

    let add_fg = mk("add_fg");
    add_fg.set_foreground(Some(p.add_fg));
    let del_fg = mk("del_fg");
    del_fg.set_foreground(Some(p.del_fg));

    let muted = mk("muted");
    muted.set_foreground(Some(p.muted));

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
            let p = pal();
            ui.sticky_label.set_markup(&format!(
                "{}  <span weight='bold'>{}</span>   <span foreground='{}'>+{}</span> <span foreground='{}'>−{}</span>",
                file_icon(&f.path),
                glib::markup_escape_text(&label),
                p.add_fg,
                f.added,
                p.del_fg,
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

/// `FileNode`: the GObject model item backing each row of the file tree. A node is either a
/// directory (has `children`) or a file leaf (carries `index`, `added`, `removed`, `icon`).
mod node {
    use std::cell::RefCell;

    use gtk::glib;
    use gtk::subclass::prelude::*;

    #[derive(Default)]
    pub struct FileNodeInner {
        pub name: RefCell<String>,
        pub icon: RefCell<String>,
        pub is_dir: std::cell::Cell<bool>,
        pub index: std::cell::Cell<u32>,
        pub added: std::cell::Cell<u32>,
        pub removed: std::cell::Cell<u32>,
        pub children: RefCell<Vec<super::node::FileNode>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FileNodeInner {
        const NAME: &'static str = "GitReviewFileNode";
        type Type = FileNode;
    }

    impl ObjectImpl for FileNodeInner {}

    glib::wrapper! {
        pub struct FileNode(ObjectSubclass<FileNodeInner>);
    }

    impl FileNode {
        pub fn new_dir(name: &str) -> Self {
            let obj: Self = glib::Object::new();
            let inner = obj.imp();
            *inner.name.borrow_mut() = name.to_string();
            inner.is_dir.set(true);
            obj
        }

        pub fn new_file(name: &str, index: usize, added: u32, removed: u32, icon: &str) -> Self {
            let obj: Self = glib::Object::new();
            let inner = obj.imp();
            *inner.name.borrow_mut() = name.to_string();
            *inner.icon.borrow_mut() = icon.to_string();
            inner.is_dir.set(false);
            inner.index.set(index as u32);
            inner.added.set(added);
            inner.removed.set(removed);
            obj
        }

        pub fn is_dir(&self) -> bool {
            self.imp().is_dir.get()
        }
        pub fn name(&self) -> String {
            self.imp().name.borrow().clone()
        }
        pub fn icon(&self) -> String {
            self.imp().icon.borrow().clone()
        }
        pub fn index(&self) -> u32 {
            self.imp().index.get()
        }
        pub fn added(&self) -> u32 {
            self.imp().added.get()
        }
        pub fn removed(&self) -> u32 {
            self.imp().removed.get()
        }
        pub fn children(&self) -> Vec<FileNode> {
            self.imp().children.borrow().clone()
        }
        pub fn set_children(&self, children: Vec<FileNode>) {
            *self.imp().children.borrow_mut() = children;
        }
    }
}
