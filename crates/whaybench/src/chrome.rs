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
pub const NAME_CHARS: usize = 44;

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
    /// The theme's title_font, title_weight (100 to 900, as CSS) and title_spacing (pixels added after each letter).
    family: String,
    weight: f32,
    spacing: f32,
    texts: HashMap<(String, [u8; 3]), Text>,
    widths: HashMap<String, f64>,
}

fn load(pattern: &str, weight: f32, why: &str) -> Option<FontVec> {
    // The face's index too: a collection (Noto Sans CJK's .ttc) holds a face per region, and face 0 is not the one asked.
    let out = Command::new("fc-match").args(["-f", "%{index}:%{file}", pattern]).stderr(Stdio::null()).output().ok()?;
    let out = String::from_utf8_lossy(&out.stdout).to_string();
    let (index, path) = out.split_once(':')?;
    // fontconfig keeps a variable font's named instance in the upper 16 bits; the weight is set below instead.
    let index = index.parse::<u32>().unwrap_or(0) & 0xffff;
    let path = path.to_string();
    let mut font = std::fs::read(&path).ok().and_then(|b| FontVec::try_from_vec_and_index(b, index).ok())?;
    // A variable font (Noto Sans CJK VF, the "sans" of Korean desktops) opens at its lightest, which reads grey at
    // title size: the weight the pattern asks. A font without that axis is left as it is.
    font.set_variation(b"wght", weight);
    record::decide("screen", "font", why, &[("file", path)]);
    Some(font)
}

/// fontconfig's name for the weight nearest a CSS weight.
fn weight_name(weight: f32) -> &'static str {
    const NAMES: [&str; 9] = ["thin", "extralight", "light", "regular", "medium", "semibold", "bold", "extrabold", "black"];
    NAMES[((weight / 100.0).round() as usize).clamp(1, 9) - 1]
}

/// A theme's family list as a fontconfig pattern: families joined by commas, its special characters escaped.
fn families(font: &str) -> String {
    font.split(',').map(|f| f.trim().replace(|c: char| c == '\'' || c == '"' || c.is_control(), "").replace('\\', "\\\\").replace('-', "\\-").replace(':', "\\:"))
        .filter(|f| !f.is_empty()).collect::<Vec<_>>().join(",")
}

fn find_font(family: &str, weight: f32) -> Option<FontVec> {
    // The desktop's sans when the theme's font cannot be looked up, so the title bars never lose their text.
    let font = [families(family), "sans".to_string()].into_iter().filter(|f| !f.is_empty())
        .find_map(|fam| load(&format!("{fam}:{}", weight_name(weight)), weight, "the title bar font").or_else(|| load(&fam, weight, "the title bar font")));
    if font.is_none() { record::warn("screen", "no font", "the title bar shows no text; install a font", &[]); }
    font
}

/// The first font that has the character; the title font (its box) when none has.
fn pick(fonts: &[FontVec], c: char) -> usize {
    fonts.iter().position(|f| f.glyph_id(c).0 != 0).unwrap_or(0)
}

impl Chrome {
    pub fn new(size: f64, family: &str, weight: f64, spacing: f64) -> Self {
        Chrome { fonts: find_font(family, weight as f32).into_iter().collect(), looked: HashSet::new(), size: size as f32,
            family: family.to_string(), weight: weight as f32, spacing: spacing as f32, texts: HashMap::new(), widths: HashMap::new() }
    }

    /// A new text size, font, weight or spacing: what was made before goes; a new font or weight is looked up again.
    pub fn set_style(&mut self, size: f64, family: &str, weight: f64, spacing: f64) {
        let (size, weight, spacing) = (size as f32, weight as f32, spacing as f32);
        if self.size == size && self.family == family && self.weight == weight && self.spacing == spacing { return; }
        if self.family != family || self.weight != weight {
            self.fonts = find_font(family, weight).into_iter().collect();
            self.looked.clear();
            self.family = family.to_string();
            self.weight = weight;
        }
        self.size = size;
        self.spacing = spacing;
        self.texts.clear();
        self.widths.clear();
    }

    /// Loads a font for each character the loaded ones lack, asking fontconfig as GTK's fallback does.
    fn cover(&mut self, text: &str) {
        if self.fonts.is_empty() { return; }
        for c in text.chars() {
            if c.is_control() || self.fonts.iter().any(|f| f.glyph_id(c).0 != 0) || !self.looked.insert(c) { continue; }
            // The theme's families first: a Hangul font named after a Latin one is the one asked for Hangul.
            let pattern = format!("{}sans:{}:charset={:x}", families(&self.family).split(',').filter(|f| !f.is_empty()).map(|f| format!("{f},")).collect::<String>(), weight_name(self.weight), c as u32);
            if let Some(font) = load(&pattern, self.weight, "a fallback title font").filter(|f| f.glyph_id(c).0 != 0) {
                self.fonts.push(font);
            }
        }
    }

