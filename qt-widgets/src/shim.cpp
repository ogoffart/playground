// Qt Widgets UI for git-review, driven from Rust over a tiny C ABI (see ffi.rs).
//
// No Q_OBJECT / moc is used: all signal/slot wiring goes through Qt's functor-based connect()
// with C++ lambdas, so this file compiles cleanly with just the `cc` crate (no moc pass).

#include <QApplication>
#include <QMainWindow>
#include <QSplitter>
#include <QListWidget>
#include <QTreeWidget>
#include <QTreeWidgetItem>
#include <QTextEdit>
#include <QTextBrowser>
#include <QScrollArea>
#include <QToolBar>
#include <QToolButton>
#include <QLabel>
#include <QVBoxLayout>
#include <QHBoxLayout>
#include <QWidget>
#include <QPushButton>
#include <QFont>
#include <QFontDatabase>
#include <QJsonDocument>
#include <QJsonArray>
#include <QJsonObject>
#include <QJsonValue>
#include <QString>
#include <QStringList>
#include <QFileInfo>
#include <QScrollBar>
#include <QStyle>
#include <QFrame>
#include <QHash>
#include <QSizePolicy>
#include <QFontMetrics>
#include <functional>
#include <vector>

#include <QStyleHints>
#include <QGuiApplication>
#include <QPalette>
#include <QColor>

// ---- Rust C ABI ----------------------------------------------------------
extern "C" {
int gr_open(const char *path, int dark);
char *gr_repo_name();
char *gr_commits();
char *gr_working(int show_space_changes);
char *gr_show(const char *oid, int show_space_changes);
char *gr_range(const char *from, const char *to, int show_space_changes);
void gr_free(char *p);
}

// Take ownership of a Rust string into a QString and free it.
static QString takeRust(char *p) {
    if (!p) return QString();
    QString s = QString::fromUtf8(p);
    gr_free(p);
    return s;
}

static QJsonDocument parseRust(char *p) {
    QString s = takeRust(p);
    return QJsonDocument::fromJson(s.toUtf8());
}

// ---- GitHub-ish palette (light + dark) -----------------------------------
// `pal` is filled in at startup from the detected colour scheme.
namespace pal {
const char *bg;
const char *panel;
const char *border;
const char *text;
const char *muted;
const char *accent;
const char *selection;
const char *hover;
const char *addBg;
const char *addMark;
const char *addFg;
const char *delBg;
const char *delMark;
const char *delFg;
const char *hunkBg;
bool dark = false;
}

static void applyPalette(bool dark) {
    pal::dark = dark;
    if (dark) {
        pal::bg = "#0d1117";
        pal::panel = "#161b22";
        pal::border = "#30363d";
        pal::text = "#e6edf3";
        pal::muted = "#8b949e";
        pal::accent = "#2f81f7";
        pal::selection = "#1f6feb";
        pal::hover = "#21262d";
        pal::addBg = "#12261e";
        pal::addMark = "#2ea043";
        pal::addFg = "#3fb950";
        pal::delBg = "#25171c";
        pal::delMark = "#f85149";
        pal::delFg = "#f85149";
        pal::hunkBg = "#161b22";
    } else {
        pal::bg = "#ffffff";
        pal::panel = "#f6f8fa";
        pal::border = "#d0d7de";
        pal::text = "#1f2328";
        pal::muted = "#656d76";
        pal::accent = "#0969da";
        pal::selection = "#ddf4ff";
        pal::hover = "#eaeef2";
        pal::addBg = "#e6ffec";
        pal::addMark = "#abf2bc";
        pal::addFg = "#1a7f37";
        pal::delBg = "#ffebe9";
        pal::delMark = "#ff8182";
        pal::delFg = "#cf222e";
        pal::hunkBg = "#f6f8fa";
    }
}

// Detect the desktop colour scheme. Env override GIT_REVIEW_THEME=dark|light wins;
// then Qt's QStyleHints::colorScheme() (Qt 6.5+); else palette window lightness; headless -> light.
static bool detectDark() {
    QByteArray env = qgetenv("GIT_REVIEW_THEME").toLower();
    if (env == "dark") return true;
    if (env == "light") return false;
#if QT_VERSION >= QT_VERSION_CHECK(6, 5, 0)
    if (auto *h = QGuiApplication::styleHints()) {
        Qt::ColorScheme cs = h->colorScheme();
        if (cs == Qt::ColorScheme::Dark) return true;
        if (cs == Qt::ColorScheme::Light) return false;
    }
#endif
    // Fallback: a dark window colour means a dark desktop theme.
    QColor win = QGuiApplication::palette().color(QPalette::Window);
    return win.lightnessF() < 0.5;
}

