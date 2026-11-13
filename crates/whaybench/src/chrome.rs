//! Text for what the compositor draws itself: title bars, tabs, drop previews. These must keep working
//! when a panel's app has stopped, so they do not go through any app.
use std::{collections::{HashMap, HashSet}, process::{Command, Stdio}};

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, VariableFont, point};
use smithay::{
    backend::{allocator::Fourcc, renderer::element::memory::MemoryRenderBuffer},
    utils::Transform,
};
use wb_core::log as record;

/// Tab titles are cut here.
pub const NAME_CHARS: usize = 32;

pub fn cut_name(name: &str) -> String {
    if name.chars().count() > NAME_CHARS { format!("{}…", name.chars().take(NAME_CHARS).collect::<String>()) } else { name.to_string() }
}

pub struct Text { pub buffer: MemoryRenderBuffer, pub w: i32, pub h: i32 }

pub struct Chrome {
    /// The title font first, then fonts fontconfig gave for characters it lacks (Hangul when "sans" is Latin only).
    fonts: Vec<FontVec>,
    /// Characters a fallback font was already looked for.
    looked: HashSet<char>,
    /// Text size in pixels (the theme's title_size).
    size: f32,
    texts: HashMap<(String, [u8; 3]), Text>,
    widths: HashMap<String, f64>,
}

fn load(pattern: &str, why: &str) -> Option<FontVec> {
    let out = Command::new("fc-match").args(["-f", "%{file}", pattern]).stderr(Stdio::null()).output().ok()?;
    let path = String::from_utf8_lossy(&out.stdout).to_string();
    let mut font = std::fs::read(&path).ok().and_then(|b| FontVec::try_from_vec(b).ok())?;
    // A variable font (Noto Sans CJK VF, the "sans" of Korean desktops) opens at its lightest, which reads grey at
    // title size: bold, as the pattern asks. A font without that axis is left as it is.
    if pattern.contains(":bold") { font.set_variation(b"wght", 700.0); }
    record::decide("screen", "font", why, &[("file", path)]);
    Some(font)
}

fn find_font() -> Option<FontVec> {
    let font = load("sans:bold", "the title bar font").or_else(|| load("sans", "the title bar font"));
    if font.is_none() { record::warn("screen", "no font", "the title bar shows no text; install a font", &[]); }
    font
}

/// The first font that has the character; the title font (its box) when none has.
fn pick(fonts: &[FontVec], c: char) -> usize {
    fonts.iter().position(|f| f.glyph_id(c).0 != 0).unwrap_or(0)
}

impl Chrome {
    pub fn new(size: f64) -> Self { Chrome { fonts: find_font().into_iter().collect(), looked: HashSet::new(), size: size as f32, texts: HashMap::new(), widths: HashMap::new() } }

    /// A new text size: what was made at the old one goes.
    pub fn set_size(&mut self, size: f64) {
        if self.size == size as f32 { return; }
        self.size = size as f32;
        self.texts.clear();
        self.widths.clear();
    }

    /// Loads a font for each character the loaded ones lack, asking fontconfig as GTK's fallback does.
    fn cover(&mut self, text: &str) {
        if self.fonts.is_empty() { return; }
        for c in text.chars() {
            if c.is_control() || self.fonts.iter().any(|f| f.glyph_id(c).0 != 0) || !self.looked.insert(c) { continue; }
            if let Some(font) = load(&format!("sans:bold:charset={:x}", c as u32), "a fallback title font").filter(|f| f.glyph_id(c).0 != 0) {
                self.fonts.push(font);
            }
        }
    }

    pub fn width(&mut self, text: &str) -> f64 {
        if let Some(w) = self.widths.get(text) { return *w; }
        if self.fonts.is_empty() { return 0.0; }
        self.cover(text);
        let w = measure(&self.fonts, self.size, text);
        if self.widths.len() > 512 { self.widths.clear(); }
        self.widths.insert(text.to_string(), w);
        w
    }

