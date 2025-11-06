// SPDX-License-Identifier: GPL-3.0-only

use cosmic::{
    Renderer,
    cosmic_theme::palette::{WithAlpha, blend::Compose},
    iced::{
        Color, Element, Length, Padding, Point, Rectangle, Size, Vector,
        advanced::graphics::text::font_system,
        event::{Event, Status},
        keyboard::{Event as KeyEvent, Modifiers},
        mouse::{self, Button, Event as MouseEvent, ScrollDelta},
    },
    iced_core::{
        Border, Shell,
        clipboard::Clipboard,
        image,
        keyboard::{Key, key::Named},
        layout::{self, Layout},
        renderer::{self, Quad, Renderer as _},
        widget::{
            self, Id, Widget,
            operation::{self, Operation},
            tree,
        },
    },
    theme::Theme,
};
use cosmic_text::{
    Action, BorrowedWithFontSystem, Edit, Metrics, Motion, Scroll, Selection, ViEditor,
};
use std::{
    cell::Cell,
    cmp,
    sync::Mutex,
    time::{Duration, Instant},
};

use crate::{LINE_NUMBER_CACHE, SWASH_CACHE, line_number::LineNumberKey};

// Color channel bit masks for RGBA/BGRA conversion
const ALPHA_GREEN_MASK: u32 = 0xFF00FF00;
const RED_MASK: u32 = 0x00FF0000;
const BLUE_MASK: u32 = 0x000000FF;
const RED_BLUE_MASK: u32 = 0x00FF00FF;
const ALPHA_GREEN_SHIFT_MASK: u32 = 0xFF00FF00;
const ALPHA_FULL_MASK: u32 = 0xFF000000;

/// Draws a rectangle to a pixel buffer with alpha blending.
/// Converts cosmic_text color format (RGBA) to pixel buffer format (BGRA).
fn draw_rect_to_buffer(
    pixels: &mut [u32],
    buffer_width: i32,
    buffer_height: i32,
    x: i32,
    y: i32,
    rect_width: i32,
    rect_height: i32,
    cosmic_color: cosmic_text::Color,
) {
    // Convert RGBA to BGRA and swap red/blue channels
    let mut color = cosmic_color.0 & ALPHA_GREEN_MASK;
    color |= (cosmic_color.0 & RED_MASK) >> 16;
    color |= (cosmic_color.0 & BLUE_MASK) << 16;

    let alpha = (color >> 24) & 0xFF;
    if alpha == 0 {
        return; // Fully transparent, skip drawing
    }

    // Draw rectangle with bounds checking
    for dx in 0..rect_width {
        let px = x + dx;
        if px < 0 || px >= buffer_width {
            continue;
        }
        for dy in 0..rect_height {
            let py = y + dy;
            if py < 0 || py >= buffer_height {
                continue;
            }
            let offset = py as usize * buffer_width as usize + px as usize;

            if alpha == 255 {
                // Fully opaque, overwrite
                pixels[offset] = color;
            } else {
                // Alpha blending with existing pixel
                let n_alpha = 255 - alpha;
                let current = pixels[offset];
                if current & ALPHA_FULL_MASK == 0 {
                    // Background is transparent, just write color
                    pixels[offset] = color;
                } else {
                    // Blend with existing color
                    let rb = ((n_alpha * (current & RED_BLUE_MASK)) + (alpha * (color & RED_BLUE_MASK))) >> 8;
                    let ag = (n_alpha * ((current & ALPHA_GREEN_SHIFT_MASK) >> 8)) + (alpha * (0x01000000 | ((color & 0x0000FF00) >> 8)));
                    pixels[offset] = (rb & RED_BLUE_MASK) | (ag & ALPHA_GREEN_SHIFT_MASK);
                }
            }
        }
    }
}

pub struct TextBox<'a, Message> {
    editor: &'a Mutex<ViEditor<'static, 'static>>,
    metrics: Metrics,
    id: Option<Id>,
    padding: Padding,
    on_auto_scroll: Option<Box<dyn Fn(Option<f32>) -> Message + 'a>>,
    on_changed: Option<Message>,
    on_focus: Option<Message>,
    click_timing: Duration,
    has_context_menu: bool,
    on_context_menu: Option<Box<dyn Fn(Option<Point>) -> Message + 'a>>,
    highlight_current_line: bool,
    line_numbers: bool,
}

impl<'a, Message> TextBox<'a, Message>
where
    Message: Clone,
{
    pub fn new(editor: &'a Mutex<ViEditor<'static, 'static>>, metrics: Metrics) -> Self {
        Self {
            editor,
            metrics,
            id: None,
            padding: Padding::new(0.0),
            on_auto_scroll: None,
            on_changed: None,
            on_focus: None,
            click_timing: Duration::from_millis(500),
            has_context_menu: false,
            on_context_menu: None,
            highlight_current_line: false,
            line_numbers: false,
        }
    }

    pub fn id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }

    pub fn padding<P: Into<Padding>>(mut self, padding: P) -> Self {
        self.padding = padding.into();
        self
    }

    pub fn on_auto_scroll(mut self, on_auto_scroll: impl Fn(Option<f32>) -> Message + 'a) -> Self {
        self.on_auto_scroll = Some(Box::new(on_auto_scroll));
        self
    }

    pub fn on_changed(mut self, on_changed: Message) -> Self {
        self.on_changed = Some(on_changed);
        self
    }

    pub fn click_timing(mut self, click_timing: Duration) -> Self {
        self.click_timing = click_timing;
        self
    }

    pub fn has_context_menu(mut self, has_context_menu: bool) -> Self {
        self.has_context_menu = has_context_menu;
        self
    }

    pub fn on_context_menu(
        mut self,
        on_context_menu: impl Fn(Option<Point>) -> Message + 'a,
    ) -> Self {
        self.on_context_menu = Some(Box::new(on_context_menu));
        self
    }

    pub fn highlight_current_line(mut self) -> Self {
        self.highlight_current_line = true;
        self
    }

    pub fn line_numbers(mut self) -> Self {
        self.line_numbers = true;
        self
    }

    pub fn on_focus(mut self, on_focus: Message) -> Self {
        self.on_focus = Some(on_focus);
        self
    }
}

