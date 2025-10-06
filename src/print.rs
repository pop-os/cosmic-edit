use ashpd::desktop::print::{Orientation, PageSetup, PrintProxy, Settings};
use cosmic_text::{Buffer, LayoutGlyph, LayoutRun, Scroll, fontdb};
use printpdf::*;
use std::{
    collections::HashMap,
    error::Error,
    fs,
    io::{self, Write},
    mem,
    sync::Arc,
};

use crate::{fl, font_system};

struct Span {
    font: FontId,
    font_size_pt: Pt,
    font_size_px: Px,
    codepoints: Vec<(i64, u16, char)>,
    x: i64,
}

impl Span {
    fn new(font: FontId, font_size_pt: Pt, font_size_px: Px) -> Self {
        Self {
            font,
            font_size_pt,
            font_size_px,
            codepoints: Vec::new(),
            x: 0,
        }
    }

    fn glyph(&mut self, font: FontId, glyph: &LayoutGlyph, run: &LayoutRun, ops: &mut Vec<Op>) {
        if font != self.font {
            // Flush and set font size if it changed
            self.flush(run, ops);
            self.font = font;
            ops.push(Op::SetFontSize {
                font: self.font.clone(),
                size: self.font_size_pt,
            });
        }

        let em_1000 = |x: f32| -> i64 { ((x * 1000.0) / (self.font_size_px.0) as f32) as i64 };
        let glyph_x = em_1000(glyph.x);
        // TODO: glyphs can share chars and one glyph can reference multiple chars
        for (i, c) in run.text[glyph.start..glyph.end].char_indices() {
            if i == 0 {
                self.codepoints.push((self.x - glyph_x, glyph.glyph_id, c));
            } else {
                eprintln!("extra char {:?}", c);
            }
        }
        self.x = glyph_x + em_1000(glyph.w);
    }

    fn flush(&mut self, run: &LayoutRun, ops: &mut Vec<Op>) {
        if !self.codepoints.is_empty() {
            let mut codepoints = Vec::new();
            mem::swap(&mut codepoints, &mut self.codepoints);
            ops.push(Op::WriteCodepointsWithKerning {
                font: self.font.clone(),
                cpk: codepoints,
            });
        }
    }

    fn line(&mut self, run: &LayoutRun, ops: &mut Vec<Op>) {
        self.flush(run, ops);
        ops.push(Op::AddLineBreak);
        self.x = 0;
    }
}