struct Settings {
    bool word_wrap = false;
    bool show_space_changes = true; // when false -> ignore whitespace
    int font_size = 12;
    bool line_numbers = true;
};

// Holds runtime UI state shared across closures.
struct AppCtx {
    Settings settings;
    QString summary;
    int totalAdded = 0;
    int totalRemoved = 0;

    // current selection
    QString single;          // oid of single commit, or "WT" for working tree, or "" none
    bool isWorking = false;
    QString fromOid;
    QString toOid;

    // widgets
    QLabel *summaryLabel = nullptr;
    QLabel *countsLabel = nullptr;
    QListWidget *commitList = nullptr;
    QTreeWidget *fileTree = nullptr;
    QWidget *diffContainer = nullptr;
    QVBoxLayout *diffLayout = nullptr;
    QScrollArea *diffScroll = nullptr;

    QToolButton *btnWrap = nullptr;
    QToolButton *btnSpace = nullptr;
    QToolButton *btnLineNo = nullptr;

    // map file path -> section widget (for click-to-scroll)
    std::vector<std::pair<QString, QWidget *>> fileSections;

    std::function<void()> reload; // recompute current diff and rebuild main view
};

static QString htmlEscape(const QString &s) {
    QString o = s;
    o.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    o.replace(' ', "&nbsp;");
    return o;
}

// File-type icon (emoji) by extension, parallel to the other apps.
static QString iconFor(const QString &path) {
    QString ext = QFileInfo(path).suffix().toLower();
    if (ext == "rs") return "\xF0\x9F\xA6\x80";          // crab
    if (ext == "py") return "\xF0\x9F\x90\x8D";          // snake
    if (ext == "js" || ext == "ts" || ext == "jsx" || ext == "tsx") return "\xF0\x9F\x93\x9C";
    if (ext == "json" || ext == "toml" || ext == "yaml" || ext == "yml" || ext == "ini")
        return "\xE2\x9A\x99\xEF\xB8\x8F"; // gear
    if (ext == "md" || ext == "txt") return "\xF0\x9F\x93\x9D";
    if (ext == "sh" || ext == "bash") return "\xF0\x9F\x96\xA5\xEF\xB8\x8F";
    if (ext == "html" || ext == "css") return "\xF0\x9F\x8C\x90";
    if (ext == "c" || ext == "cpp" || ext == "h" || ext == "hpp") return "\xF0\x9F\x94\xA7";
    return "\xF0\x9F\x93\x84"; // page
}