    pub fn width(&mut self, text: &str) -> f64 {
        if let Some(w) = self.widths.get(text) { return *w; }
        if self.fonts.is_empty() { return 0.0; }
        self.cover(text);
        let w = measure(&self.fonts, self.size, self.spacing, text);
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
            let made = raster(&self.fonts, self.size, self.spacing, text, colour)?;
            self.texts.insert(key.clone(), made);
        }
        self.texts.get(&key)
    }
}

fn measure(fonts: &[FontVec], size: f32, spacing: f32, text: &str) -> f64 {
    let mut x = 0.0;
    let mut last = None;
    for c in text.chars() {
        let i = pick(fonts, c);
        let f = fonts[i].as_scaled(PxScale::from(size));
        let g = f.glyph_id(c);
        // Kerning is only between glyphs of one font.
        if let Some((li, l)) = last && li == i { x += f.kern(l, g); }
        x += f.h_advance(g) + spacing;
        last = Some((i, g));
    }
    // Spacing goes between letters, not after the last one, so the text stays in the middle of its tab.
    if last.is_some() { x -= spacing; }
    x as f64
}

fn raster(fonts: &[FontVec], size: f32, spacing: f32, text: &str, colour: [u8; 3]) -> Option<Text> {
    let w = measure(fonts, size, spacing, text).ceil() as i32 + 2;
    if w <= 2 { return None; }
    let h = (size * 1.4).ceil() as i32;
    let mut bytes = vec![0u8; (w * h * 4) as usize];
    // The letters' middle on the picture's middle: a capital's height above the baseline, as a title bar centres its
    // text by eye. The font's ascent and descent would centre its whole line box, and CJK fonts give that box more
    // room above, which set the text low.
    let first = fonts[0].as_scaled(PxScale::from(size));
    let cap = fonts[0].outline_glyph(first.glyph_id('H').with_scale_and_position(PxScale::from(size), point(0.0, 0.0)))
        .map_or(first.ascent() * 0.7, |g| -g.px_bounds().min.y);
    let base = ((h as f32 + cap) / 2.0).round();
    let (mut x, mut last) = (1.0, None);
    for c in text.chars() {
        let i = pick(fonts, c);
        let f = fonts[i].as_scaled(PxScale::from(size));
        let id = f.glyph_id(c);
        if let Some((li, l)) = last && li == i { x += f.kern(l, id); }
        let glyph = id.with_scale_and_position(PxScale::from(size), point(x, base));
        x += f.h_advance(id) + spacing;
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
        // A Latin-only title font; skipped where this machine has none.
        let Some(latin) = ["DejaVu Sans:bold", "Adwaita Sans:bold", "Cantarell:bold"].iter()
            .find_map(|p| load(p, 700.0, "test").filter(|f| f.glyph_id('a').0 != 0 && f.glyph_id('\u{AC00}').0 == 0)) else { return };
        let mut chrome = Chrome { fonts: vec![latin], looked: HashSet::new(), size: 14.0, family: "sans".into(), weight: 700.0, spacing: 0.0, texts: HashMap::new(), widths: HashMap::new() };
        chrome.cover("a\u{AC00}");
        if chrome.fonts.len() == 1 { return; } // no font with Hangul installed
        assert_eq!(pick(&chrome.fonts, 'a'), 0);
        assert_eq!(pick(&chrome.fonts, '\u{AC00}'), 1);
    }

    #[test]
    fn title_style_names_and_spacing() {
        assert_eq!((weight_name(300.0), weight_name(700.0), weight_name(20.0), weight_name(990.0)), ("light", "bold", "thin", "black"));
        assert_eq!(families("Pretendard Light, 'Noto Sans-CJK'"), "Pretendard Light,Noto Sans\\-CJK");
        let Some(font) = load("sans", 400.0, "test") else { return };
        let fonts = vec![font];
        let (plain, spaced) = (measure(&fonts, 14.0, 0.0, "abc"), measure(&fonts, 14.0, 2.0, "abc"));
        assert!((spaced - plain - 4.0).abs() < 1e-3);
        assert_eq!(families("Bad\u{0}Name"), "BadName");
        // A family fontconfig cannot be asked for still leaves the title a font.
        assert!(find_font("\u{0}", 700.0).is_some());
    }

    #[test]
    fn a_face_inside_a_collection_opens() {
        // fontconfig gives Noto Sans CJK KR as face 1 of a .ttc, with a named instance in the upper bits.
        let found = Command::new("fc-match").args(["-f", "%{family}", "Noto Sans CJK KR:bold"]).output().map(|o| o.stdout).unwrap_or_default();
        if !String::from_utf8_lossy(&found).contains("Noto Sans CJK KR") { return; }
        assert!(load("Noto Sans CJK KR:bold", 700.0, "test").is_some_and(|kr| kr.glyph_id('\u{AC00}').0 != 0));
    }

    #[test]
    fn names_are_cut_at_the_limit() {
        assert_eq!(cut_name("short"), "short");
        let cut = cut_name(&"é".repeat(NAME_CHARS + 8));
        assert_eq!(cut.chars().count(), NAME_CHARS + 1);
        assert!(cut.ends_with('…'));
    }
}