fn generate_pdf<W: Write>(buffer: &mut Buffer, page_setup: &PageSetup, w: &mut W) {
    //TODO: title?
    let mut doc = PdfDocument::new("");
    let mut pdf_fonts: HashMap<fontdb::ID, Option<FontId>> = HashMap::new();

    let outer_width = Mm(page_setup.width.unwrap_or(216.0) as f32);
    let outer_height = Mm(page_setup.height.unwrap_or(279.0) as f32);
    let margin_top = Mm(page_setup.margin_top.unwrap_or(25.4) as f32);
    let margin_bottom = Mm(page_setup.margin_bottom.unwrap_or(25.4) as f32);
    let margin_left = Mm(page_setup.margin_left.unwrap_or(25.4) as f32);
    let margin_right = Mm(page_setup.margin_right.unwrap_or(25.4) as f32);
    let inner_width_mm = outer_width - margin_left - margin_right;
    let inner_height_mm = outer_height - margin_top - margin_bottom;
    let dpi = 144.0;
    let inner_width_px = inner_width_mm.into_pt().into_px(dpi);
    let inner_height_px = inner_height_mm.into_pt().into_px(dpi);
    let metrics = buffer.metrics();
    let font_size_px = Px(metrics.font_size as usize);
    let font_size_pt = font_size_px.into_pt(dpi);
    let line_height = Px(metrics.line_height as usize).into_pt(dpi);
    let (buffer_width, buffer_height, translate_x, translate_y, rotate) =
        match page_setup.orientation.unwrap_or(Orientation::Portrait) {
            Orientation::Portrait => (
                inner_width_px.0 as f32,
                inner_height_px.0 as f32,
                Pt::from(margin_left),
                Pt::from(outer_height - margin_top) - line_height,
                0.0,
            ),
            Orientation::Landscape => (
                inner_height_px.0 as f32,
                inner_width_px.0 as f32,
                Pt::from(margin_top) + line_height,
                Pt::from(margin_left),
                90.0,
            ),
            Orientation::ReversePortrait => (
                inner_width_px.0 as f32,
                inner_height_px.0 as f32,
                Pt::from(outer_width - margin_left),
                Pt::from(margin_top) + line_height,
                180.0,
            ),
            Orientation::ReverseLandscape => (
                inner_height_px.0 as f32,
                inner_width_px.0 as f32,
                Pt::from(outer_width - margin_left) - line_height,
                Pt::from(outer_height - margin_top),
                270.0,
            ),
        };

    {
        let mut font_system = font_system().write().unwrap();
        let mut buffer = buffer.borrow_with(font_system.raw());
        buffer.set_scroll(Scroll::default());
        buffer.set_size(Some(buffer_width), None);
        buffer.shape_until_scroll(true);
    }

    let mut skip = 0;
    loop {
        let mut ops = vec![
            Op::SaveGraphicsState,
            Op::StartTextSection,
            Op::SetTextMatrix {
                matrix: TextMatrix::TranslateRotate(translate_x, translate_y, rotate),
            },
            Op::SetLineHeight { lh: line_height },
        ];

        let mut span = Span::new(FontId(String::new()), font_size_pt, font_size_px);
        let mut lines = 0;
        let mut height = 0.0;
        for run in buffer.layout_runs().skip(skip) {
            height += metrics.line_height;
            if height > buffer_height {
                break;
            }
            lines += 1;

            for glyph in run.glyphs.iter() {
                let font_opt = pdf_fonts
                    .entry(glyph.font_id)
                    .or_insert_with(|| {
                        let mut font_system = font_system().write().unwrap();
                        let info = font_system.raw().db().face(glyph.font_id)?;
                        let data: &[u8] = match &info.source {
                            fontdb::Source::Binary(data) => data.as_ref().as_ref(),
                            fontdb::Source::SharedFile(_path, data) => data.as_ref().as_ref(),
                            _ => return None,
                        };
                        let mut warnings = Vec::new();
                        let parsed =
                            ParsedFont::from_bytes(data, info.index as usize, &mut warnings)?;
                        Some(doc.add_font(&parsed))
                    })
                    .clone();

                //TODO: what to do with missing font?
                if let Some(font) = font_opt {
                    span.glyph(font, &glyph, &run, &mut ops);
                }
            }

            span.line(&run, &mut ops);
        }

        if lines == 0 {
            break;
        } else {
            skip += lines;
        }

        ops.push(Op::EndTextSection);
        ops.push(Op::RestoreGraphicsState);

        let page = PdfPage::new(outer_width, outer_height, ops);
        doc.pages.push(page);
    }

    let mut warnings = Vec::new();
    doc.save_writer(w, &PdfSaveOptions::default(), &mut warnings);
    //println!("{:#?}", warnings);
}

pub(crate) async fn print(mut buffer: Arc<Buffer>) -> Result<(), Box<dyn Error>> {
    let proxy = PrintProxy::new().await?;

    let window_id = None; //TODO: support setting window ID
    let title = fl!("print");
    let modal = true;
    let pre_print = proxy
        .prepare_print(
            window_id,
            &title,
            Settings::default(),
            PageSetup::default(),
            None,
            modal,
        )
        .await?
        .response()?;
    println!("{:#?}", pre_print);
    let token_opt = Some(pre_print.token);

    let file = tokio::task::spawn_blocking(move || -> io::Result<fs::File> {
        let mut file = tempfile::Builder::new()
            .prefix("cosmic-edit")
            .suffix(".pdf")
            .tempfile()?;
        generate_pdf(Arc::make_mut(&mut buffer), &pre_print.page_setup, &mut file);
        file.reopen()
    })
    .await??;

    proxy
        .print(window_id, &title, &file, token_opt, modal)
        .await?
        .response()?;

    Ok(())
}