// Build the per-file diff HTML (gutter + change marker + colored spans + line bg).
static QString buildFileHtml(const QJsonObject &file, const Settings &s, int fontPx) {
    QJsonArray hunks = file["hunks"].toArray();
    bool binary = file["binary"].toBool();

    QString mono = QFontDatabase::systemFont(QFontDatabase::FixedFont).family();
    QString html;
    html += QString("<div style='font-family:\"%1\",monospace; font-size:%2px; "
                    "white-space:%3;'>")
                .arg(mono)
                .arg(fontPx)
                .arg(s.word_wrap ? "pre-wrap" : "pre");

    if (binary) {
        html += "<div style='color:" + QString(pal::muted) + "; padding:6px;'>Binary file not shown</div>";
        html += "</div>";
        return html;
    }

    html += "<table cellspacing='0' cellpadding='0' style='border-collapse:collapse; width:100%;'>";
    for (const QJsonValue &hv : hunks) {
        QJsonObject hunk = hv.toObject();
        // hunk header row
        html += "<tr><td colspan='3' style='background:" + QString(pal::hunkBg) + "; color:" +
                QString(pal::accent) + "; padding:2px 8px;'>" +
                htmlEscape(hunk["header"].toString()) + "</td></tr>";

        QJsonArray lines = hunk["lines"].toArray();
        for (const QJsonValue &lv : lines) {
            QJsonObject line = lv.toObject();
            int kind = line["kind"].toInt(); // 0 ctx 1 add 2 del
            int oldNo = line["old_no"].toInt(-1);
            int newNo = line["new_no"].toInt(-1);

            QString rowBg, markBg, marker;
            if (kind == 1) { rowBg = pal::addBg; markBg = pal::addMark; marker = "+"; }
            else if (kind == 2) { rowBg = pal::delBg; markBg = pal::delMark; marker = "-"; }
            else { rowBg = "transparent"; markBg = "transparent"; marker = "&nbsp;"; }

            // spans
            QString code;
            QJsonArray spans = line["spans"].toArray();
            if (spans.isEmpty()) {
                code = "&nbsp;";
            } else {
                for (const QJsonValue &sv : spans) {
                    QJsonObject sp = sv.toObject();
                    QString t = htmlEscape(sp["text"].toString());
                    QString col = sp["color"].toString();
                    QString style = "color:" + col + ";";
                    if (sp["bold"].toBool()) style += "font-weight:bold;";
                    if (sp["italic"].toBool()) style += "font-style:italic;";
                    code += "<span style='" + style + "'>" + t + "</span>";
                }
            }

            QString gutter;
            if (s.line_numbers) {
                QString oldS = oldNo >= 0 ? QString::number(oldNo) : "";
                QString newS = newNo >= 0 ? QString::number(newNo) : "";
                gutter = QString("<td style='width:84px; text-align:right; color:%1; "
                                 "padding:0 6px; background:%2; user-select:none;'>"
                                 "<span style='display:inline-block;width:34px;'>%3</span>"
                                 "<span style='display:inline-block;width:34px;'>%4</span></td>")
                             .arg(pal::muted)
                             .arg(rowBg)
                             .arg(oldS)
                             .arg(newS);
            }

            html += "<tr>";
            html += gutter;
            html += QString("<td style='width:16px; text-align:center; background:%1; color:%2; "
                            "padding:0 2px;'>%3</td>")
                        .arg(markBg)
                        .arg(pal::text)
                        .arg(marker);
            html += QString("<td style='background:%1; padding:0 6px;'>%2</td>")
                        .arg(rowBg)
                        .arg(code);
            html += "</tr>";
        }
    }
    html += "</table></div>";
    return html;
}

// A QTextEdit that sizes itself to its document height (so the outer scroll area scrolls).
static QTextEdit *makeAutoTextEdit() {
    QTextEdit *te = new QTextEdit();
    te->setReadOnly(true);
    te->setFrameStyle(QFrame::NoFrame);
    te->setVerticalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    te->setHorizontalScrollBarPolicy(Qt::ScrollBarAsNeeded);
    return te;
}

static void sizeTextEdit(QTextEdit *te, bool wrap) {
    te->setLineWrapMode(wrap ? QTextEdit::WidgetWidth : QTextEdit::NoWrap);
    te->document()->adjustSize();
    int h = (int)te->document()->size().height() + 8;
    te->setMinimumHeight(h);
    te->setMaximumHeight(h + 4);
}

// ---- File tree (real hierarchy) ------------------------------------------
//
// Items carry: text(0) = display name of this path component (no rich text here;
// leaves get a coloured item-widget set later). Qt::UserRole = full file path on
// leaves only. Qt::UserRole+1 = bool isLeaf. Qt::UserRole+2 = the bare component
// name (used while collapsing single-child dir chains).

// Find or create a child *directory* node with the given component name.
static QTreeWidgetItem *childDir(QTreeWidgetItem *parent, const QString &comp) {
    for (int i = 0; i < parent->childCount(); ++i) {
        QTreeWidgetItem *c = parent->child(i);
        if (!c->data(0, Qt::UserRole + 1).toBool() &&
            c->data(0, Qt::UserRole + 2).toString() == comp) {
            return c;
        }
    }
    QTreeWidgetItem *c = new QTreeWidgetItem(parent);
    c->setData(0, Qt::UserRole + 1, false);
    c->setData(0, Qt::UserRole + 2, comp);
    c->setText(0, "\xF0\x9F\x93\x81  " + comp);
    return c;
}

