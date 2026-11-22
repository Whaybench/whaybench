//! What the panels' web pages share: Markdown as HTML, the theme's colours and fonts as CSS, and the address of
//! a link that was pressed.
use gtk::prelude::*;
use pulldown_cmark::{html, Event, Options, Parser, Tag, TagEnd};
use wb_core::{paths::Data, theme};

pub fn escape(t: &str) -> String { t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;") }

/// What happens to HTML written in the Markdown: a document of the person's keeps it, an answer shows it as text.
#[derive(Clone, Copy, PartialEq)]
pub enum RawHtml { Kept, AsText, Untrusted }

pub fn markdown(md: &str, raw: RawHtml) -> String {
    let md = crate::common::table_pipes(md);
    // The address of the link being read (Untrusted): written after its words, as text.
    let mut dest = String::new();
    let events = Parser::new_ext(&md, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS).filter_map(move |ev| Some(match ev {
        Event::Html(t) | Event::InlineHtml(t) if raw != RawHtml::Kept => Event::Text(t),
        // Text someone else wrote (a Briefsafe project's): its links and images are their words only. Nothing it names
        // is followed, fetched or handed to another program from here (a web address would reach the host's views);
        // a link's address is shown as text after it, so the reader still sees where it points.
        Event::Start(Tag::Link { dest_url, .. }) if raw == RawHtml::Untrusted => { dest = dest_url.to_string(); return None; }
        Event::End(TagEnd::Link) if raw == RawHtml::Untrusted => {
            let d = std::mem::take(&mut dest);
            // Only a web address: others (javascript:, file:, the panel's own) say nothing a reader needs.
            if !(d.to_lowercase().starts_with("http://") || d.to_lowercase().starts_with("https://")) { return None; }
            Event::Text(format!(" ({d})").into())
        }
        Event::Start(Tag::Image { .. }) | Event::End(TagEnd::Image) if raw == RawHtml::Untrusted => return None,
        other => other,
    }));
    let mut out = String::new();
    html::push_html(&mut out, events);
    out
}

/// The active theme as CSS values.
pub struct Page { pub bg: String, pub ink: String, pub accent: String, pub line: String, pub surface: String, pub font: String, pub mono: String }

impl Page {
    pub fn of(d: &Data) -> Self {
        let t = theme::load_active(d);
        let hex = theme::hex;
        Page {
            // A see-through panel shows its own fill under the page.
            bg: if t.see_through() { "transparent".into() } else { hex(t.panel) }, ink: theme::ink_hex(t.ink), accent: hex(t.accent), line: hex(t.border), surface: hex(t.surface),
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

/// Whether the navigation the web view asks about comes from the person's own press (not a page's script, refresh
/// or frame): a page whose text is someone else's acts only on those.
pub fn by_person(decision: &webkit6::PolicyDecision) -> bool {
    decision.downcast_ref::<webkit6::NavigationPolicyDecision>().and_then(|n| n.navigation_action()).is_some_and(|a| a.is_user_gesture())
}

#[cfg(test)]
mod tests {
    use super::*;
    use RawHtml::{AsText, Kept, Untrusted};
    #[test]
    fn untrusted_markdown_links_only_to_web_pages_and_fetches_nothing() {
        let h = markdown("[a](javascript:alert(1)) [b](WB:bs:close) [c](file:///etc/passwd) [d](HTTPS://example.org) ![e](http://x/y.png) <b>x</b>", Untrusted);
        assert!(!h.to_lowercase().contains("javascript:") && !h.to_lowercase().contains("wb:") && !h.contains("file:"), "{h}");
        assert!(!h.contains("<a ") && !h.contains("<img"), "no link, no image: {h}");
        assert!(h.contains("&lt;b&gt;") && h.contains('d') && h.contains('e'), "their words stay: {h}");
        assert!(h.contains("d (HTTPS://example.org)"), "a web address is shown as text: {h}");
    }

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
