//! An answer as HTML for the answer panel's web view: paragraphs, headings, lists, code blocks and tables.
use pulldown_cmark::{html, Event, Options, Parser};

/// The answer's Markdown as HTML. HTML written in the answer is shown as text, not run.
pub fn to_html(md: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    let events = Parser::new_ext(md, opts).map(|ev| match ev {
        Event::Html(t) | Event::InlineHtml(t) => Event::Text(t),
        other => other,
    });
    let mut out = String::new();
    html::push_html(&mut out, events);
    out
}

pub fn escape(t: &str) -> String { t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;") }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paragraphs_headings_inline() {
        assert_eq!(to_html("# Title\n\ntext **bold** `code`\n"), "<h1>Title</h1>\n<p>text <strong>bold</strong> <code>code</code></p>\n");
    }
    #[test]
    fn lists_are_lists() {
        assert_eq!(to_html("- a\n- b\n\n1. one\n"), "<ul>\n<li>a</li>\n<li>b</li>\n</ul>\n<ol>\n<li>one</li>\n</ol>\n");
    }
    #[test]
    fn tables_are_tables() {
        let page = to_html("| a | b |\n|---|---|\n| **1** | 2 |\n");
        assert!(page.starts_with("<table><thead><tr><th>a</th><th>b</th></tr></thead>"), "{page}");
        assert!(page.contains("<td><strong>1</strong></td><td>2</td>"), "{page}");
    }
    #[test]
    fn html_in_the_answer_is_text() {
        assert_eq!(to_html("a <script>x</script> b\n"), "<p>a &lt;script&gt;x&lt;/script&gt; b</p>\n");
        assert_eq!(to_html("<div onclick=\"x\">\nhi\n</div>\n"), "&lt;div onclick=\"x\"&gt;\nhi\n&lt;/div&gt;\n");
        assert_eq!(to_html("```\n<b>as is</b>\n```\n"), "<pre><code>&lt;b&gt;as is&lt;/b&gt;\n</code></pre>\n");
    }
}