// Insert a file into the tree, creating intermediate directory nodes.
static void addFileToTree(AppCtx *ctx, const QString &path, const QString &name,
                          int added, int removed) {
    QStringList parts = path.split('/', Qt::SkipEmptyParts);
    QTreeWidgetItem *node = ctx->fileTree->invisibleRootItem();
    for (int i = 0; i < parts.size() - 1; ++i) {
        node = childDir(node, parts[i]);
    }
    QTreeWidgetItem *leaf = new QTreeWidgetItem(node);
    leaf->setData(0, Qt::UserRole, path);
    leaf->setData(0, Qt::UserRole + 1, true);
    leaf->setData(0, Qt::UserRole + 2, name);
    // Coloured rich-text row via an item widget.
    QLabel *lbl = new QLabel(
        QString("%1&nbsp;&nbsp;%2&nbsp;&nbsp;&nbsp;"
                "<span style='color:%3;'>+%4</span> "
                "<span style='color:%5;'>\xE2\x88\x92%6</span>")
            .arg(iconFor(path))
            .arg(name.toHtmlEscaped())
            .arg(pal::addFg)
            .arg(added)
            .arg(pal::delFg)
            .arg(removed));
    lbl->setStyleSheet(QString("color:%1; background:transparent;").arg(pal::text));
    ctx->fileTree->setItemWidget(leaf, 0, lbl);
}

// GitHub-style: collapse a directory whose only child is itself a directory into a
// single combined "a/b" node, recursively. Distinct subtrees keep branching.
static void collapseSingleChildDirs(QTreeWidgetItem *node) {
    for (int i = 0; i < node->childCount(); ++i) {
        QTreeWidgetItem *c = node->child(i);
        bool cIsLeaf = c->data(0, Qt::UserRole + 1).toBool();
        if (cIsLeaf) continue;
        // collapse chains: while this dir has exactly one child that is a dir, merge.
        while (c->childCount() == 1 &&
               !c->child(0)->data(0, Qt::UserRole + 1).toBool()) {
            QTreeWidgetItem *only = c->takeChild(0);
            QString merged = c->data(0, Qt::UserRole + 2).toString() + "/" +
                             only->data(0, Qt::UserRole + 2).toString();
            c->setData(0, Qt::UserRole + 2, merged);
            c->setText(0, "\xF0\x9F\x93\x81  " + merged);
            // adopt grandchildren
            QList<QTreeWidgetItem *> kids = only->takeChildren();
            c->addChildren(kids);
            delete only;
        }
        collapseSingleChildDirs(c);
    }
}