pub fn text_box<'a, Message>(
    editor: &'a Mutex<ViEditor<'static, 'static>>,
    metrics: Metrics,
) -> TextBox<'a, Message>
where
    Message: Clone,
{
    TextBox::new(editor, metrics)
}

impl<'a, Message> Widget<Message, cosmic::Theme, Renderer> for TextBox<'a, Message>
where
    Message: Clone,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::new())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &self,
        _tree: &mut widget::Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let limits = limits.width(Length::Fill).height(Length::Fill);

        let mut editor = self.editor.lock().unwrap();
        //TODO: set size?
        editor
            .borrow_with(font_system().write().unwrap().raw())
            .shape_as_needed(true);

        editor.with_buffer(|buffer| {
            let mut layout_lines = 0;
            for line in buffer.lines.iter() {
                match line.layout_opt() {
                    Some(layout) => layout_lines += layout.len(),
                    None => (),
                }
            }

            let height = layout_lines as f32 * buffer.metrics().line_height;
            let size = Size::new(limits.max().width, height);

            layout::Node::new(limits.resolve(Length::Fill, Length::Fill, size))
        })
    }

    fn operate(
        &self,
        tree: &mut widget::Tree,
        _layout: Layout<'_>,
        _renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let state = tree.state.downcast_mut::<State>();

        operation.focusable(state, self.id.as_ref());
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor_position: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<State>();

        if let Some(Dragging::ScrollbarV { .. }) = &state.dragging {
            return mouse::Interaction::Idle;
        }

        if let Some(p) = cursor_position.position_in(layout.bounds()) {
            let editor_offset_x = state.editor_offset_x.get();
            let scale_factor = state.scale_factor.get();
            let editor = self.editor.lock().unwrap();
            let buffer_size = editor.with_buffer(|buffer| buffer.size());

            let x_logical = p.x - self.padding.left;
            let y_logical = p.y - self.padding.top;
            let x = x_logical * scale_factor - editor_offset_x as f32;
            let y = y_logical * scale_factor;
            if x >= 0.0
                && x < buffer_size.0.unwrap_or(0.0)
                && y >= 0.0
                && y < buffer_size.1.unwrap_or(0.0)
            {
                return mouse::Interaction::Text;
            }
        }

        mouse::Interaction::Idle
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor_position: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let instant = Instant::now();

        let state = tree.state.downcast_ref::<State>();

        let mut editor = self.editor.lock().unwrap();

        let cosmic_theme = theme.cosmic();
        let scrollbar_w = cosmic_theme.spacing.space_xxs as i32;

        let view_w = cmp::min(viewport.width as i32, layout.bounds().width as i32)
            - self.padding.horizontal() as i32
            - scrollbar_w;
        let view_h = cmp::min(viewport.height as i32, layout.bounds().height as i32)
            - self.padding.vertical() as i32;

        let scale_factor = style.scale_factor as f32;
        let metrics = self.metrics.scale(scale_factor);

        let calculate_image_scaled = |view: i32| -> (i32, f32) {
            // Get smallest set of physical pixels that fit inside the logical pixels
            let image = ((view as f32) * scale_factor).floor() as i32;
            // Convert that back into logical pixels
            let scaled = (image as f32) / scale_factor;
            (image, scaled)
        };
        let calculate_ideal = |view_start: i32| -> (i32, f32) {
            // Search for a perfect match within 16 pixels
            for i in 0..16 {
                let view = view_start - i;
                let (image, scaled) = calculate_image_scaled(view);
                if view == scaled as i32 {
                    return (image, scaled);
                }
            }
            let (image, scaled) = calculate_image_scaled(view_start);
            (image, scaled)
        };

        let (image_w, scaled_w) = calculate_ideal(view_w);
        let (image_h, scaled_h) = calculate_ideal(view_h);

        if image_w <= 0 || image_h <= 0 {
            // Zero sized image
            return;
        }

        // Lock font system (used throughout)
        let mut font_system = font_system().write().unwrap();

        // Calculate line number information
        let (line_number_chars, editor_offset_x) = if self.line_numbers {
            // Calculate number of characters needed in line number
            let mut line_number_chars = 1;
            let mut line_count = editor.with_buffer(|buffer| buffer.lines.len());
            while line_count >= 10 {
                line_count /= 10;
                line_number_chars += 1;
            }

            // Calculate line number width
            let mut line_number_width = 0.0;
            {
                let mut line_number_cache = LINE_NUMBER_CACHE.get().unwrap().lock().unwrap();
                if let Some(layout_line) = line_number_cache
                    .get(
                        font_system.raw(),
                        LineNumberKey {
                            number: 1,
                            width: line_number_chars,
                        },
                    )
                    .first()
                {
                    let line_width = layout_line.w * metrics.font_size;
                    if line_width > line_number_width {
                        line_number_width = line_width;
                    }
                }
            }

            (line_number_chars, (line_number_width + 8.0).ceil() as i32)
        } else {
            (0, 0)
        };

        // Save editor offset in state
        if state.editor_offset_x.replace(editor_offset_x) != editor_offset_x {
            // Mark buffer as needing redraw if editor offset has changed
            editor.set_redraw(true);
        }

        // Set metrics and size
        editor.with_buffer_mut(|buffer| {
            buffer.set_metrics_and_size(
                font_system.raw(),
                metrics,
                Some((image_w - editor_offset_x) as f32),
                Some(image_h as f32),
            )
        });

        // Shape and layout as needed
        editor.shape_as_needed(font_system.raw(), true);

        let needs_redraw = editor.redraw();
        if needs_redraw {
            // GPU rendering - skip pixel buffer creation and calculate scrollbar positions
            editor.with_buffer(|buffer| {
                let mut start_line_opt = None;
                let mut end_line = 0;
                let mut max_line_width = 0.0;
                for run in buffer.layout_runs() {
                    end_line = run.line_i;
                    if start_line_opt.is_none() {
                        start_line_opt = Some(end_line);
                    }
                    if run.line_w > max_line_width {
                        max_line_width = run.line_w;
                    }
                }

                let start_line = start_line_opt.unwrap_or(end_line);
                let lines = buffer.lines.len();
                let start_y = (start_line * image_h as usize) / lines;
                let end_y = ((end_line + 1) * image_h as usize) / lines;

                let rect = Rectangle::new(
                    [image_w as f32 / scale_factor, start_y as f32 / scale_factor].into(),
                    Size::new(
                        scrollbar_w as f32,
                        (end_y as f32 - start_y as f32) / scale_factor,
                    ),
                );
                state.scrollbar_v_rect.set(rect);

                let (buffer_w_opt, buffer_h_opt) = buffer.size();
                let buffer_w = buffer_w_opt.unwrap_or(0.0);
                let buffer_h = buffer_h_opt.unwrap_or(0.0);
                let scrollbar_h_width = image_w as f32 / scale_factor - scrollbar_w as f32;
                if buffer_w < max_line_width {
                    let rect = Rectangle::new(
                        [
                            (buffer.scroll().horizontal / max_line_width) * scrollbar_h_width,
                            buffer_h / scale_factor - scrollbar_w as f32,
                        ]
                        .into(),
                        Size::new(
                            (buffer_w / max_line_width) * scrollbar_h_width,
                            scrollbar_w as f32,
                        ),
                    );
                    state.scrollbar_h_rect.set(Some(rect));
                } else {
                    state.scrollbar_h_rect.set(None);
                }
            });

            editor.set_redraw(false);
            state.scale_factor.set(scale_factor);
        }

        let image_position = layout.position() + [self.padding.left, self.padding.top].into();

        // Render line numbers to gutter pixel buffer
        let mut gutter_handle_opt = state.gutter_handle_opt.lock().unwrap();
        let scale_factor_changed = state.scale_factor.get() != scale_factor;
        if self.line_numbers && (needs_redraw || scale_factor_changed || gutter_handle_opt.is_none()) {
            let gutter_w = editor_offset_x;
            if gutter_w > 0 && image_h > 0 {
                // Create pixel buffer for gutter only
                let mut gutter_pixels_u8 = vec![0; gutter_w as usize * image_h as usize * 4];
                {
                    let mut swash_cache = SWASH_CACHE.get().unwrap().lock().unwrap();

                    // SAFETY: Converting u8 slice to u32 slice for efficient pixel manipulation.
                    // This is safe because:
                    // 1. Vec<u8> allocates with proper alignment for u32 access
                    // 2. Length is exactly divisible by 4 (width * height * 4 bytes / 4 = width * height u32s)
                    // 3. Lifetime is tied to gutter_pixels_u8, preventing dangling references
                    // 4. We have exclusive mutable access to the vector
                    let gutter_pixels = unsafe {
                        std::slice::from_raw_parts_mut(
                            gutter_pixels_u8.as_mut_ptr() as *mut u32,
                            gutter_pixels_u8.len() / 4,
                        )
                    };

                    let (gutter, gutter_foreground) = {
                        let convert_color = |color: syntect::highlighting::Color| {
                            cosmic_text::Color::rgba(color.r, color.g, color.b, color.a)
                        };
                        let syntax_theme = editor.theme();

                        // Get gutter color, or create a contrasting one
                        let gutter = syntax_theme.settings.gutter.map(convert_color).unwrap_or_else(|| {
                            let bg = editor.background_color();
                            // Create a slightly different color from background
                            let avg = (bg.r() as f32 + bg.g() as f32 + bg.b() as f32) / 3.0;
                            if avg > 128.0 {
                                // Light theme - darken
                                cosmic_text::Color::rgb(
                                    (bg.r() as f32 * 0.95) as u8,
                                    (bg.g() as f32 * 0.95) as u8,
                                    (bg.b() as f32 * 0.95) as u8,
                                )
                            } else {
                                // Dark theme - lighten
                                cosmic_text::Color::rgb(
                                    (bg.r() as f32 * 1.1).min(255.0) as u8,
                                    (bg.g() as f32 * 1.1).min(255.0) as u8,
                                    (bg.b() as f32 * 1.1).min(255.0) as u8,
                                )
                            }
                        });

                        let gutter_foreground = syntax_theme.settings.gutter_foreground.map_or(editor.foreground_color(), convert_color);
                        (gutter, gutter_foreground)
                    };

                    // Fill gutter background
                    draw_rect_to_buffer(gutter_pixels, gutter_w, image_h, 0, 0, gutter_w, image_h, gutter);

                    // Draw line numbers
                    editor.with_buffer(|buffer| {
                        let mut line_number_cache = LINE_NUMBER_CACHE.get().unwrap().lock().unwrap();
                        let mut last_line_number = 0;
                        for run in buffer.layout_runs() {
                            let line_number = run.line_i.saturating_add(1);
                            if line_number == last_line_number {
                                continue;
                            }
                            last_line_number = line_number;

                            if let Some(layout_line) = line_number_cache
                                .get(
                                    font_system.raw(),
                                    LineNumberKey {
                                        number: line_number,
                                        width: line_number_chars,
                                    },
                                )
                                .first()
                            {
                                let max_ascent = layout_line.max_ascent * metrics.font_size;
                                let max_descent = layout_line.max_descent * metrics.font_size;
                                let glyph_height = max_ascent + max_descent;
                                let centering_offset = (metrics.line_height - glyph_height) / 2.0;
                                let line_y = run.line_top + centering_offset + max_ascent;

                                for layout_glyph in layout_line.glyphs.iter() {
                                    let physical_glyph = layout_glyph.physical((0., line_y), metrics.font_size);

                                    swash_cache.with_pixels(
                                        font_system.raw(),
                                        physical_glyph.cache_key,
                                        gutter_foreground,
                                        |x, y, color| {
                                            draw_rect_to_buffer(
                                                gutter_pixels,
                                                gutter_w,
                                                image_h,
                                                physical_glyph.x + x,
                                                physical_glyph.y + y,
                                                1,
                                                1,
                                                color,
                                            );
                                        },
                                    );
                                }
                            }
                        }
                    });
                }

                *gutter_handle_opt = Some(image::Handle::from_rgba(
                    gutter_w as u32,
                    image_h as u32,
                    gutter_pixels_u8,
                ));
            }
        }
        drop(gutter_handle_opt);

        // GPU rendering
        {
            // Helper to convert syntect colors to iced colors
            let convert_color = |color: syntect::highlighting::Color| {
                Color::from_rgba8(color.r, color.g, color.b, color.a as f32 / 255.0)
            };

            let syntax_theme = editor.theme();
            let text_color = syntax_theme.settings.foreground.map_or(Color::WHITE, convert_color);
            let bg_color = syntax_theme.settings.background.map_or(Color::BLACK, convert_color);

            // Draw main background FIRST for entire area
            renderer.fill_quad(
                Quad {
                    bounds: Rectangle::new(image_position, Size::new(scaled_w as f32, scaled_h as f32)),
                    ..Default::default()
                },
                bg_color,
            );

            // Then draw gutter image on top (if line numbers enabled)
            // This way the text area already has its background set
            if self.line_numbers {
                let handle_clone = {
                    let gutter_handle_opt = state.gutter_handle_opt.lock().unwrap();
                    gutter_handle_opt.as_ref().cloned()
                };

                if let Some(handle) = handle_clone {
                    use cosmic::iced_core::image;
                    use cosmic::iced_core::Radians;

                    // Save any renderer state
                    // Draw gutter as isolated operation
                    let gutter_size = Size::new(
                        editor_offset_x as f32 / scale_factor,
                        scaled_h as f32
                    );

                    image::Renderer::draw_image(
                        renderer,
                        handle,
                        image::FilterMethod::Nearest,
                        Rectangle::new(image_position, gutter_size),
                        Radians(0.0),
                        1.0,
                        [0.0; 4],
                    );
                }
            }

            use cosmic::iced_core::Pixels;
            use cosmic::iced_core::text::Renderer as TextRendererTrait;
            use cosmic::iced_core::text::Text;

            let selection_color = syntax_theme.settings.selection.map_or(Color::from_rgba8(100, 100, 255, 0.3), convert_color);
            let cursor_color = syntax_theme.settings.caret.map_or(Color::WHITE, convert_color);
            let line_highlight_color = syntax_theme.settings.line_highlight.map_or(bg_color, convert_color);
            let cursor = editor.cursor();
            let base_x = image_position.x + (editor_offset_x as f32 / scale_factor);
            let base_y = image_position.y;

            editor.with_buffer(|buffer| {
                // Render current line highlighting if enabled
                if self.highlight_current_line {
                    for run in buffer.layout_runs() {
                        if run.line_i != cursor.line {
                            continue;
                        }

                        renderer.fill_quad(
                            Quad {
                                bounds: Rectangle::new(
                                    Point::new(
                                        base_x,
                                        base_y + (run.line_top / scale_factor),
                                    ),
                                    Size::new(
                                        scaled_w as f32 - (editor_offset_x as f32 / scale_factor),
                                        run.line_height / scale_factor,
                                    ),
                                ),
                                ..Default::default()
                            },
                            line_highlight_color,
                        );
                    }
                }

                // Render selection
                if let Some((start, end)) = editor.selection_bounds() {

                    for run in buffer.layout_runs() {
                        let line_i = run.line_i;
                        if line_i < start.line || line_i > end.line {
                            continue;
                        }

                        let line_start = if line_i == start.line { start.index } else { 0 };
                        let line_end = if line_i == end.line { end.index } else { run.text.len() };

                        if line_start >= line_end {
                            continue;
                        }

                        let mut sel_x = 0.0;
                        let mut sel_end_x = 0.0;

                        for glyph in run.glyphs.iter() {
                            if glyph.start >= line_end {
                                break;
                            }
                            if glyph.end <= line_start {
                                sel_x = (glyph.x + glyph.w) / scale_factor;
                                continue;
                            }
                            sel_end_x = (glyph.x + glyph.w) / scale_factor;
                        }

                        let sel_width = sel_end_x - sel_x;

                        if sel_width > 0.0 {
                            renderer.fill_quad(
                                Quad {
                                    bounds: Rectangle::new(
                                        Point::new(
                                            base_x + sel_x,
                                            base_y + (run.line_top / scale_factor),
                                        ),
                                        Size::new(sel_width, run.line_height / scale_factor),
                                    ),
                                    ..Default::default()
                                },
                                selection_color,
                            );
                        }
                    }
                }

                // Render cursor and text in a single buffer iteration
                // Clip bounds should start after the gutter to avoid overlap
                let text_clip_x = if self.line_numbers {
                    base_x
                } else {
                    image_position.x
                };
                let text_clip_width = if self.line_numbers {
                    scaled_w as f32 - (editor_offset_x as f32 / scale_factor)
                } else {
                    scaled_w as f32
                };
                let clip_bounds = Rectangle::new(
                    Point::new(text_clip_x, image_position.y),
                    Size::new(text_clip_width, scaled_h as f32)
                );

                for run in buffer.layout_runs() {
                    // Render cursor on the current line
                    if run.line_i == cursor.line {
                        let mut cursor_x = 0.0;
                        for glyph in run.glyphs.iter() {
                            if glyph.start >= cursor.index {
                                break;
                            }
                            cursor_x = (glyph.x + glyph.w) / scale_factor;
                        }

                        renderer.fill_quad(
                            Quad {
                                bounds: Rectangle::new(
                                    Point::new(base_x + cursor_x, base_y + (run.line_top / scale_factor)),
                                    Size::new(2.0, run.line_height / scale_factor),
                                ),
                                ..Default::default()
                            },
                            cursor_color,
                        );
                    }

                    // Render glyphs - batch consecutive glyphs with same color
                    if run.glyphs.is_empty() {
                        continue;
                    }

                    let run_y = base_y + (run.line_top / scale_factor);

                    let mut batch_color = text_color;
                    let mut batch_text = String::new();
                    let mut batch_x = 0.0;

                    for (i, glyph) in run.glyphs.iter().enumerate() {
                        // Skip glyphs that would render in the gutter area
                        let glyph_x_logical = glyph.x / scale_factor;
                        if self.line_numbers && glyph_x_logical < 0.0 {
                            continue;
                        }

                        // Get glyph color from syntax highlighting or use default text color
                        let glyph_color = glyph
                            .color_opt
                            .map(|c| Color::from_rgba8(c.r(), c.g(), c.b(), c.a() as f32 / 255.0))
                            .unwrap_or(text_color);

                        // Initialize batch color from first glyph's actual color
                        if i == 0 {
                            batch_color = glyph_color;
                            batch_x = glyph.x / scale_factor;
                        }

                        // Start new batch if color changes
                        if i > 0 && glyph_color != batch_color {
                            // Render accumulated batch
                            if !batch_text.is_empty() {
                                let text = Text {
                                    content: batch_text.clone(),
                                    bounds: Size::new(f32::INFINITY, run.line_height / scale_factor),
                                    size: Pixels(metrics.font_size / scale_factor),
                                    line_height: cosmic::iced_core::text::LineHeight::Absolute(Pixels(
                                        metrics.line_height / scale_factor,
                                    )),
                                    font: cosmic::iced_core::Font::MONOSPACE,
                                    horizontal_alignment: cosmic::iced_core::alignment::Horizontal::Left,
                                    vertical_alignment: cosmic::iced_core::alignment::Vertical::Top,
                                    shaping: cosmic::iced_core::text::Shaping::Advanced,
                                    wrapping: cosmic::iced_core::text::Wrapping::None,
                                };

                                renderer.fill_text(
                                    text,
                                    Point::new(base_x + batch_x, run_y),
                                    batch_color,
                                    clip_bounds,
                                );
                            }

                            // Start new batch
                            batch_text.clear();
                            batch_color = glyph_color;
                            batch_x = glyph.x / scale_factor;
                        }

                        // Add character to current batch
                        if let Some(ch) = run.text[glyph.start..glyph.end].chars().next() {
                            batch_text.push(ch);
                        }
                    }

                    // Render final batch
                    if !batch_text.is_empty() {
                        let text = Text {
                            content: batch_text,
                            bounds: Size::new(f32::INFINITY, run.line_height / scale_factor),
                            size: Pixels(metrics.font_size / scale_factor),
                            line_height: cosmic::iced_core::text::LineHeight::Absolute(Pixels(
                                metrics.line_height / scale_factor,
                            )),
                            font: cosmic::iced_core::Font::MONOSPACE,
                            horizontal_alignment: cosmic::iced_core::alignment::Horizontal::Left,
                            vertical_alignment: cosmic::iced_core::alignment::Vertical::Top,
                            shaping: cosmic::iced_core::text::Shaping::Advanced,
                            wrapping: cosmic::iced_core::text::Wrapping::None,
                        };

                        renderer.fill_text(
                            text,
                            Point::new(base_x + batch_x, run_y),
                            batch_color,
                            clip_bounds,
                        );
                    }
                }
            });
        }

        // Draw vertical scrollbar
        {
            let scrollbar_v_rect = state.scrollbar_v_rect.get();

            // neutral_3, 0.7
            let track_color = cosmic_theme
                .palette
                .neutral_3
                .without_alpha()
                .with_alpha(0.7);

            // Draw track quad
            renderer.fill_quad(
                Quad {
                    bounds: Rectangle::new(
                        Point::new(image_position.x + scrollbar_v_rect.x, image_position.y),
                        Size::new(scrollbar_v_rect.width, layout.bounds().height),
                    ),
                    border: Border {
                        radius: (scrollbar_v_rect.width / 2.0).into(),
                        width: 0.0,
                        color: Color::TRANSPARENT,
                    },
                    ..Default::default()
                },
                Color::from(track_color),
            );

            let pressed = matches!(&state.dragging, Some(Dragging::ScrollbarV { .. }));

            let mut hover = false;
            if let Some(p) = cursor_position.position_in(layout.bounds()) {
                let x = p.x - self.padding.left;
                if x >= scrollbar_v_rect.x && x < (scrollbar_v_rect.x + scrollbar_v_rect.width) {
                    hover = true;
                }
            }

            let mut scrollbar_draw =
                scrollbar_v_rect + Vector::new(image_position.x, image_position.y);
            if !hover && !pressed {
                // Decrease draw width and keep centered when not hovered or pressed
                scrollbar_draw.width /= 2.0;
                scrollbar_draw.x += scrollbar_draw.width / 2.0;
            }

            // neutral_6, 0.7
            let base_color = cosmic_theme
                .palette
                .neutral_6
                .without_alpha()
                .with_alpha(0.7);
            let scrollbar_color = if pressed {
                // pressed_state_color, 0.5
                cosmic_theme
                    .background
                    .component
                    .pressed
                    .without_alpha()
                    .with_alpha(0.5)
                    .over(base_color)
            } else if hover {
                // hover_state_color, 0.2
                cosmic_theme
                    .background
                    .component
                    .hover
                    .without_alpha()
                    .with_alpha(0.2)
                    .over(base_color)
            } else {
                base_color
            };

            // Draw scrollbar quad
            renderer.fill_quad(
                Quad {
                    bounds: scrollbar_draw,
                    border: Border {
                        radius: (scrollbar_draw.width / 2.0).into(),
                        width: 0.0,
                        color: Color::TRANSPARENT,
                    },
                    ..Default::default()
                },
                Color::from(scrollbar_color),
            );
        }

        // Draw horizontal scrollbar
        //TODO: reduce repitition
        if let Some(scrollbar_h_rect) = state.scrollbar_h_rect.get() {
            /*TODO: horizontal scrollbar track?
            // neutral_3, 0.7
            let track_color = cosmic_theme
                .palette
                .neutral_3
                .without_alpha()
                .with_alpha(0.7);

            // Draw track quad
            renderer.fill_quad(
                Quad {
                    bounds: Rectangle::new(
                        Point::new(image_position.x, image_position.y + scrollbar_h_rect.y),
                        Size::new(
                            layout.bounds().width - scrollbar_w as f32,
                            scrollbar_h_rect.height,
                        ),
                    ),
                    border: Border {
                        radius: (scrollbar_h_rect.height / 2.0).into(),
                        width: 0.0,
                        color: Color::TRANSPARENT,
                    },
                    ..Default::default()
                },
                Color::from(track_color),
            );
            */

            let pressed = matches!(&state.dragging, Some(Dragging::ScrollbarH { .. }));

            let mut hover = false;
            if let Some(p) = cursor_position.position_in(layout.bounds()) {
                let y = p.y - self.padding.top;
                if y >= scrollbar_h_rect.y && y < (scrollbar_h_rect.y + scrollbar_h_rect.height) {
                    hover = true;
                }
            }

            let mut scrollbar_draw =
                scrollbar_h_rect + Vector::new(image_position.x, image_position.y);
            if !hover && !pressed {
                // Decrease draw width and keep centered when not hovered or pressed
                scrollbar_draw.height /= 2.0;
                scrollbar_draw.y += scrollbar_draw.height / 2.0;
            }

            // neutral_6, 0.7
            let base_color = cosmic_theme
                .palette
                .neutral_6
                .without_alpha()
                .with_alpha(0.7);
            let scrollbar_color = if pressed {
                // pressed_state_color, 0.5
                cosmic_theme
                    .background
                    .component
                    .pressed
                    .without_alpha()
                    .with_alpha(0.5)
                    .over(base_color)
            } else if hover {
                // hover_state_color, 0.2
                cosmic_theme
                    .background
                    .component
                    .hover
                    .without_alpha()
                    .with_alpha(0.2)
                    .over(base_color)
            } else {
                base_color
            };

            // Draw scrollbar quad
            renderer.fill_quad(
                Quad {
                    bounds: scrollbar_draw,
                    border: Border {
                        radius: (scrollbar_draw.height / 2.0).into(),
                        width: 0.0,
                        color: Color::TRANSPARENT,
                    },
                    ..Default::default()
                },
                Color::from(scrollbar_color),
            );
        }

        let duration = instant.elapsed();
        log::debug!("redraw {}, {}: {:?}", view_w, view_h, duration);
    }

    fn on_event(
        &mut self,
        tree: &mut widget::Tree,
        event: Event,
        layout: Layout<'_>,
        cursor_position: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle<f32>,
    ) -> Status {
        let state = tree.state.downcast_mut::<State>();
        let editor_offset_x = state.editor_offset_x.get();
        let scale_factor = state.scale_factor.get();
        let scrollbar_v_rect = state.scrollbar_v_rect.get();
        let mut editor = self.editor.lock().unwrap();
        let (buffer_size, buffer_scroll) =
            editor.with_buffer(|buffer| (buffer.size(), buffer.scroll()));
        let last_changed = editor.changed();
        //TODO: better handling of status line update
        let (last_parser_mode, last_parser_cmd) = {
            let parser = editor.parser();
            (parser.mode.clone(), parser.cmd)
        };
        let mut font_system = font_system().write().unwrap();
        let mut editor = editor.borrow_with(font_system.raw());

        // Adjust motions based on Ctrl and Shift
        fn motion_modifiers(
            editor: &mut BorrowedWithFontSystem<'_, ViEditor<'static, 'static>>,
            original_motion: Motion,
            modifiers: Modifiers,
        ) {
            let motion = if modifiers.control() {
                match original_motion {
                    Motion::Left => Motion::LeftWord,
                    Motion::Right => Motion::RightWord,
                    Motion::Home => Motion::BufferStart,
                    Motion::End => Motion::BufferEnd,
                    _ => original_motion,
                }
            } else {
                original_motion
            };
            let cursor = editor.cursor();
            match editor.selection() {
                Selection::None => {
                    if modifiers.shift() {
                        //TODO: Selection::Word if ctrl held?
                        editor.set_selection(Selection::Normal(cursor));
                    }
                }
                _ => {
                    if !modifiers.shift() {
                        editor.set_selection(Selection::None)
                    }
                }
            }
            editor.action(Action::Motion(motion));
        }

        // Pre-select word for CTRL+<backspace> and CTRL+<delete>
        fn delete_modifiers(
            editor: &mut BorrowedWithFontSystem<'_, ViEditor<'static, 'static>>,
            motion_to_apply: Motion,
            modifiers: Modifiers,
        ) {
            if modifiers.control() && editor.selection() == Selection::None {
                let cursor = editor.cursor();
                editor.set_selection(Selection::Normal(cursor));
                editor.action(Action::Motion(motion_to_apply));
            }
        }

        if let Some(on_focus) = self.on_focus.as_ref()
            && state.emit_focus
        {
            state.emit_focus = false;
            shell.publish(on_focus.clone());
        }

        let mut status = Status::Ignored;
        match event {
            Event::Keyboard(KeyEvent::KeyPressed {
                modified_key: Key::Named(key),
                modifiers,
                ..
            }) if state.is_focused && !matches!(key, Named::Space) => match key {
                Named::ArrowLeft => {
                    motion_modifiers(&mut editor, Motion::Left, modifiers);
                    status = Status::Captured;
                }
                Named::ArrowRight => {
                    motion_modifiers(&mut editor, Motion::Right, modifiers);
                    status = Status::Captured;
                }
                Named::ArrowUp => {
                    motion_modifiers(&mut editor, Motion::Up, modifiers);
                    status = Status::Captured;
                }
                Named::ArrowDown => {
                    motion_modifiers(&mut editor, Motion::Down, modifiers);
                    status = Status::Captured;
                }
                Named::Home => {
                    motion_modifiers(&mut editor, Motion::Home, modifiers);
                    status = Status::Captured;
                }
                Named::End => {
                    motion_modifiers(&mut editor, Motion::End, modifiers);
                    status = Status::Captured;
                }
                Named::PageUp => {
                    motion_modifiers(&mut editor, Motion::PageUp, modifiers);
                    status = Status::Captured;
                }
                Named::PageDown => {
                    motion_modifiers(&mut editor, Motion::PageDown, modifiers);
                    status = Status::Captured;
                }
                Named::Escape => {
                    editor.action(Action::Escape);
                    status = Status::Captured;
                }
                Named::Enter => {
                    editor.action(Action::Enter);
                    status = Status::Captured;
                }
                Named::Backspace => {
                    delete_modifiers(&mut editor, Motion::LeftWord, modifiers);
                    editor.action(Action::Backspace);
                    status = Status::Captured;
                }
                Named::Delete => {
                    delete_modifiers(&mut editor, Motion::RightWord, modifiers);
                    editor.action(Action::Delete);
                    status = Status::Captured;
                }
                Named::Tab => {
                    if !modifiers.control() && !modifiers.alt() {
                        if modifiers.shift() {
                            editor.action(Action::Unindent);
                        } else {
                            editor.action(Action::Indent);
                        }
                        status = Status::Captured;
                    }
                }
                _ => (),
            },
            Event::Keyboard(KeyEvent::KeyPressed { text, .. }) if state.is_focused => {
                let character = text.unwrap_or_default().chars().next().unwrap_or_default();
                // Only parse keys when Super, Ctrl, and Alt are not pressed
                if !state.modifiers.logo() && !state.modifiers.control() && !state.modifiers.alt() {
                    if !character.is_control() {
                        editor.action(Action::Insert(character));
                    }
                    status = Status::Captured;
                }
            }
            Event::Keyboard(KeyEvent::ModifiersChanged(modifiers)) => {
                state.modifiers = modifiers;
            }
            Event::Mouse(MouseEvent::ButtonPressed(button)) => {
                if let Some(p) = cursor_position.position_in(layout.bounds()) {
                    state.is_focused = true;

                    if let Some(on_focus) = self.on_focus.as_ref() {
                        shell.publish(on_focus.clone());
                    }

                    // Handle left click drag
                    if let Button::Left = button {
                        let x_logical = p.x - self.padding.left;
                        let y_logical = p.y - self.padding.top;
                        let mut x = x_logical * scale_factor - editor_offset_x as f32;
                        let y = y_logical * scale_factor;

                        // Do this first as the horizontal scrollbar is on top of the buffer
                        if let Some(scrollbar_h_rect) = state.scrollbar_h_rect.get() {
                            if scrollbar_h_rect.contains(Point::new(x_logical, y_logical)) {
                                state.dragging = Some(Dragging::ScrollbarH { start_x: x });
                            }
                        }

                        if matches!(state.dragging, Some(Dragging::ScrollbarH { .. })) {
                            // The horizontal scrollbar is on top of the buffer,
                            // so we need to ignore clicks when it is being dragged
                        } else if x >= 0.0
                            && x < buffer_size.0.unwrap_or(0.0)
                            && y >= 0.0
                            && y < buffer_size.1.unwrap_or(0.0)
                        {
                            x += buffer_scroll.horizontal;
                            let click_kind =
                                if let Some((click_kind, click_time)) = state.click.take() {
                                    if click_time.elapsed() < self.click_timing {
                                        match click_kind {
                                            ClickKind::Single => ClickKind::Double,
                                            ClickKind::Double => ClickKind::Triple,
                                            ClickKind::Triple => ClickKind::Single,
                                        }
                                    } else {
                                        ClickKind::Single
                                    }
                                } else {
                                    ClickKind::Single
                                };
                            match click_kind {
                                ClickKind::Single => editor.action(Action::Click {
                                    x: x as i32,
                                    y: y as i32,
                                }),
                                ClickKind::Double => editor.action(Action::DoubleClick {
                                    x: x as i32,
                                    y: y as i32,
                                }),
                                ClickKind::Triple => editor.action(Action::TripleClick {
                                    x: x as i32,
                                    y: y as i32,
                                }),
                            }
                            state.click = Some((click_kind, Instant::now()));
                            state.dragging = Some(Dragging::Buffer);
                        } else if scrollbar_v_rect.contains(Point::new(x_logical, y_logical)) {
                            state.dragging = Some(Dragging::ScrollbarV {
                                start_y: y,
                                start_scroll: editor.with_buffer(|buffer| buffer.scroll()),
                            });
                        } else if x_logical >= scrollbar_v_rect.x
                            && x_logical < (scrollbar_v_rect.x + scrollbar_v_rect.width)
                        {
                            editor.with_buffer_mut(|buffer| {
                                let mut scroll = buffer.scroll();
                                //TODO: if buffer height is undefined, what should this do?
                                let scroll_line = ((y / buffer.size().1.unwrap_or(1.0))
                                    * buffer.lines.len() as f32)
                                    as i32;
                                scroll.line = scroll_line.try_into().unwrap_or_default();
                                buffer.set_scroll(scroll);
                                state.dragging = Some(Dragging::ScrollbarV {
                                    start_y: y,
                                    start_scroll: buffer.scroll(),
                                });
                            });
                        }
                    }

                    // Update context menu state
                    if let Some(on_context_menu) = &self.on_context_menu {
                        shell.publish((on_context_menu)(if self.has_context_menu {
                            None
                        } else {
                            match button {
                                Button::Right => Some(p),
                                _ => None,
                            }
                        }));
                    }

                    status = Status::Captured;
                } else {
                    state.is_focused = false;
                }
            }
            Event::Mouse(MouseEvent::ButtonReleased(Button::Left)) => {
                state.dragging = None;
                status = Status::Captured;
                if let Some(on_auto_scroll) = &self.on_auto_scroll {
                    shell.publish(on_auto_scroll(None));
                }
            }
            Event::Mouse(MouseEvent::CursorMoved { .. }) => {
                if let Some(dragging) = &state.dragging {
                    if let Some(p) = cursor_position.position() {
                        let x_logical = (p.x - layout.bounds().x) - self.padding.left;
                        let y_logical = (p.y - layout.bounds().y) - self.padding.top;
                        let mut x = x_logical * scale_factor - editor_offset_x as f32;
                        let y = y_logical * scale_factor;
                        match dragging {
                            Dragging::Buffer => {
                                x += buffer_scroll.horizontal;
                                editor.action(Action::Drag {
                                    x: x as i32,
                                    y: y as i32,
                                });
                                let auto_scroll = editor.with_buffer(|buffer| {
                                    //TODO: ideal auto scroll speed
                                    let speed = 10.0;
                                    if y < 0.0 {
                                        Some(y * speed)
                                    } else if y > buffer.size().1.unwrap_or(0.0) {
                                        Some((y - buffer.size().1.unwrap_or(0.0)) * speed)
                                    } else {
                                        None
                                    }
                                });
                                if let Some(on_auto_scroll) = &self.on_auto_scroll {
                                    shell.publish(on_auto_scroll(auto_scroll));
                                }
                            }
                            Dragging::ScrollbarV {
                                start_y,
                                start_scroll,
                            } => {
                                editor.with_buffer_mut(|buffer| {
                                    let mut scroll = buffer.scroll();
                                    //TODO: if buffer size is undefined, what should this do?
                                    let scroll_offset = (((y - start_y)
                                        / buffer.size().1.unwrap_or(1.0))
                                        * buffer.lines.len() as f32)
                                        as i32;
                                    scroll.line = (start_scroll.line as i32 + scroll_offset)
                                        .try_into()
                                        .unwrap_or_default();
                                    buffer.set_scroll(scroll);
                                });
                            }
                            Dragging::ScrollbarH { start_x } => {
                                editor.with_buffer_mut(|buffer| {
                                    //TODO: store this in state?
                                    let mut max_line_width = 0.0;
                                    for run in buffer.layout_runs() {
                                        if run.line_w > max_line_width {
                                            max_line_width = run.line_w;
                                        }
                                    }

                                    let buffer_w = buffer.size().0.unwrap_or(0.0);
                                    let mut scroll = buffer.scroll();
                                    scroll.horizontal = (((x - start_x) / buffer_w)
                                        * max_line_width)
                                        .max(0.0)
                                        .min(max_line_width - buffer_w);
                                    buffer.set_scroll(scroll);
                                });
                            }
                        }
                    }
                    status = Status::Captured;
                }
            }
            Event::Mouse(MouseEvent::WheelScrolled { delta }) => {
                if let Some(_p) = cursor_position.position_in(layout.bounds()) {
                    let pixels = match delta {
                        ScrollDelta::Lines { x: _, y } => {
                            //TODO: this adjustment is just a guess!
                            let metrics = editor.with_buffer(|buffer| buffer.metrics());
                            -y * metrics.line_height
                        }
                        ScrollDelta::Pixels { x: _, y } => -y,
                    } * 4.0;
                    editor.action(Action::Scroll { pixels });
                    status = Status::Captured;
                }
            }
            _ => (),
        }

        if let Some(on_changed) = &self.on_changed {
            //TODO: better handling of status line update
            let parser = editor.parser();
            if editor.changed() != last_changed
                || (&parser.mode, &parser.cmd) != (&last_parser_mode, &last_parser_cmd)
            {
                shell.publish(on_changed.clone());
            }
        }

        status
    }
}

