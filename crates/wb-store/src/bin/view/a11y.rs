//! What the panel's Firefox shows, read through the accessibility bus (AT-SPI over D-Bus), as a screen reader does:
//! the text on screen (or the whole page), the selected sentences, the address, and the page of a PDF. GIO's D-Bus
//! client, which the panels already build on, makes the calls.
use gio::glib::{Variant, prelude::*};
use serde_json::{Value, json};
use wb_core::paths::Data;

const ACC: &str = "org.a11y.atspi.Accessible";
const TEXT: &str = "org.a11y.atspi.Text";
const ROLE_DOCUMENT_WEB: u32 = 95;
const ROLE_SPIN_BUTTON: u32 = 52;
const STATE_SHOWING: u32 = 25;
/// Deeper than any page the tree walk needs; it stops a looping or huge tree.
const DEPTH: usize = 200;

/// An accessible object: its bus name and object path.
type Node = (String, String);

struct Bus(gio::DBusConnection);

impl Bus {
    fn call(&self, n: &Node, iface: &str, method: &str, args: Variant) -> Option<Variant> {
        self.0.call_sync(Some(&n.0), &n.1, iface, method, Some(&args), None, gio::DBusCallFlags::NONE, 3000, gio::Cancellable::NONE).ok()
    }
    fn prop(&self, n: &Node, iface: &str, name: &str) -> Option<Variant> {
        self.call(n, "org.freedesktop.DBus.Properties", "Get", (iface, name).to_variant())?.child_value(0).as_variant()
    }
    fn children(&self, n: &Node) -> Vec<Node> {
        let Some(v) = self.call(n, ACC, "GetChildren", ().to_variant()) else { return Vec::new() };
        v.child_value(0).iter().map(|c| (c.child_value(0).str().unwrap_or("").to_string(), c.child_value(1).str().unwrap_or("").to_string()))
            .filter(|c| !c.0.is_empty() && !c.1.ends_with("/null")).collect()
    }
    fn role(&self, n: &Node) -> u32 { self.call(n, ACC, "GetRole", ().to_variant()).and_then(|v| v.child_value(0).get::<u32>()).unwrap_or(0) }
    fn showing(&self, n: &Node) -> bool {
        let states = self.call(n, ACC, "GetState", ().to_variant()).and_then(|v| v.child_value(0).get::<Vec<u32>>()).unwrap_or_default();
        states.get((STATE_SHOWING / 32) as usize).is_some_and(|w| w >> (STATE_SHOWING % 32) & 1 == 1)
    }
    fn has_text(&self, n: &Node) -> bool {
        self.call(n, ACC, "GetInterfaces", ().to_variant()).and_then(|v| v.child_value(0).get::<Vec<String>>()).is_some_and(|i| i.iter().any(|x| x == TEXT))
    }
    /// The text between two offsets, without the placeholders that stand for embedded objects.
    fn text(&self, n: &Node, from: i32, to: i32) -> String {
        let t = self.call(n, TEXT, "GetText", (from, to).to_variant()).and_then(|v| v.child_value(0).str().map(String::from)).unwrap_or_default();
        t.replace('\u{fffc}', "").trim().to_string()
    }
}

struct Found { texts: Vec<(String, bool)>, selected: Vec<String>, entries: Vec<String> }

fn documents(bus: &Bus, n: &Node, depth: usize, out: &mut Vec<Node>) {
    if bus.role(n) == ROLE_DOCUMENT_WEB { out.push(n.clone()); return; }
    if depth < 30 { for c in bus.children(n) { documents(bus, &c, depth + 1, out); } }
}