// Rebuild the entire main diff view from a diff JSON object.
static void renderDiff(AppCtx *ctx, const QJsonObject &diff) {
    // clear old
    QLayoutItem *item;
    while ((item = ctx->diffLayout->takeAt(0)) != nullptr) {
        if (item->widget()) item->widget()->deleteLater();
        delete item;
    }
    ctx->fileSections.clear();
    ctx->fileTree->clear();

    ctx->summary = diff["summary"].toString();
    ctx->totalAdded = diff["added"].toInt();
    ctx->totalRemoved = diff["removed"].toInt();
    ctx->summaryLabel->setText(ctx->summary);
    ctx->countsLabel->setText(
        QString("<span style='color:%1;'>+%2</span> &nbsp;"
                "<span style='color:%3;'>\xE2\x88\x92%4</span>")
            .arg(pal::addFg)
            .arg(ctx->totalAdded)
            .arg(pal::delFg)
            .arg(ctx->totalRemoved));

    int fontPx = ctx->settings.font_size + 2;

    // commit message block (single commit only)
    if (diff["has_message"].toBool()) {
        QJsonObject m = diff["message"].toObject();
        QWidget *msg = new QWidget();
        msg->setStyleSheet(QString("background:%1; border:1px solid %2; border-radius:6px;")
                               .arg(pal::panel)
                               .arg(pal::border));
        QVBoxLayout *ml = new QVBoxLayout(msg);
        ml->setContentsMargins(12, 10, 12, 10);
        QLabel *title = new QLabel(m["title"].toString());
        title->setStyleSheet(QString("font-size:%1px; font-weight:bold; color:%2; background:transparent;")
                                 .arg(ctx->settings.font_size + 4)
                                 .arg(pal::text));
        title->setWordWrap(true);
        ml->addWidget(title);
        QLabel *meta = new QLabel(QString("%1 \xC2\xB7 %2 \xC2\xB7 commit %3")
                                      .arg(m["author"].toString())
                                      .arg(m["date"].toString())
                                      .arg(m["short"].toString()));
        meta->setStyleSheet(QString("color:%1; background:transparent;").arg(pal::muted));
        ml->addWidget(meta);
        QString body = m["body"].toString();
        if (!body.isEmpty()) {
            QLabel *b = new QLabel(body);
            b->setWordWrap(true);
            b->setStyleSheet(QString("color:%1; background:transparent; margin-top:6px;").arg(pal::text));
            ml->addWidget(b);
        }
        ctx->diffLayout->addWidget(msg);
    }

    QJsonArray files = diff["files"].toArray();
    if (files.isEmpty()) {
        QLabel *empty = new QLabel("No changes.");
        empty->setStyleSheet(QString("color:%1; padding:16px;").arg(pal::muted));
        ctx->diffLayout->addWidget(empty);
    }

    for (const QJsonValue &fv : files) {
        QJsonObject file = fv.toObject();
        QString path = file["path"].toString();
        int added = file["added"].toInt();
        int removed = file["removed"].toInt();
        int slash = path.lastIndexOf('/');
        QString name = slash >= 0 ? path.mid(slash + 1) : path;

        // ---- file section in main view ----
        QWidget *section = new QWidget();
        QVBoxLayout *sl = new QVBoxLayout(section);
        sl->setContentsMargins(0, 0, 0, 0);
        sl->setSpacing(0);

        // sticky-style header
        QWidget *header = new QWidget();
        header->setStyleSheet(QString("background:%1; border:1px solid %2; "
                                      "border-top-left-radius:6px; border-top-right-radius:6px;")
                                  .arg(pal::panel)
                                  .arg(pal::border));
        QHBoxLayout *hl = new QHBoxLayout(header);
        hl->setContentsMargins(10, 6, 10, 6);
        QLabel *hpath = new QLabel(iconFor(path) + "  " + path);
        hpath->setStyleSheet(QString("font-weight:bold; color:%1; background:transparent;").arg(pal::text));
        hl->addWidget(hpath);
        hl->addStretch();
        QLabel *hcounts = new QLabel(QString("<span style='color:%1;'>+%2</span> "
                                             "<span style='color:%3;'>\xE2\x88\x92%4</span>")
                                         .arg(pal::addFg)
                                         .arg(added)
                                         .arg(pal::delFg)
                                         .arg(removed));
        hcounts->setStyleSheet("background:transparent;");
        hl->addWidget(hcounts);
        sl->addWidget(header);

        QTextEdit *te = makeAutoTextEdit();
        te->setStyleSheet(QString("border:1px solid %1; border-top:none; background:%2;")
                              .arg(pal::border)
                              .arg(pal::bg));
        te->setHtml(buildFileHtml(file, ctx->settings, fontPx));
        sizeTextEdit(te, ctx->settings.word_wrap);
        sl->addWidget(te);

        ctx->diffLayout->addWidget(section);
        ctx->fileSections.push_back({path, section});

        // ---- file-tree entry (real hierarchy, built incrementally below) ----
        addFileToTree(ctx, path, name, added, removed);
    }

    collapseSingleChildDirs(ctx->fileTree->invisibleRootItem());
    ctx->fileTree->expandAll();
    ctx->diffLayout->addStretch();
}

// Compute current diff from selection and render.
static void reloadDiff(AppCtx *ctx) {
    int sp = ctx->settings.show_space_changes ? 1 : 0;
    QJsonDocument doc;
    if (!ctx->fromOid.isEmpty() && !ctx->toOid.isEmpty()) {
        doc = parseRust(gr_range(ctx->fromOid.toUtf8().constData(),
                                 ctx->toOid.toUtf8().constData(), sp));
    } else if (ctx->isWorking) {
        doc = parseRust(gr_working(sp));
    } else if (!ctx->single.isEmpty()) {
        doc = parseRust(gr_show(ctx->single.toUtf8().constData(), sp));
    } else {
        doc = parseRust(gr_working(sp));
    }
    if (doc.isObject()) {
        renderDiff(ctx, doc.object());
    }
}

