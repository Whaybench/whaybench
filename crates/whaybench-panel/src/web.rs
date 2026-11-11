//! What the panels' web pages share: Markdown as HTML, the theme's colours and fonts as CSS, and the address of
//! a link that was pressed.
use gtk::prelude::*;
use pulldown_cmark::{html, Event, Options, Parser};
use wb_core::{paths::Data, theme};

pub fn escape(t: &str) -> String { t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;") }

/// What happens to HTML written in the Markdown: a document of the person's keeps it, an answer shows it as text.
#[derive(Clone, Copy, PartialEq)]
pub enum RawHtml { Kept, AsText }

pub fn markdown(md: &str, raw: RawHtml) -> String {
    let md = crate::common::table_pipes(md);
    let events = Parser::new_ext(&md, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS).map(|ev| match ev {
        Event::Html(t) | Event::InlineHtml(t) if raw == RawHtml::AsText => Event::Text(t),
        other => other,
    });
    let mut out = String::new();
    html::push_html(&mut out, events);
    out
}

/// The active theme as CSS values.
pub struct Page { pub bg: String, pub ink: String, pub accent: String, pub line: String, pub surface: String, pub font: String, pub mono: String }

impl Page {
    pub fn of(d: &Data) -> Self {
        let t = theme::load_active(d);
        let hex = |c: [f32; 4]| format!("#{:02x}{:02x}{:02x}", (c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8);
        Page {
            bg: hex(t.panel), ink: format!("#{:02x}{:02x}{:02x}", t.ink[0], t.ink[1], t.ink[2]), accent: hex(t.accent), line: hex(t.border), surface: hex(t.surface),
            font: t.font.replace('\'', ""), mono: t.terminal_font.replace('\'', ""),
        }
    }
    /// The rules every page starts from; `size` is the body's font size and line height, as in `14px/1.55`.
    /// A panel adds its own spacing and sizes after these.
    pub fn base(&self, size: &str) -> String {
        format!("html,body{{margin:0;background:{};color:{};font:{size} '{}',sans-serif}} table{{border-collapse:collapse}} td,th{{border:1px solid {}}} \
                 pre,code{{font-family:'{}',monospace}} pre{{white-space:pre-wrap}} ", self.bg, self.ink, self.font, self.line, self.mono)
    }
}

/// The address a press on a link asks for, when the web view asks what to do with it. None: not a link press.
pub fn pressed_link(decision: &webkit6::PolicyDecision, kind: webkit6::PolicyDecisionType) -> Option<String> {
    if kind != webkit6::PolicyDecisionType::NavigationAction && kind != webkit6::PolicyDecisionType::NewWindowAction { return None; }
    let nav = decision.downcast_ref::<webkit6::NavigationPolicyDecision>()?;
    Some(nav.navigation_action().and_then(|a| a.request()).and_then(|r| r.uri()).map(|u| u.to_string()).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use RawHtml::{AsText, Kept};
    #[test]
    fn paragraphs_headings_inline() {
        assert_eq!(markdown("# Title\n\ntext **bold** `code`\n", AsText), "<h1>Title</h1>\n<p>text <strong>bold</strong> <code>code</code></p>\n");
    }
    #[test]
    fn lists_are_lists() {
        assert_eq!(markdown("- a\n- b\n\n1. one\n", AsText), "<ul>\n<li>a</li>\n<li>b</li>\n</ul>\n<ol>\n<li>one</li>\n</ol>\n");
    }
    #[test]
    fn tables_are_tables() {
        let page = markdown("| a | b |\n|---|---|\n| **1** | `x | y` |\n", AsText);
        assert!(page.starts_with("<table><thead><tr><th>a</th><th>b</th></tr></thead>"), "{page}");
        assert!(page.contains("<td><strong>1</strong></td><td><code>x | y</code></td>"), "a pipe in code stays in its cell: {page}");
    }
    #[test]
    fn html_in_an_answer_is_text_and_in_a_document_is_kept() {
        assert_eq!(markdown("a <script>x</script> b\n", AsText), "<p>a &lt;script&gt;x&lt;/script&gt; b</p>\n");
        assert_eq!(markdown("<div onclick=\"x\">\nhi\n</div>\n", AsText), "&lt;div onclick=\"x\"&gt;\nhi\n&lt;/div&gt;\n");
        assert_eq!(markdown("```\n<b>as is</b>\n```\n", AsText), "<pre><code>&lt;b&gt;as is&lt;/b&gt;\n</code></pre>\n");
        assert_eq!(markdown("a <b>x</b>\n", Kept), "<p>a <b>x</b></p>\n");
    }
}