fn collect(bus: &Bus, n: &Node, depth: usize, f: &mut Found) {
    if depth > DEPTH { return; }
    if bus.has_text(n) {
        let count = bus.prop(n, TEXT, "CharacterCount").and_then(|v| v.get::<i32>()).unwrap_or(0);
        let t = bus.text(n, 0, count);
        if !t.is_empty() {
            if bus.role(n) == ROLE_SPIN_BUTTON { f.entries.push(t.clone()); }
            f.texts.push((t, bus.showing(n)));
        }
        let picks = bus.call(n, TEXT, "GetNSelections", ().to_variant()).and_then(|v| v.child_value(0).get::<i32>()).unwrap_or(0);
        for k in 0..picks {
            let Some((a, b)) = bus.call(n, TEXT, "GetSelection", (k,).to_variant()).and_then(|v| v.get::<(i32, i32)>()) else { continue };
            let s = bus.text(n, a, b);
            if !s.is_empty() { f.selected.push(s); }
        }
    }
    for c in bus.children(n) { collect(bus, &c, depth + 1, f); }
}

/// The accessibility bus, whose address the session bus gives.
fn connect() -> Result<Bus, String> {
    let session = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).map_err(|e| e.to_string())?;
    let address = session.call_sync(Some("org.a11y.Bus"), "/org/a11y/bus", "org.a11y.Bus", "GetAddress", None, None, gio::DBusCallFlags::NONE, 3000, gio::Cancellable::NONE)
        .map_err(|e| e.to_string())?.child_value(0).str().unwrap_or("").to_string();
    let flags = gio::DBusConnectionFlags::AUTHENTICATION_CLIENT | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION;
    gio::DBusConnection::for_address_sync(&address, flags, None::<&gio::DBusAuthObserver>, gio::Cancellable::NONE).map(Bus).map_err(|e| e.to_string())
}

/// What the panel's Firefox shows, as text. `all`: the whole page, not only what is on screen.
pub fn read_text(d: &Data, all: bool) -> Result<Value, String> {
    let pid = wb_core::view::firefox_pid(d).ok_or(wb_core::i18n::t("no-firefox-in-the-view-panel"))?;
    let bus = connect()?;
    let root = ("org.a11y.atspi.Registry".to_string(), "/org/a11y/atspi/accessible/root".to_string());
    let pid_of = |name: &str| bus.0.call_sync(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", "org.freedesktop.DBus", "GetConnectionUnixProcessID",
        Some(&(name,).to_variant()), None, gio::DBusCallFlags::NONE, 3000, gio::Cancellable::NONE).ok().and_then(|v| v.child_value(0).get::<u32>());
    let Some(app) = bus.children(&root).into_iter().find(|a| pid_of(&a.0) == Some(pid)) else { return Ok(json!({"error": "Firefox not on the accessibility bus"})) };
    let mut docs = Vec::new();
    documents(&bus, &app, 0, &mut docs);
    let Some(doc) = docs.into_iter().find(|n| bus.showing(n)) else { return Ok(json!({"error": "no page showing"})) };
    let mut f = Found { texts: Vec::new(), selected: Vec::new(), entries: Vec::new() };
    collect(&bus, &doc, 0, &mut f);
    let attrs = bus.call(&doc, "org.a11y.atspi.Document", "GetAttributes", ().to_variant()).map(|v| v.child_value(0)).into_iter().flat_map(|a| a.iter().collect::<Vec<_>>())
        .map(|e| (e.child_value(0).str().unwrap_or("").to_string(), e.child_value(1).str().unwrap_or("").to_string())).collect::<Vec<_>>();
    let url = attrs.iter().find(|(k, _)| k == "DocURL").map(|(_, v)| v.clone()).unwrap_or_default();
    let title = bus.prop(&doc, ACC, "Name").and_then(|v| v.str().map(String::from)).unwrap_or_default();
    let mut out = json!({"url": url, "title": title, "selected": f.selected, "text": f.texts.iter().filter(|(_, shown)| all || *shown).map(|(t, _)| t).collect::<Vec<_>>()});
    if url.to_lowercase().split('#').next().unwrap_or("").ends_with(".pdf") {
        if let Some(page) = f.entries.first() {
            out["page"] = json!(page);
            out["pages"] = json!(f.texts.iter().find(|(t, _)| t.starts_with("/ ") || t.starts_with("of ")).and_then(|(t, _)| t.trim_start_matches(['/', ' ']).split_whitespace().last()).unwrap_or(""));
        }
    }
    Ok(out)
}