// Two-line commit row widget.
static QWidget *makeCommitRow(AppCtx *ctx, const QJsonObject &c, QListWidget *list,
                              QListWidgetItem *item) {
    bool working = c["working"].toBool();
    QString oid = c["oid"].toString();

    QWidget *w = new QWidget();
    QVBoxLayout *v = new QVBoxLayout(w);
    v->setContentsMargins(8, 6, 8, 6);
    v->setSpacing(2);

    // line 1: [from][to] sha date author
    QWidget *l1 = new QWidget();
    QHBoxLayout *h = new QHBoxLayout(l1);
    h->setContentsMargins(0, 0, 0, 0);
    h->setSpacing(4);

    auto makeEndpoint = [&](const QString &label) {
        QPushButton *b = new QPushButton(label);
        b->setFixedHeight(18);
        b->setCursor(Qt::PointingHandCursor);
        b->setStyleSheet(QString(
            "QPushButton{font-size:10px; padding:0 5px; border:1px solid %1; border-radius:4px;"
            "background:%2; color:%3;} QPushButton:hover{background:%4;}")
            .arg(pal::border).arg(pal::bg).arg(pal::muted).arg(pal::hover));
        return b;
    };

    if (!working) {
        QPushButton *fromB = makeEndpoint("from");
        QPushButton *toB = makeEndpoint("to");
        QObject::connect(fromB, &QPushButton::clicked, [ctx, oid]() {
            ctx->fromOid = oid;
            reloadDiff(ctx);
        });
        QObject::connect(toB, &QPushButton::clicked, [ctx, oid]() {
            ctx->toOid = oid;
            reloadDiff(ctx);
        });
        h->addWidget(fromB);
        h->addWidget(toB);
    }

    QLabel *sha = new QLabel(working ? "working" : c["short"].toString());
    sha->setStyleSheet(QString("font-family:monospace; color:%1; background:transparent;").arg(pal::accent));
    h->addWidget(sha);
    QLabel *date = new QLabel(c["date"].toString());
    date->setStyleSheet(QString("color:%1; background:transparent;").arg(pal::muted));
    h->addWidget(date);
    QLabel *author = new QLabel(c["author"].toString());
    author->setStyleSheet(QString("color:%1; background:transparent;").arg(pal::muted));
    author->setMaximumWidth(120);
    h->addWidget(author);
    h->addStretch();
    v->addWidget(l1);

    // line 2: title
    QLabel *title = new QLabel(c["title"].toString());
    title->setStyleSheet(QString("color:%1; background:transparent;").arg(pal::text));
    title->setMaximumWidth(260);
    QFontMetrics fm(title->font());
    title->setText(fm.elidedText(c["title"].toString(), Qt::ElideRight, 250));
    v->addWidget(title);

    item->setSizeHint(w->sizeHint());
    return w;
}

