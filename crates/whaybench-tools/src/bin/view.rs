//! What the AI uses to show things in the view panel and to see what the person sees.
#[path = "view/a11y.rs"]
mod a11y;

use std::{io::Read, path::Path, process::Command};

use serde_json::{json, Value};
use wb_core::{paths::{self, Data}, sessions, view::{self, Link}};

const HELP: &str = "The view panel is where everything you show the person opens, and nowhere else. It goes with your session: when
the person comes back to this session, your views are there. Never open anything outside it: no xdg-open, gio open,
kde-open, browsers, viewers or other desktop apps.
Work on an app through its connector or API; use this to show things and to see where the person is. You read, you do not click.

  whaybench-view open [--firefox] PATH|URL|- [PAGE]   show it in the view panel, a PDF at PAGE; a log, archive or binary
                                           opens with its usual tool; '-' reads HTML from standard input. A web address
                                           (http, https) opens as a new tab of the browser inside Whaybench (a real Firefox
                                           in a panel beside this one), where the person's sign-ins are; --firefox opens a
                                           file there too. Never a browser outside Whaybench
  whaybench-view edit FILE                 open FILE in vim in a terminal in the view place
  whaybench-view read [--all] [--firefox]  what the person sees, as text, with the address and any selected sentences;
                                           --all: the whole page; --firefox: the Firefox tab instead of the view panel.
                                           Use this first: pictures cost far more
  whaybench-view where [--text]            the page the person is on in a PDF opened with --firefox; --text adds its text
                                           (read works on a PDF in the view panel too, but gives the whole file's text)
  whaybench-view shot [--out FILE.png] [--firefox]   a picture of the panel, only when text cannot tell; prints the file's path
  whaybench-view new [--title T] [--session|--pin] [PATH|URL]   one more view. Default: linked to the chosen session;
                                           --session: stays on this session; --pin: stays on this page or file
  whaybench-view show NAME PATH|URL        show it in that view
  whaybench-view list                      the open views and what each is tied to
  whaybench-view close NAME                close a view (not the first)
Show a drawing by writing HTML to show/<your session id>/latest.html in the data directory: it opens by itself.";

fn absolute(p: &str) -> String {
    if p.starts_with('/') { return p.to_string(); }
    std::env::current_dir().unwrap_or_default().join(p).to_string_lossy().to_string()
}

/// The session asking: from Claude Code, or from the conversation tab this runs in.
fn session(d: &Data) -> Option<String> {
    let id = ["WHAYBENCH_SESSION_ID", "CLAUDE_CODE_SESSION_ID", "CLAUDE_SESSION_ID"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .or_else(|| sessions::session_of_tab(d, &std::env::var("WHAYBENCH_TAB").ok()?))?;
    Some(sessions::current_id(d, &id))
}

/// A path as an absolute one that exists, an address as it is.
fn resolve_target(target: &str) -> Result<String, String> {
    if view::is_web(target) { return Ok(target.to_string()); }
    let a = absolute(target);
    if Path::new(&a).exists() { Ok(a) } else { Err(wb_core::i18n::f("file-not-found", &[("a", (a).to_string())])) }
}

fn flag(args: &[String], name: &str) -> bool { args.iter().any(|a| a == name) }

fn link_text(l: &Link) -> String {
    match l { Link::Follow => wb_core::i18n::t("linked-to-session-tab").into(), Link::Session(s) => wb_core::i18n::f("session", &[("s", (s).to_string())]), Link::Target(p, t) => wb_core::i18n::f("pinned-view", &[("t", (t).to_string()), ("p", (p).to_string())]) }
}

fn print_read(v: &Value) {
    if let Some(e) = v.get("error").and_then(Value::as_str) { println!("{}", wb_core::i18n::f("read-failed-2", &[("e", (e).to_string())])); return; }
    if let Some(l) = v.get("link").and_then(Value::as_str) { println!("{l}"); }
    println!("{}", wb_core::i18n::f("address", &[("a1", (v["url"].as_str().unwrap_or("")).to_string())]));
    println!("{}", wb_core::i18n::f("title", &[("a1", (v["title"].as_str().unwrap_or("")).to_string())]));
    if let Some(p) = v.get("page").and_then(Value::as_str) { println!("{}", wb_core::i18n::f("page", &[("p", (p).to_string()), ("a1", (v["pages"].as_str().unwrap_or("?")).to_string())])); }
    let sel: Vec<&str> = v["selected"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if !sel.is_empty() { println!("{}", wb_core::i18n::f("selected-text", &[("a1", (sel.join("\n")).to_string())])); }
    println!("{}", wb_core::i18n::f("text", &[("a1", (v["text"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join("\n")).to_string())]));
}

fn run(args: Vec<String>, d: &Data) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("open") => {
            let firefox = flag(&args, "--firefox");
            let rest: Vec<&String> = args[1..].iter().filter(|a| *a != "--firefox").collect();
            let target = rest.first().ok_or(HELP)?.as_str();
            let page = rest.get(1).and_then(|p| p.parse().ok());
            let who = session(d);
            let what = if target == "-" {
                let dir = d.root.join("show").join(who.as_deref().unwrap_or("shared")).join("views");
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let mut html = String::new();
                std::io::stdin().read_to_string(&mut html).map_err(|e| e.to_string())?;
                let f = dir.join(format!("{}.html", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|t| t.as_millis()).unwrap_or(0)));
                std::fs::write(&f, html).map_err(|e| e.to_string())?;
                f.to_string_lossy().to_string()
            } else { resolve_target(target)? };
            // A web address opens in the browser tab, where the person's sign-ins are; the view keeps pages and files.
            if firefox || what.starts_with("http://") || what.starts_with("https://") {
                view::open_in_firefox(d, &what, page).map_err(|e| e.to_string())?;
                println!("{}", wb_core::i18n::f("open-requested-in-firefox", &[("what", (what).to_string())]));
                return Ok(());
            }
            match &who {
                Some(s) => view::open_for_session(d, s, &what, page),
                None => view::open(d, &what, page),
            }.map_err(|e| e.to_string())?;
            println!("{}", wb_core::i18n::f("open-requested-in-the-view-panel", &[("what", (what).to_string())]));
            Ok(())
        }
        Some("new") => {
            let mut title = None;
            let mut target = None;
            let mut it = args[1..].iter();
            while let Some(a) = it.next() {
                match a.as_str() { "--title" => title = it.next().cloned(), "--session" | "--pin" => {}, other => target = Some(resolve_target(other)?) }
            }
            let link = if flag(&args, "--pin") {
                let t = target.clone().ok_or(wb_core::i18n::t("pin-requires-an-address-or-file"))?;
                let name = title.clone().unwrap_or_else(|| Path::new(&t).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| t.clone()));
                Link::Target(t, name)
            } else if flag(&args, "--session") {
                Link::Session(session(d).ok_or(wb_core::i18n::t("calling-session-unknown"))?)
            } else { Link::Follow };
            let exe = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name("whaybench-panel");
            let name = view::new_view(d, &exe, &link).map_err(|e| e.to_string())?;
            if let (Some(t), false) = (&target, matches!(link, Link::Target(..))) { view::open_in_view(d, &name, t, None).map_err(|e| e.to_string())?; }
            println!("{name}");
            Ok(())
        }
        Some("show") => {
            let (name, target) = (args.get(1).ok_or(HELP)?, resolve_target(args.get(2).ok_or(HELP)?)?);
            if !view::views(d).iter().any(|(n, _)| n == name) { return Err(wb_core::i18n::f("no-view", &[("name", (name).to_string())])); }
            view::open_in_view(d, name, &target, None).map_err(|e| e.to_string())
        }
        Some("list") => {
            for (name, link) in view::views(d) { println!("{name}\t{}\t{}", view::panel_title(&name), link_text(&link)); }
            Ok(())
        }
        Some("close") => {
            let name = args.get(1).ok_or(HELP)?;
            if name == view::FIRST_VIEW { return Err(wb_core::i18n::t("the-first-view-cannot-be-closed").into()); }
            if !view::views(d).iter().any(|(n, _)| n == name) { return Err(wb_core::i18n::f("no-view", &[("name", (name).to_string())])); }
            view::ask(d, &view::for_view(name), &json!({"do": "close"})).map_err(|e| e.to_string())
        }
        Some("edit") => { let a = absolute(args.get(1).ok_or(HELP)?); view::vim(d, &a).map_err(|e| e.to_string()) }
        Some("read") => {
            let all = flag(&args, "--all");
            let read = if flag(&args, "--firefox") {
                let mut v = a11y::read_text(d, all)?;
                v["link"] = serde_json::json!(wb_core::i18n::ai("view-link-firefox"));
                v
            } else { view::read_panel(d, view::FIRST_VIEW, all)? };
            print_read(&read);
            Ok(())
        }
        Some("where") => {
            // Only Firefox says which page the person is on; the view panel's PDF viewer does not.
            let v = a11y::read_text(d, false).map_err(|e| wb_core::i18n::f("for-the-page-number-open-firefox", &[("e", (e).to_string())]))?;
            let url = v["url"].as_str().unwrap_or("");
            if v.get("page").is_none() { return Err(wb_core::i18n::t("no-pdf-in-firefox-for-the").into()); }
            println!("{}", wb_core::i18n::f("page-2", &[("url", (url).to_string()), ("a1", (v["page"].as_str().unwrap_or("?")).to_string()), ("a2", (v["pages"].as_str().unwrap_or("?")).to_string())]));
            if args.iter().any(|a| a == "--text") {
                let path = url.strip_prefix("file://").unwrap_or("").split('#').next().unwrap_or("").to_string();
                let page = v["page"].as_str().unwrap_or("1").to_string();
                if path.is_empty() { return Err(wb_core::i18n::t("not-a-pdf-opened-from-a").into()); }
                let o = Command::new("pdftotext").args(["-f", &page, "-l", &page, "-layout", &path, "-"]).output().map_err(|e| e.to_string())?;
                print!("{}", String::from_utf8_lossy(&o.stdout));
            }
            Ok(())
        }
        Some("shot") => {
            let out = args.iter().position(|a| a == "--out").and_then(|i| args.get(i + 1)).map(|a| absolute(a)).unwrap_or_else(|| d.cache().join("view-shot.png").to_string_lossy().to_string());
            let exe = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name("whaybench");
            let o = Command::new(exe).args(["ctl", "--data-dir", &d.root.to_string_lossy(), "shot", "--out", &out, "--app", if flag(&args, "--firefox") { view::FIREFOX_APP } else { view::VIEW_APP }]).output().map_err(|e| e.to_string())?;
            if !o.status.success() { return Err(String::from_utf8_lossy(&o.stderr).trim().to_string()); }
            print!("{}", String::from_utf8_lossy(&o.stdout));
            Ok(())
        }
        _ => Err(HELP.into()),
    }
}

fn main() {
    paths::private_files();
    let Some(root) = paths::resolve(None, |k| std::env::var(k).ok()) else { eprintln!("error: no data directory"); std::process::exit(1) };
    if let Err(e) = run(std::env::args().skip(1).collect(), &Data::new(root)) { eprintln!("{e}"); std::process::exit(1); }
}