impl<'a, Message> From<TextBox<'a, Message>> for Element<'a, Message, cosmic::Theme, Renderer>
where
    Message: Clone + 'a,
{
    fn from(text_box: TextBox<'a, Message>) -> Self {
        Self::new(text_box)
    }
}

enum ClickKind {
    Single,
    Double,
    Triple,
}

enum Dragging {
    Buffer,
    ScrollbarV { start_y: f32, start_scroll: Scroll },
    ScrollbarH { start_x: f32 },
}

pub struct State {
    modifiers: Modifiers,
    click: Option<(ClickKind, Instant)>,
    dragging: Option<Dragging>,
    editor_offset_x: Cell<i32>,
    is_focused: bool,
    emit_focus: bool,
    scale_factor: Cell<f32>,
    scrollbar_v_rect: Cell<Rectangle<f32>>,
    scrollbar_h_rect: Cell<Option<Rectangle<f32>>>,
    gutter_handle_opt: Mutex<Option<image::Handle>>,
}

impl State {
    /// Creates a new [`State`].
    pub fn new() -> State {
        State {
            modifiers: Modifiers::empty(),
            click: None,
            dragging: None,
            editor_offset_x: Cell::new(0),
            is_focused: false,
            emit_focus: false,
            scale_factor: Cell::new(1.0),
            scrollbar_v_rect: Cell::new(Rectangle::default()),
            scrollbar_h_rect: Cell::new(None),
            gutter_handle_opt: Mutex::new(None),
        }
    }
}

impl operation::Focusable for State {
    fn is_focused(&self) -> bool {
        self.is_focused
    }

    fn focus(&mut self) {
        self.is_focused = true;
        self.emit_focus = true;
    }

    fn unfocus(&mut self) {
        self.is_focused = false;
    }
}
