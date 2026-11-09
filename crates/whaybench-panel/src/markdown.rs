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

fn text_label(markup: &str) -> gtk::Label {
    let l = gtk::Label::new(None);
    l.set_markup(markup);
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_selectable(true);
    l
}

/// Fills `into` with the document.
pub fn show(md: &str, into: &gtk::Box) {
    for b in blocks(md) {
        match b {
            Block::Text { markup, class, indent } => {
                let l = text_label(&markup);
                l.set_margin_start(indent * 16);
                if !class.is_empty() { l.add_css_class(class); }
                into.append(&l);
            }
            Block::Code(text) => {
                let l = gtk::Label::new(Some(&text));
                l.set_xalign(0.0);
                l.set_selectable(true);
                l.add_css_class("code");
                let scroll = gtk::ScrolledWindow::new();
                scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
                scroll.set_propagate_natural_height(true);
                scroll.set_child(Some(&l));
                into.append(&scroll);
            }
            Block::Table(rows) => {
                let grid = gtk::Grid::new();
                for (y, row) in rows.iter().enumerate() {
                    for (x, cell) in row.iter().enumerate() {
                        let l = text_label(cell);
                        l.add_css_class("cell");
                        if y == 0 { l.add_css_class("head"); }
                        l.set_hexpand(true);
                        grid.attach(&l, x as i32, y as i32, 1, 1);
                    }
                }
                into.append(&grid);
            }
            Block::Rule => into.append(&gtk::Separator::new(gtk::Orientation::Horizontal)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn text(markup: &str, class: &'static str, indent: i32) -> Block { Block::Text { markup: markup.into(), class, indent } }
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
