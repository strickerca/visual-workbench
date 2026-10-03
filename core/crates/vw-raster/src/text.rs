use crate::RasterError;
use rustybuzz::{Direction, Face, UnicodeBuffer, ttf_parser};
use serde::{Deserialize, Serialize};
use unicode_script::{Script, UnicodeScript};

pub const MAX_TEXT_BYTES: usize = 65536;
pub const MAX_GLYPHS: usize = 32768;
pub const MAX_OUTLINE_COMMANDS: usize = 1_000_000;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FontFamily {
    Inter,
    NotoSans,
    NotoSansMono,
}
impl FontFamily {
    pub fn parse(name: &str) -> Result<Self, RasterError> {
        match name {
            "Inter" => Ok(Self::Inter),
            "Noto Sans" => Ok(Self::NotoSans),
            "Noto Sans Mono" => Ok(Self::NotoSansMono),
            _ => Err(RasterError::Unsupported(
                "font; bundled Inter, Noto Sans, Noto Sans Mono only",
            )),
        }
    }
    pub fn bytes(self) -> &'static [u8] {
        match self {
            Self::Inter => include_bytes!("../fonts/Inter.ttf"),
            Self::NotoSans => include_bytes!("../fonts/NotoSans.ttf"),
            Self::NotoSansMono => include_bytes!("../fonts/NotoSansMono.ttf"),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OutlineCommand {
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Glyph {
    pub glyph_id: u32,
    /// UTF-8 byte offset in original logical text, including previous newlines.
    pub cluster: usize,
    pub font: FontFamily,
    pub x: f32,
    pub y: f32,
    pub advance: f32,
    pub outline: Vec<OutlineCommand>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextLayout {
    pub algorithm_version: u32,
    pub glyphs: Vec<Glyph>,
    pub width: f32,
    pub height: f32,
    pub line_height: f32,
}
fn quantize(value: f32) -> f32 {
    (libm::roundf(value * 256.0)) / 256.0
}
fn face(font: FontFamily) -> Result<Face<'static>, RasterError> {
    Face::from_slice(font.bytes(), 0).ok_or(RasterError::Invalid("bundled font"))
}
fn invisible(c: char) -> bool {
    matches!(c,'\u{200c}'|'\u{200d}'|'\u{200e}'|'\u{200f}'|'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}'|'\u{feff}'|'\u{fe00}'..='\u{fe0f}')
}
fn choose_font(text: &str, preferred: FontFamily) -> Result<FontFamily, RasterError> {
    // Keep each script run in one font. Character-by-character fallback can
    // detach inherited marks and joining controls from their base glyphs.
    for font in [preferred, FontFamily::NotoSans, FontFamily::NotoSansMono] {
        let face = face(font)?;
        if text
            .chars()
            .all(|c| invisible(c) || face.glyph_index(c).is_some())
        {
            return Ok(font);
        }
    }
    for c in text.chars().filter(|c| !invisible(*c)) {
        let mut supported = false;
        for font in [preferred, FontFamily::NotoSans, FontFamily::NotoSansMono] {
            supported |= face(font)?.glyph_index(c).is_some();
        }
        if !supported {
            return Err(RasterError::MissingGlyph(c as u32));
        }
    }
    Err(RasterError::Unsupported(
        "no bundled font covers the complete shaping run",
    ))
}
struct Outline {
    commands: Vec<OutlineCommand>,
    scale: f32,
    x: f32,
    y: f32,
    overflow: bool,
}
impl Outline {
    fn p(&self, x: f32, y: f32) -> (f32, f32) {
        (
            quantize(self.x + x * self.scale),
            quantize(self.y - y * self.scale),
        )
    }
    fn push(&mut self, c: OutlineCommand) {
        if self.commands.len() < MAX_OUTLINE_COMMANDS {
            self.commands.push(c);
        } else {
            self.overflow = true;
        }
    }
}
impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.p(x, y);
        self.push(OutlineCommand::Move(x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.p(x, y);
        self.push(OutlineCommand::Line(x, y));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (a, b) = self.p(x1, y1);
        let (c, d) = self.p(x, y);
        self.push(OutlineCommand::Quad(a, b, c, d));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (a, b) = self.p(x1, y1);
        let (c, d) = self.p(x2, y2);
        let (e, f) = self.p(x, y);
        self.push(OutlineCommand::Cubic(a, b, c, d, e, f));
    }
    fn close(&mut self) {
        self.push(OutlineCommand::Close);
    }
}

/// Layout starts at a baseline origin (0,0), with explicit newlines and no wrapping.
/// UAX#9 visual runs, script/font itemization, Rustybuzz OpenType shaping and bundled
/// default variation axes. Both apps consume these exact 1/256-px outline positions.
/// Missing scripts/glyphs fail explicitly instead of consulting device-local fonts.
pub fn layout_text(text: &str, font: FontFamily, size: f32) -> Result<TextLayout, RasterError> {
    if text.len() > MAX_TEXT_BYTES || !size.is_finite() || !(0.25..=4096.0).contains(&size) {
        return Err(RasterError::Invalid("text size/length"));
    }
    if text.chars().any(|c| c.is_control() && c != '\n') {
        return Err(RasterError::Unsupported(
            "text control; normalize tabs and CRLF explicitly",
        ));
    }
    let line_height = quantize(size * 1.25);
    let mut result = TextLayout {
        algorithm_version: 1,
        glyphs: Vec::new(),
        width: 0.0,
        height: 0.0,
        line_height,
    };
    let mut logical_offset = 0;
    let mut command_count = 0usize;
    for (line_index, line) in text.split('\n').enumerate() {
        let bidi = unicode_bidi::BidiInfo::new(line, None);
        let mut pen_x = 0.0;
        let baseline = quantize(line_index as f32 * line_height);
        for para in &bidi.paragraphs {
            let (levels, runs) = bidi.visual_runs(para, para.range.clone());
            for run in runs {
                let rtl = levels[run.start].is_rtl();
                let slice = &line[run.clone()];
                let mut segments: Vec<(usize, usize, Script)> = Vec::new();
                for (offset, c) in slice.char_indices() {
                    let script = c.script();
                    let pos = run.start + offset;
                    if let Some(last) = segments.last_mut()
                        && (last.2 == script
                            || matches!(script, Script::Common | Script::Inherited)
                            || matches!(last.2, Script::Common | Script::Inherited))
                    {
                        if !matches!(script, Script::Common | Script::Inherited) {
                            last.2 = script;
                        }
                        last.1 = pos + c.len_utf8();
                        continue;
                    }
                    segments.push((pos, pos + c.len_utf8(), script));
                }
                if rtl {
                    segments.reverse();
                }
                for (start, end, _) in segments {
                    let font = choose_font(&line[start..end], font)?;
                    let face = face(font)?;
                    let scale = size / face.units_per_em() as f32;
                    let mut buffer = UnicodeBuffer::new();
                    buffer.push_str(&line[start..end]);
                    buffer.guess_segment_properties();
                    buffer.set_direction(if rtl {
                        Direction::RightToLeft
                    } else {
                        Direction::LeftToRight
                    });
                    let shaped = rustybuzz::shape(&face, &[], buffer);
                    for (info, position) in
                        shaped.glyph_infos().iter().zip(shaped.glyph_positions())
                    {
                        if result.glyphs.len() >= MAX_GLYPHS {
                            return Err(RasterError::Invalid("glyph limit"));
                        }
                        let x = quantize(pen_x + position.x_offset as f32 * scale);
                        let y = quantize(baseline - position.y_offset as f32 * scale);
                        let advance = quantize(position.x_advance as f32 * scale);
                        let mut outline = Outline {
                            commands: Vec::new(),
                            scale,
                            x,
                            y,
                            overflow: false,
                        };
                        let id = u16::try_from(info.glyph_id)
                            .map_err(|_| RasterError::Invalid("glyph id"))?;
                        face.outline_glyph(ttf_parser::GlyphId(id), &mut outline);
                        command_count = command_count
                            .checked_add(outline.commands.len())
                            .ok_or(RasterError::Allocation)?;
                        if outline.overflow || command_count > MAX_OUTLINE_COMMANDS {
                            return Err(RasterError::Invalid("outline limit"));
                        }
                        result.glyphs.push(Glyph {
                            glyph_id: info.glyph_id,
                            cluster: logical_offset + start + info.cluster as usize,
                            font,
                            x,
                            y,
                            advance,
                            outline: outline.commands,
                        });
                        pen_x = quantize(pen_x + advance);
                    }
                }
            }
        }
        result.width = result.width.max(pen_x);
        result.height = baseline + line_height;
        logical_offset += line.len() + 1;
    }
    Ok(result)
}

pub(crate) fn append_outlines(builder: &mut tiny_skia::PathBuilder, layout: &TextLayout) {
    for glyph in &layout.glyphs {
        for command in &glyph.outline {
            match *command {
                OutlineCommand::Move(x, y) => builder.move_to(x, y),
                OutlineCommand::Line(x, y) => builder.line_to(x, y),
                OutlineCommand::Quad(a, b, c, d) => builder.quad_to(a, b, c, d),
                OutlineCommand::Cubic(a, b, c, d, e, f) => builder.cubic_to(a, b, c, d, e, f),
                OutlineCommand::Close => builder.close(),
            }
        }
    }
}