    /// The text as a buffer the renderer can draw, made once per text and colour.
    pub fn text(&mut self, text: &str, colour: [u8; 3]) -> Option<&Text> {
        if self.fonts.is_empty() { return None; }
        let key = (text.to_string(), colour);
        if !self.texts.contains_key(&key) {
            if self.texts.len() > 256 { self.texts.clear(); }
            self.cover(text);
            let made = raster(&self.fonts, self.size, text, colour)?;
            self.texts.insert(key.clone(), made);
        }
        self.texts.get(&key)
    }
}

fn measure(fonts: &[FontVec], size: f32, text: &str) -> f64 {
    let mut x = 0.0;
    let mut last = None;
    for c in text.chars() {
        let i = pick(fonts, c);
        let f = fonts[i].as_scaled(PxScale::from(size));
        let g = f.glyph_id(c);
        // Kerning is only between glyphs of one font.
        if let Some((li, l)) = last && li == i { x += f.kern(l, g); }
        x += f.h_advance(g);
        last = Some((i, g));
    }
    x as f64
}

fn raster(fonts: &[FontVec], size: f32, text: &str, colour: [u8; 3]) -> Option<Text> {
    let w = measure(fonts, size, text).ceil() as i32 + 2;
    if w <= 2 { return None; }
    let h = (size * 1.4).ceil() as i32;
    let mut bytes = vec![0u8; (w * h * 4) as usize];
    let first = fonts[0].as_scaled(PxScale::from(size));
    let base = ((h as f32 + first.ascent() + first.descent()) / 2.0).round();
    let (mut x, mut last) = (1.0, None);
    for c in text.chars() {
        let i = pick(fonts, c);
        let f = fonts[i].as_scaled(PxScale::from(size));
        let id = f.glyph_id(c);
        if let Some((li, l)) = last && li == i { x += f.kern(l, id); }
        let glyph = id.with_scale_and_position(PxScale::from(size), point(x, base));
        x += f.h_advance(id);
        last = Some((i, id));
        let Some(outline) = fonts[i].outline_glyph(glyph) else { continue };
        let b = outline.px_bounds();
        outline.draw(|gx, gy, cover| {
            let (px, py) = (gx as i32 + b.min.x as i32, gy as i32 + b.min.y as i32);
            if px < 0 || py < 0 || px >= w || py >= h { return; }
            let at = ((py * w + px) * 4) as usize;
            let a = (cover.clamp(0.0, 1.0) * 255.0) as u32;
            if a <= bytes[at + 3] as u32 { return; }
            // Argb8888 in memory is B G R A; the renderer wants premultiplied alpha.
            bytes[at] = (colour[2] as u32 * a / 255) as u8;
            bytes[at + 1] = (colour[1] as u32 * a / 255) as u8;
            bytes[at + 2] = (colour[0] as u32 * a / 255) as u8;
            bytes[at + 3] = a as u8;
        });
    }
    let buffer = MemoryRenderBuffer::from_slice(&bytes, Fourcc::Argb8888, (w, h), 1, Transform::Normal, None);
    Some(Text { buffer, w, h })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_character_the_title_font_lacks_comes_from_a_fallback_font() {
        // A Latin-only title font, as "sans" is on Ubuntu under WSL; skipped where this machine has none.
        let Some(latin) = ["DejaVu Sans:bold", "Adwaita Sans:bold", "Cantarell:bold"].iter()
            .find_map(|p| load(p, "test").filter(|f| f.glyph_id('a').0 != 0 && f.glyph_id('\u{AC00}').0 == 0)) else { return };
        let mut chrome = Chrome { fonts: vec![latin], looked: HashSet::new(), size: 14.0, texts: HashMap::new(), widths: HashMap::new() };
        chrome.cover("a\u{AC00}");
        if chrome.fonts.len() == 1 { return; } // no font with Hangul installed
        assert_eq!(pick(&chrome.fonts, 'a'), 0);
        assert_eq!(pick(&chrome.fonts, '\u{AC00}'), 1);
    }

    #[test]
    fn names_are_cut_at_32_characters() {
        assert_eq!(cut_name("short"), "short");
        let cut = cut_name(&"é".repeat(40));
        assert_eq!(cut.chars().count(), 33);
        assert!(cut.ends_with('…'));
    }
}