extern "C" int gr_run_app(int argc, const char **argv, const char *repo_path) {
    static int s_argc = argc;
    QApplication app(s_argc, const_cast<char **>(argv));
    app.setApplicationName("git-review (Qt Widgets)");

    // Follow the desktop light/dark colour scheme.
    bool dark = detectDark();
    applyPalette(dark);

    // Apply a matching QPalette so native chrome (scrollbars, tooltips, base widgets)
    // tracks the scheme too, alongside the per-widget stylesheets below.
    {
        QPalette p = app.palette();
        QColor bg(pal::bg), panel(pal::panel), text(pal::text), accent(pal::accent),
            sel(pal::selection), border(pal::border);
        p.setColor(QPalette::Window, bg);
        p.setColor(QPalette::WindowText, text);
        p.setColor(QPalette::Base, bg);
        p.setColor(QPalette::AlternateBase, panel);
        p.setColor(QPalette::Text, text);
        p.setColor(QPalette::Button, panel);
        p.setColor(QPalette::ButtonText, text);
        p.setColor(QPalette::Highlight, sel);
        p.setColor(QPalette::HighlightedText, dark ? QColor(pal::text) : QColor("#0d1117"));
        p.setColor(QPalette::ToolTipBase, panel);
        p.setColor(QPalette::ToolTipText, text);
        p.setColor(QPalette::Mid, border);
        app.setPalette(p);
    }

    if (!gr_open(repo_path, dark ? 1 : 0)) {
        QLabel *err = new QLabel(QString("Could not open a git repository at:\n%1")
                                     .arg(QString::fromUtf8(repo_path)));
        err->setMargin(40);
        err->show();
        return app.exec();
    }

    AppCtx *ctx = new AppCtx();

    QMainWindow win;
    win.setWindowTitle(QString("git-review \xE2\x80\x94 %1").arg(takeRust(gr_repo_name())));
    win.resize(1280, 800);
    win.setStyleSheet(QString("QMainWindow{background:%1;} "
                              "QToolTip{background:%2; color:%3; border:1px solid %4;}")
                          .arg(pal::bg).arg(pal::panel).arg(pal::text).arg(pal::border));

    // ===== left side panel: nested splitters =====
    ctx->commitList = new QListWidget();
    ctx->commitList->setStyleSheet(QString(
        "QListWidget{background:%1; border:none;} "
        "QListWidget::item{border-bottom:1px solid %2;} "
        "QListWidget::item:selected{background:%3;}")
        .arg(pal::panel).arg(pal::border).arg(pal::selection));

    ctx->fileTree = new QTreeWidget();
    ctx->fileTree->setHeaderHidden(true);
    ctx->fileTree->setStyleSheet(QString(
        "QTreeWidget{background:%1; border:none;} "
        "QTreeWidget::item{padding:2px;}")
        .arg(pal::panel));

    QWidget *commitPanel = new QWidget();
    QVBoxLayout *cpl = new QVBoxLayout(commitPanel);
    cpl->setContentsMargins(0, 0, 0, 0);
    cpl->setSpacing(0);
    QLabel *commitsHdr = new QLabel("  Commits");
    commitsHdr->setStyleSheet(QString("font-weight:bold; color:%1; background:%2; padding:6px;")
                                  .arg(pal::muted).arg(pal::panel));
    cpl->addWidget(commitsHdr);
    cpl->addWidget(ctx->commitList);

    QWidget *filePanel = new QWidget();
    QVBoxLayout *fpl = new QVBoxLayout(filePanel);
    fpl->setContentsMargins(0, 0, 0, 0);
    fpl->setSpacing(0);
    QLabel *filesHdr = new QLabel("  Files");
    filesHdr->setStyleSheet(QString("font-weight:bold; color:%1; background:%2; padding:6px;")
                                .arg(pal::muted).arg(pal::panel));
    fpl->addWidget(filesHdr);
    fpl->addWidget(ctx->fileTree);

    QSplitter *leftSplit = new QSplitter(Qt::Vertical);
    leftSplit->addWidget(commitPanel);
    leftSplit->addWidget(filePanel);
    leftSplit->setSizes({450, 380});

    // ===== main view =====
    QWidget *mainView = new QWidget();
    QVBoxLayout *mvl = new QVBoxLayout(mainView);
    mvl->setContentsMargins(0, 0, 0, 0);
    mvl->setSpacing(0);

    // toolbar
    QToolBar *toolbar = new QToolBar();
    toolbar->setMovable(false);
    toolbar->setStyleSheet(QString("QToolBar{background:%1; border-bottom:1px solid %2; spacing:6px; padding:4px;}")
                               .arg(pal::panel).arg(pal::border));
    ctx->summaryLabel = new QLabel("git-review");
    ctx->summaryLabel->setStyleSheet(QString("font-family:monospace; font-weight:bold; color:%1; padding:0 8px;").arg(pal::text));
    toolbar->addWidget(ctx->summaryLabel);
    ctx->countsLabel = new QLabel();
    toolbar->addWidget(ctx->countsLabel);

    QWidget *spacer = new QWidget();
    spacer->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Preferred);
    toolbar->addWidget(spacer);

    auto makeTool = [&](const QString &text, const QString &tip, bool checkable) {
        QToolButton *b = new QToolButton();
        b->setText(text);
        b->setToolTip(tip);
        b->setCheckable(checkable);
        b->setCursor(Qt::PointingHandCursor);
        b->setStyleSheet(QString(
            "QToolButton{border:1px solid %1; border-radius:5px; padding:3px 8px; background:%2; color:%3;}"
            "QToolButton:hover{background:%5;}"
            "QToolButton:checked{background:%6; border-color:%4; color:%4;}")
            .arg(pal::border).arg(pal::bg).arg(pal::text).arg(pal::accent)
            .arg(pal::hover).arg(pal::selection));
        toolbar->addWidget(b);
        return b;
    };

    ctx->btnWrap = makeTool("\xE2\x86\xB5", "Word wrap", true);
    ctx->btnSpace = makeTool("\xE2\x90\xA3", "Show space changes", true);
    QToolButton *btnDec = makeTool("A-", "Decrease font size", false);
    QToolButton *btnInc = makeTool("A+", "Increase font size", false);
    ctx->btnLineNo = makeTool("#", "Toggle line numbers", true);

    ctx->btnSpace->setChecked(true);  // show by default
    ctx->btnLineNo->setChecked(true);

    // scrollable diff body
    ctx->diffScroll = new QScrollArea();
    ctx->diffScroll->setWidgetResizable(true);
    ctx->diffScroll->setStyleSheet(QString("QScrollArea{background:%1; border:none;}").arg(pal::bg));
    ctx->diffContainer = new QWidget();
    ctx->diffContainer->setStyleSheet(QString("background:%1;").arg(pal::bg));
    ctx->diffLayout = new QVBoxLayout(ctx->diffContainer);
    ctx->diffLayout->setContentsMargins(12, 12, 12, 12);
    ctx->diffLayout->setSpacing(16);
    ctx->diffScroll->setWidget(ctx->diffContainer);
    mvl->addWidget(ctx->diffScroll);

    // ===== outer horizontal splitter =====
    QSplitter *outer = new QSplitter(Qt::Horizontal);
    outer->addWidget(leftSplit);
    outer->addWidget(mainView);
    outer->setSizes({320, 960});
    outer->setStretchFactor(0, 0);
    outer->setStretchFactor(1, 1);
    win.setCentralWidget(outer);

    // Full-width toolbar pinned at the very top, spanning above both side panel and
    // main view (a top QToolBar in a QMainWindow spans the whole window width).
    toolbar->setMovable(false);
    toolbar->setFloatable(false);
    win.addToolBar(Qt::TopToolBarArea, toolbar);

    // ===== wiring =====
    ctx->reload = [ctx]() { reloadDiff(ctx); };

    // toolbar toggles
    QObject::connect(ctx->btnWrap, &QToolButton::toggled, [ctx](bool on) {
        ctx->settings.word_wrap = on;
        reloadDiff(ctx);
    });
    QObject::connect(ctx->btnSpace, &QToolButton::toggled, [ctx](bool on) {
        ctx->settings.show_space_changes = on;
        reloadDiff(ctx);
    });
    QObject::connect(ctx->btnLineNo, &QToolButton::toggled, [ctx](bool on) {
        ctx->settings.line_numbers = on;
        reloadDiff(ctx);
    });
    QObject::connect(btnDec, &QToolButton::clicked, [ctx]() {
        if (ctx->settings.font_size > 7) ctx->settings.font_size--;
        reloadDiff(ctx);
    });
    QObject::connect(btnInc, &QToolButton::clicked, [ctx]() {
        if (ctx->settings.font_size < 28) ctx->settings.font_size++;
        reloadDiff(ctx);
    });

    // file-tree click -> scroll to section
    QObject::connect(ctx->fileTree, &QTreeWidget::itemClicked,
                     [ctx](QTreeWidgetItem *item, int) {
        QString path = item->data(0, Qt::UserRole).toString();
        if (path.isEmpty()) return;
        for (auto &pr : ctx->fileSections) {
            if (pr.first == path) {
                ctx->diffScroll->ensureWidgetVisible(pr.second, 0, 0);
                break;
            }
        }
    });

    // commit-list click -> open commit / working tree
    QObject::connect(ctx->commitList, &QListWidget::itemClicked,
                     [ctx](QListWidgetItem *item) {
        QString oid = item->data(Qt::UserRole).toString();
        bool working = item->data(Qt::UserRole + 1).toBool();
        ctx->fromOid.clear();
        ctx->toOid.clear();
        if (working) {
            ctx->isWorking = true;
            ctx->single.clear();
        } else {
            ctx->isWorking = false;
            ctx->single = oid;
        }
        reloadDiff(ctx);
    });

    // ===== populate commit list =====
    QJsonDocument commitsDoc = parseRust(gr_commits());
    QJsonArray commits = commitsDoc.array();
    for (const QJsonValue &cv : commits) {
        QJsonObject c = cv.toObject();
        QListWidgetItem *item = new QListWidgetItem(ctx->commitList);
        item->setData(Qt::UserRole, c["oid"].toString());
        item->setData(Qt::UserRole + 1, c["working"].toBool());
        QWidget *row = makeCommitRow(ctx, c, ctx->commitList, item);
        ctx->commitList->setItemWidget(item, row);
    }

    // default selection: first real commit (git show) if any, else working tree.
    if (commits.size() >= 2) {
        ctx->commitList->setCurrentRow(1);
        ctx->single = commits[1].toObject()["oid"].toString();
        ctx->isWorking = false;
    } else {
        ctx->commitList->setCurrentRow(0);
        ctx->isWorking = true;
    }
    reloadDiff(ctx);

    win.show();
    return app.exec();
}
