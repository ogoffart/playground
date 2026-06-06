// Tiny C++ helper to detect the desktop light/dark colour scheme.
//
// Qt 6.5 added Qt::ColorScheme / QStyleHints::colorScheme(), but this app targets Qt 6.4,
// where that API does not exist. Instead we read the application palette that the active
// platform theme installed at startup: if the default Window background is darker than the
// window text colour, the desktop is using a dark scheme. In a headless environment (no
// desktop theme) Qt falls back to the light Fusion/default palette, so this returns false
// (light), matching the spec's "headless => light" rule.
#include <QtGui/QGuiApplication>
#include <QtGui/QPalette>
#include <QtGui/QColor>

extern "C" bool git_review_is_dark() {
    const QPalette pal = QGuiApplication::palette();
    const QColor bg = pal.color(QPalette::Active, QPalette::Window);
    const QColor fg = pal.color(QPalette::Active, QPalette::WindowText);
    // Lightness in [0,255]; dark scheme => background notably darker than text.
    return bg.lightness() < fg.lightness();
}
