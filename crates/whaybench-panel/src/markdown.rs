//! An answer as widgets: paragraphs, headings, lists, code blocks and tables, without a web view.
use gtk::{glib, prelude::*};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// A block of the document, independent of the widgets, so it can be tested.
#[derive(Debug, PartialEq)]
pub enum Block {
    Text { markup: String, class: &'static str, indent: i32 },
    Code(String),
    Table(Vec<Vec<String>>),
    Rule,
}

pub fn blocks(md: &str) -> Vec<Block> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut code: Option<String> = None;
    let (mut lists, mut quote): (Vec<Option<u64>>, i32) = (Vec::new(), 0);
    let mut class = "";
    let (mut table, mut row, mut cell): (Vec<Vec<String>>, Vec<String>, String) = Default::default();
    let mut in_table = false;
    let mut item_start = false;
    let flush = |buf: &mut String, out: &mut Vec<Block>, class: &'static str, indent: i32| {
        if !buf.trim().is_empty() { out.push(Block::Text { markup: buf.trim_end().to_string(), class, indent }); }
        buf.clear();
    };
    for ev in Parser::new_ext(md, opts) {
        let indent = lists.len() as i32 + quote;
        let target: &mut String = if in_table { &mut cell } else { &mut buf };
        match ev {
            Event::Start(Tag::Heading { level, .. }) => class = match level { HeadingLevel::H1 => "h1", HeadingLevel::H2 => "h2", _ => "h3" },
            Event::End(TagEnd::Heading(_)) => { flush(&mut buf, &mut out, class, indent); class = ""; }
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => flush(&mut buf, &mut out, "", indent),
            Event::Start(Tag::BlockQuote(_)) => quote += 1,
            Event::End(TagEnd::BlockQuote(_)) => quote -= 1,
            Event::Start(Tag::List(first)) => { flush(&mut buf, &mut out, "", indent); lists.push(first); }
            Event::End(TagEnd::List(_)) => { flush(&mut buf, &mut out, "", indent); lists.pop(); }
            Event::Start(Tag::Item) => {
                flush(&mut buf, &mut out, "", indent);
                let mark = match lists.last_mut() { Some(Some(n)) => { let m = format!("{n}. "); *n += 1; m } _ => "• ".to_string() };
                buf.push_str(&mark);
                item_start = true;
            }
            Event::End(TagEnd::Item) => { flush(&mut buf, &mut out, "", indent); item_start = false; }
            Event::Start(Tag::CodeBlock(kind)) => { flush(&mut buf, &mut out, "", indent); let _ = matches!(kind, CodeBlockKind::Fenced(_)); code = Some(String::new()); }
            Event::End(TagEnd::CodeBlock) => if let Some(c) = code.take() { out.push(Block::Code(c.trim_end().to_string())); },
            Event::Start(Tag::Table(_)) => { flush(&mut buf, &mut out, "", indent); in_table = true; table.clear(); }
            Event::End(TagEnd::Table) => { in_table = false; out.push(Block::Table(std::mem::take(&mut table))); }
            Event::Start(Tag::TableHead) | Event::Start(Tag::TableRow) => row.clear(),
            Event::End(TagEnd::TableHead) | Event::End(TagEnd::TableRow) => table.push(std::mem::take(&mut row)),
            Event::Start(Tag::TableCell) => cell.clear(),
            Event::End(TagEnd::TableCell) => row.push(std::mem::take(&mut cell).trim().to_string()),
            Event::Start(Tag::Strong) => target.push_str("<b>"),
            Event::End(TagEnd::Strong) => target.push_str("</b>"),
            Event::Start(Tag::Emphasis) => target.push_str("<i>"),
            Event::End(TagEnd::Emphasis) => target.push_str("</i>"),
            Event::Start(Tag::Strikethrough) => target.push_str("<s>"),
            Event::End(TagEnd::Strikethrough) => target.push_str("</s>"),
            Event::Start(Tag::Link { dest_url, .. }) => target.push_str(&format!("<a href=\"{}\">", glib::markup_escape_text(&dest_url))),
            Event::End(TagEnd::Link) => target.push_str("</a>"),
            Event::Text(t) => match &mut code { Some(c) => c.push_str(&t), None => target.push_str(&glib::markup_escape_text(&t)) },
            Event::Code(t) => target.push_str(&format!("<tt>{}</tt>", glib::markup_escape_text(&t))),
            Event::SoftBreak => target.push(' '),
            Event::HardBreak => target.push('\n'),
            Event::Html(t) | Event::InlineHtml(t) => target.push_str(&glib::markup_escape_text(&t)),
            Event::Rule => { flush(&mut buf, &mut out, "", indent); out.push(Block::Rule); }
            _ => {}
        }
    }
    let _ = item_start;
    flush(&mut buf, &mut out, "", 0);
    out
}

