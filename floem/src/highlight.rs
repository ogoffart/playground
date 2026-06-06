//! Syntax highlighting via `syntect` + `two-face`.
//!
//! Produces, for a line of source, a list of coloured spans `(rgb, text)`. Highlighting is done
//! per-line and statelessly (a fresh highlighter per line) so that interleaved added/removed
//! diff lines never corrupt each other's parse state. The trade-off is that constructs spanning
//! multiple lines (e.g. block comments) are not carried across lines — an acceptable
//! approximation for a diff viewer, and identical across every app in this repo.

use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Style, Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};

pub type Rgb = (u8, u8, u8);

#[derive(Clone)]
pub struct Span {
    pub color: Rgb,
    pub bold: bool,
    pub italic: bool,
    pub text: String,
}

pub struct Highlighter {
    syntaxes: SyntaxSet,
    theme: Theme,
}

impl Highlighter {
    pub fn new() -> Self {
        Self::with_theme(false)
    }

    /// Construct a highlighter using the dark syntect theme (`base16-ocean.dark`).
    pub fn with_dark() -> Self {
        Self::with_theme(true)
    }

    fn with_theme(dark: bool) -> Self {
        let syntaxes = two_face::syntax::extra_newlines();
        let themes = ThemeSet::load_defaults();
        let theme = if dark {
            themes
                .themes
                .get("base16-ocean.dark")
                .cloned()
                .unwrap_or_else(|| themes.themes["Solarized (dark)"].clone())
        } else {
            // A light, GitHub-like theme.
            themes
                .themes
                .get("InspiredGitHub")
                .cloned()
                .unwrap_or_else(|| themes.themes["Solarized (light)"].clone())
        };
        Self { syntaxes, theme }
    }

    /// Pick a syntax by file path/extension, falling back to plain text.
    pub fn syntax_for<'a>(&'a self, path: &str) -> &'a SyntaxReference {
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        self.syntaxes
            .find_syntax_by_extension(ext)
            .or_else(|| {
                let name = std::path::Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                self.syntaxes.find_syntax_by_token(name)
            })
            .unwrap_or_else(|| self.syntaxes.find_syntax_plain_text())
    }

    /// Highlight a single line. `syntax` should come from [`Self::syntax_for`].
    pub fn line(&self, syntax: &SyntaxReference, text: &str) -> Vec<Span> {
        if text.is_empty() {
            return Vec::new();
        }
        let mut h = HighlightLines::new(syntax, &self.theme);
        match h.highlight_line(text, &self.syntaxes) {
            Ok(ranges) => ranges
                .into_iter()
                .map(|(style, piece)| to_span(style, piece))
                .collect(),
            Err(_) => vec![Span {
                color: (31, 35, 40),
                bold: false,
                italic: false,
                text: text.to_string(),
            }],
        }
    }
}

fn to_span(style: Style, text: &str) -> Span {
    Span {
        color: (style.foreground.r, style.foreground.g, style.foreground.b),
        bold: style.font_style.contains(FontStyle::BOLD),
        italic: style.font_style.contains(FontStyle::ITALIC),
        text: text.to_string(),
    }
}
