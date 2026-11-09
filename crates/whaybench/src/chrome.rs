//! Text for what the compositor draws itself: title bars, tabs, drop previews. These must keep working
//! when a panel's app has stopped, so they do not go through any app.
use std::{collections::HashMap, process::{Command, Stdio}};

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};
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
    font: Option<FontVec>,
    /// Text size in pixels (the theme's title_size).
    size: f32,
    texts: HashMap<(String, [u8; 3]), Text>,
    widths: HashMap<String, f64>,
}

fn find_font() -> Option<FontVec> {
    for pattern in ["sans:bold", "sans"] {
        let Ok(out) = Command::new("fc-match").args(["-f", "%{file}", pattern]).stderr(Stdio::null()).output() else { continue };
        let path = String::from_utf8_lossy(&out.stdout).to_string();
        if let Some(font) = std::fs::read(&path).ok().and_then(|b| FontVec::try_from_vec(b).ok()) {
            record::decide("screen", "font", "the title bar font", &[("file", path)]);
            return Some(font);
        }
    }
    record::warn("screen", "no font", "the title bar shows no text; install a font", &[]);
    None
}

impl Chrome {
    pub fn new(size: f64) -> Self { Chrome { font: find_font(), size: size as f32, texts: HashMap::new(), widths: HashMap::new() } }

    /// A new text size: what was made at the old one goes.
    pub fn set_size(&mut self, size: f64) {
        if self.size == size as f32 { return; }
        self.size = size as f32;
        self.texts.clear();
        self.widths.clear();
    }

    pub fn width(&mut self, text: &str) -> f64 {
        if let Some(w) = self.widths.get(text) { return *w; }
        let Some(font) = &self.font else { return 0.0 };
        let w = measure(font, self.size, text);
        if self.widths.len() > 512 { self.widths.clear(); }
        self.widths.insert(text.to_string(), w);
        w
    }

    /// The text as a buffer the renderer can draw, made once per text and colour.
    pub fn text(&mut self, text: &str, colour: [u8; 3]) -> Option<&Text> {
        let font = self.font.as_ref()?;
        let key = (text.to_string(), colour);
        if !self.texts.contains_key(&key) {
            if self.texts.len() > 256 { self.texts.clear(); }
            let made = raster(font, self.size, text, colour)?;
            self.texts.insert(key.clone(), made);
        }
        self.texts.get(&key)
    }
}

fn measure(font: &FontVec, size: f32, text: &str) -> f64 {
    let f = font.as_scaled(PxScale::from(size));
    let mut x = 0.0;
    let mut last = None;
    for c in text.chars() {
        let g = f.glyph_id(c);
        if let Some(l) = last { x += f.kern(l, g); }
        x += f.h_advance(g);
        last = Some(g);
    }
    x as f64
}

fn raster(font: &FontVec, size: f32, text: &str, colour: [u8; 3]) -> Option<Text> {
    let w = measure(font, size, text).ceil() as i32 + 2;
    if w <= 2 { return None; }
    let h = (size * 1.4).ceil() as i32;
    let mut bytes = vec![0u8; (w * h * 4) as usize];
    let f = font.as_scaled(PxScale::from(size));
    let base = ((h as f32 + f.ascent() + f.descent()) / 2.0).round();
    let (mut x, mut last) = (1.0, None);
    for c in text.chars() {
        let id = f.glyph_id(c);
        if let Some(l) = last { x += f.kern(l, id); }
        let glyph = id.with_scale_and_position(PxScale::from(size), point(x, base));
        x += f.h_advance(id);
        last = Some(id);
        let Some(outline) = font.outline_glyph(glyph) else { continue };
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
    fn names_are_cut_at_32_characters() {
        assert_eq!(cut_name("short"), "short");
        let cut = cut_name(&"é".repeat(40));
        assert_eq!(cut.chars().count(), 33);
        assert!(cut.ends_with('…'));
    }
}
