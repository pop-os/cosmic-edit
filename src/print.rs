use ashpd::desktop::print::{Orientation, PageSetup, PrintProxy, Settings};
use cosmic_text::{Buffer, Scroll, fontdb};
use printpdf::*;
use std::{
    cmp,
    collections::HashMap,
    error::Error,
    fs,
    io::{self, Write},
    ops::Range,
    sync::Arc,
};

use crate::{fl, font_system};

struct Run {
    font: FontId,
    font_size: Pt,
    range: Range<usize>,
}
impl Run {
    fn new(font: FontId, font_size: Pt) -> Self {
        Self {
            font,
            font_size,
            range: Default::default(),
        }
    }

    fn glyph(&mut self, font: FontId, start: usize, end: usize, text: &str, ops: &mut Vec<Op>) {
        if font != self.font {
            // Flush and set font if it changed
            self.flush(text, ops);
            self.font = font;
            ops.push(Op::SetFontSize {
                font: self.font.clone(),
                size: self.font_size,
            });
        }

        if self.range.is_empty() {
            // Set range if empty
            self.range = start..end;
        } else if start > self.range.end || end < self.range.start {
            // Flush and reset range if glyph out of range
            self.flush(text, ops);
            self.range = start..end;
        } else {
            // Expand range to include glyph
            self.range.start = cmp::min(self.range.start, start);
            self.range.end = cmp::max(self.range.end, end);
        }
    }

    fn flush(&mut self, text: &str, ops: &mut Vec<Op>) {
        if !self.range.is_empty() {
            let string = text[self.range.clone()].to_string();
            self.range = Default::default();
            ops.push(Op::WriteText {
                font: self.font.clone(),
                items: vec![TextItem::Text(string)],
            });
        }
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
    let dpi = 96.0;
    let inner_width_px = inner_width_mm.into_pt().into_px(dpi);
    let inner_height_px = inner_height_mm.into_pt().into_px(dpi);
    let metrics = buffer.metrics();
    let font_size = Px(metrics.font_size as usize).into_pt(dpi);
    let line_height = Px(metrics.line_height as usize).into_pt(dpi);
    let (buffer_width, buffer_height, matrix) =
        match page_setup.orientation.unwrap_or(Orientation::Portrait) {
            Orientation::Portrait => (
                inner_width_px.0 as f32,
                inner_height_px.0 as f32,
                TextMatrix::TranslateRotate(
                    Pt::from(margin_left),
                    Pt::from(outer_height - margin_top) - line_height,
                    0.0,
                ),
            ),
            Orientation::Landscape => (
                inner_height_px.0 as f32,
                inner_width_px.0 as f32,
                TextMatrix::TranslateRotate(
                    Pt::from(margin_top) + line_height,
                    Pt::from(margin_left),
                    90.0,
                ),
            ),
            Orientation::ReversePortrait => (
                inner_width_px.0 as f32,
                inner_height_px.0 as f32,
                TextMatrix::TranslateRotate(
                    Pt::from(outer_width - margin_left),
                    Pt::from(margin_top) + line_height,
                    180.0,
                ),
            ),
            Orientation::ReverseLandscape => (
                inner_height_px.0 as f32,
                inner_width_px.0 as f32,
                TextMatrix::TranslateRotate(
                    Pt::from(outer_width - margin_left) - line_height,
                    Pt::from(outer_height - margin_top),
                    270.0,
                ),
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
            Op::SetTextMatrix { matrix },
            Op::SetLineHeight { lh: line_height },
        ];

        let mut current_run = Run::new(FontId(String::new()), font_size);
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
                    current_run.glyph(font, glyph.start, glyph.end, &run.text, &mut ops);
                }
            }

            current_run.flush(&run.text, &mut ops);
            ops.push(Op::AddLineBreak);
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

    dbg!(
        proxy
            .print(window_id, &title, &file, token_opt, modal)
            .await?
            .response()
    )?;

    Ok(())
}