/// A table as lines of text, each column as wide as its widest cell, so it can be selected with the rest.
pub fn table_lines(rows: &[Vec<String>]) -> Vec<String> {
    let plain: Vec<Vec<String>> = rows.iter().map(|r| r.iter().map(|c| strip_markup(c)).collect()).collect();
    let cols = plain.iter().map(Vec::len).max().unwrap_or(0);
    let width = |x: usize| plain.iter().filter_map(|r| r.get(x)).map(|c| c.chars().map(cell_width).sum::<usize>()).max().unwrap_or(0);
    let widths: Vec<usize> = (0..cols).map(width).collect();
    plain.iter().map(|r| (0..cols).map(|x| {
        let c = r.get(x).map(String::as_str).unwrap_or("");
        let pad = widths[x] - c.chars().map(cell_width).sum::<usize>();
        format!("{c}{}", " ".repeat(pad))
    }).collect::<Vec<_>>().join("  │  ").trim_end().to_string()).collect()
}

/// Wide characters (Hangul, CJK) take two columns of a monospace font.
fn cell_width(c: char) -> usize { if ('\u{1100}'..='\u{11FF}').contains(&c) || ('\u{2E80}'..='\u{A4CF}').contains(&c) || ('\u{AC00}'..='\u{D7A3}').contains(&c) || ('\u{F900}'..='\u{FAFF}').contains(&c) || ('\u{FF00}'..='\u{FF60}').contains(&c) { 2 } else { 1 } }

fn strip_markup(m: &str) -> String {
    let mut out = String::new();
    let mut tag = false;
    for c in m.chars() { match c { '<' => tag = true, '>' if tag => tag = false, _ if !tag => out.push(c), _ => {} } }
    out.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&")
}

/// Markup split at its links: (markup, link address) pieces, since a text view's markup has no links.
fn pieces(markup: &str) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    let mut rest = markup;
    while let Some(at) = rest.find("<a href=\"") {
        out.push((rest[..at].to_string(), None));
        let after = &rest[at + 9..];
        let Some(q) = after.find("\">") else { break };
        let Some(end) = after.find("</a>") else { break };
        out.push((after[q + 2..end].to_string(), Some(strip_markup(&after[..q]))));
        rest = &after[end + 4..];
    }
    out.push((rest.to_string(), None));
    out.into_iter().filter(|(m, _)| !m.is_empty()).collect()
}

/// Fills `into` with the document as one read-only text view: the whole answer selects and copies as one text.
pub fn show(md: &str, into: &gtk::Box) {
    let view = gtk::TextView::new();
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.add_css_class("answer");
    let buf = view.buffer();
    let tag = |name: &str, f: &dyn Fn(&gtk::TextTag)| { let t = gtk::TextTag::new(Some(name)); f(&t); buf.tag_table().add(&t); };
    tag("h1", &|t| { t.set_scale(1.45); t.set_weight(700); });
    tag("h2", &|t| { t.set_scale(1.25); t.set_weight(700); });
    tag("h3", &|t| { t.set_scale(1.1); t.set_weight(700); });
    tag("code", &|t| { t.set_family(Some("monospace")); t.set_left_margin(12); });
    tag("head", &|t| { t.set_weight(700); });
    tag("rule", &|t| { t.set_foreground(Some("gray")); });
    tag("gap", &|t| { t.set_pixels_below_lines(8); });
    let mut it = buf.end_iter();
    let mut links = 0;
    let all = blocks(md);
    let last = all.len().saturating_sub(1);
    for (i, b) in all.into_iter().enumerate() {
        let start = buf.create_mark(None, &it, true);
        match b {
            Block::Text { markup, class, indent } => {
                for (m, link) in pieces(&markup) {
                    let from = buf.create_mark(None, &it, true);
                    buf.insert_markup(&mut it, &m);
                    if let Some(url) = link {
                        links += 1;
                        let t = gtk::TextTag::new(Some(&format!("link{links}\u{1}{url}")));
                        t.set_underline(gtk::pango::Underline::Single);
                        t.set_foreground(Some("#1e66f5"));
                        buf.tag_table().add(&t);
                        buf.apply_tag(&t, &buf.iter_at_mark(&from), &it);
                    }
                    buf.delete_mark(&from);
                }
                let s = buf.iter_at_mark(&start);
                if !class.is_empty() { buf.apply_tag_by_name(class, &s, &it); }
                if indent > 0 {
                    let t = gtk::TextTag::new(None);
                    t.set_left_margin(indent * 16);
                    buf.tag_table().add(&t);
                    buf.apply_tag(&t, &buf.iter_at_mark(&start), &it);
                }
            }
            Block::Code(text) => { buf.insert(&mut it, &text); buf.apply_tag_by_name("code", &buf.iter_at_mark(&start), &it); }
            Block::Table(rows) => {
                for (y, line) in table_lines(&rows).iter().enumerate() {
                    if y > 0 { buf.insert(&mut it, "\n"); }
                    let from = buf.create_mark(None, &it, true);
                    buf.insert(&mut it, line);
                    if y == 0 { buf.apply_tag_by_name("head", &buf.iter_at_mark(&from), &it); }
                    buf.delete_mark(&from);
                }
                buf.apply_tag_by_name("code", &buf.iter_at_mark(&start), &it);
            }
            Block::Rule => { buf.insert(&mut it, "────────"); buf.apply_tag_by_name("rule", &buf.iter_at_mark(&start), &it); }
        }
        if i < last { buf.insert(&mut it, "\n"); }
        buf.apply_tag_by_name("gap", &buf.iter_at_mark(&start), &it);
        buf.delete_mark(&start);
    }
    // A press on a link opens it, unless the press was the end of a selection.
    let click = gtk::GestureClick::new();
    {
        let view = view.clone();
        click.connect_released(move |_, _, x, y| {
            if view.buffer().has_selection() { return; }
            let (bx, by) = view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
            let Some(at) = view.iter_at_location(bx, by) else { return };
            let Some(url) = at.tags().iter().filter_map(|t| t.name()).find_map(|n| n.split_once('\u{1}').map(|(_, u)| u.to_string())) else { return };
            gtk::UriLauncher::new(&url).launch(None::<&gtk::Window>, None::<&gtk::gio::Cancellable>, |_| {});
        });
    }
    view.add_controller(click);
    into.append(&view);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn text(markup: &str, class: &'static str, indent: i32) -> Block { Block::Text { markup: markup.into(), class, indent } }
    #[test]
    fn tables_line_up_and_links_split_out() {
        let rows = vec![vec!["ＡＢ".to_string(), "b".to_string()], vec!["a <b>x</b>".to_string(), "longer".to_string()]];
        assert_eq!(table_lines(&rows), ["ＡＢ  │  b", "a x   │  longer"], "a wide character is two columns");
        assert_eq!(pieces("see <a href=\"https://a.b/?x=1&amp;y\">here</a> now"), [("see ".to_string(), None), ("here".to_string(), Some("https://a.b/?x=1&y".to_string())), (" now".to_string(), None)]);
    }
    #[test]
    fn paragraphs_headings_inline() {
        assert_eq!(blocks("# Title\n\ntext **bold** `code` <tag>\n"), [text("Title", "h1", 0), text("text <b>bold</b> <tt>code</tt> &lt;tag&gt;", "", 0)]);
    }
    #[test]
    fn lists_keep_their_marks_and_depth() {
        assert_eq!(blocks("- a\n- b\n\n1. one\n2. two\n"), [text("• a", "", 1), text("• b", "", 1), text("1. one", "", 1), text("2. two", "", 1)]);
    }
    #[test]
    fn code_blocks_are_not_markup() {
        assert_eq!(blocks("```\n<b>as is</b>\n```\n"), [Block::Code("<b>as is</b>".into())]);
    }
    #[test]
    fn tables_come_as_rows() {
        assert_eq!(blocks("| a | b |\n|---|---|\n| **1** | 2 |\n"), [Block::Table(vec![vec!["a".into(), "b".into()], vec!["<b>1</b>".into(), "2".into()]])]);
    }
}
