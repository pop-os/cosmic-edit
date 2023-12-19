// SPDX-License-Identifier: GPL-3.0-only

use cosmic::{
    iced::{
        event::{Event, Status},
        keyboard::{Event as KeyEvent, KeyCode, Modifiers},
        mouse::{self, Button, Event as MouseEvent, ScrollDelta},
        Color, Element, Length, Padding, Point, Rectangle, Size, Vector,
    },
    iced_core::{
        clipboard::Clipboard,
        image,
        layout::{self, Layout},
        renderer::{self, Quad},
        widget::{self, tree, Widget},
        Shell,
    },
};
use cosmic_text::{Action, Edit, Metrics, Motion, Scroll, ViEditor};
use std::{
    cell::Cell,
    cmp,
    sync::Mutex,
    time::{Duration, Instant},
};

use crate::{line_number::LineNumberKey, FONT_SYSTEM, LINE_NUMBER_CACHE, SWASH_CACHE};

pub struct TextBox<'a, Message> {
    editor: &'a Mutex<ViEditor<'static, 'static>>,
    metrics: Metrics,
    padding: Padding,
    on_changed: Option<Message>,
    click_timing: Duration,
    context_menu: Option<Point>,
    on_context_menu: Option<Box<dyn Fn(Option<Point>) -> Message + 'a>>,
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
            padding: Padding::new(0.0),
            on_changed: None,
            click_timing: Duration::from_millis(500),
            context_menu: None,
            on_context_menu: None,
            line_numbers: false,
        }
    }

    pub fn padding<P: Into<Padding>>(mut self, padding: P) -> Self {
        self.padding = padding.into();
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

    pub fn context_menu(mut self, position: Point) -> Self {
        self.context_menu = Some(position);
        self
    }

    pub fn on_context_menu(
        mut self,
        on_context_menu: impl Fn(Option<Point>) -> Message + 'a,
    ) -> Self {
        self.on_context_menu = Some(Box::new(on_context_menu));
        self
    }

    pub fn line_numbers(mut self) -> Self {
        self.line_numbers = true;
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

//TODO: improve performance
fn draw_rect(
    buffer: &mut [u32],
    image_w: i32,
    image_h: i32,
    start_x: i32,
    start_y: i32,
    w: i32,
    h: i32,
    cosmic_color: cosmic_text::Color,
) {
    // Grab alpha channel and green channel
    let mut color = cosmic_color.0 & 0xFF00FF00;
    // Shift red channel
    color |= (cosmic_color.0 & 0x00FF0000) >> 16;
    // Shift blue channel
    color |= (cosmic_color.0 & 0x000000FF) << 16;

    let alpha = (color >> 24) & 0xFF;
    if alpha == 0 {
        // Do not draw if alpha is zero
        return;
    } else if alpha >= 255 {
        // Handle overwrite
        for y in start_y..start_y + h {
            if y < 0 || y >= image_h {
                // Skip if y out of bounds
                continue;
            }

            let line_offset = y as usize * image_w as usize;
            for x in start_x..start_x + w {
                if x < 0 || x >= image_w {
                    // Skip if x out of bounds
                    continue;
                }

                let offset = line_offset + x as usize;
                buffer[offset] = color;
            }
        }
    } else {
        let n_alpha = 255 - alpha;
        for y in start_y..start_y + h {
            if y < 0 || y >= image_h {
                // Skip if y out of bounds
                continue;
            }

            let line_offset = y as usize * image_w as usize;
            for x in start_x..start_x + w {
                if x < 0 || x >= image_w {
                    // Skip if x out of bounds
                    continue;
                }

                // Alpha blend with current value
                let offset = line_offset + x as usize;
                let current = buffer[offset];
                if current & 0xFF000000 == 0 {
                    // Overwrite if buffer empty
                    buffer[offset] = color;
                } else {
                    let rb =
                        ((n_alpha * (current & 0x00FF00FF)) + (alpha * (color & 0x00FF00FF))) >> 8;
                    let ag = (n_alpha * ((current & 0xFF00FF00) >> 8))
                        + (alpha * (0x01000000 | ((color & 0x0000FF00) >> 8)));
                    buffer[offset] = (rb & 0x00FF00FF) | (ag & 0xFF00FF00);
                }
            }
        }
    }
}

impl<'a, 'editor, Message, Renderer> Widget<Message, Renderer> for TextBox<'a, Message>
where
    Message: Clone,
    Renderer: renderer::Renderer + image::Renderer<Handle = image::Handle>,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::new())
    }

    fn width(&self) -> Length {
        Length::Fill
    }

    fn height(&self) -> Length {
        Length::Fill
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
            .borrow_with(&mut FONT_SYSTEM.lock().unwrap())
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

            layout::Node::new(limits.resolve(size))
        })
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

        match &state.dragging {
            Some(Dragging::Scrollbar { .. }) => return mouse::Interaction::Idle,
            _ => {}
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
            if x >= 0.0 && x < buffer_size.0 && y >= 0.0 && y < buffer_size.1 {
                return mouse::Interaction::Text;
            }
        }

        mouse::Interaction::Idle
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        _theme: &Renderer::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        _cursor_position: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let instant = Instant::now();

        let state = tree.state.downcast_ref::<State>();

        let mut editor = self.editor.lock().unwrap();

        let view_w = cmp::min(viewport.width as i32, layout.bounds().width as i32)
            - self.padding.horizontal() as i32;
        let view_h = cmp::min(viewport.height as i32, layout.bounds().height as i32)
            - self.padding.vertical() as i32;

        let scale_factor = style.scale_factor as f32;
        let metrics = self.metrics.scale(scale_factor);

        let image_w = (view_w as f32 * scale_factor) as i32;
        let image_h = (view_h as f32 * scale_factor) as i32;

        //TODO: make this configurable and do not repeat
        let scrollbar_w = (8.0 * scale_factor) as i32;

        if image_w <= scrollbar_w || image_h <= 0 {
            // Zero sized image
            return;
        }

        // Adjust image width by scrollbar width
        let image_w = image_w - scrollbar_w;

        // Lock font system (used throughout)
        let mut font_system = FONT_SYSTEM.lock().unwrap();

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
                let mut line_number_cache = LINE_NUMBER_CACHE.lock().unwrap();
                if let Some(layout_line) = line_number_cache
                    .get(
                        &mut font_system,
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
                &mut font_system,
                metrics,
                (image_w - editor_offset_x) as f32,
                image_h as f32,
            )
        });

        // Shape and layout as needed
        editor.shape_as_needed(&mut font_system, true);

        let mut handle_opt = state.handle_opt.lock().unwrap();
        if editor.redraw() || handle_opt.is_none() {
            // Draw to pixel buffer
            let mut pixels_u8 = vec![0; image_w as usize * image_h as usize * 4];
            {
                let mut swash_cache = SWASH_CACHE.lock().unwrap();

                let pixels = unsafe {
                    std::slice::from_raw_parts_mut(
                        pixels_u8.as_mut_ptr() as *mut u32,
                        pixels_u8.len() / 4,
                    )
                };

                if self.line_numbers {
                    let (gutter, gutter_foreground) = {
                        let convert_color = |color: syntect::highlighting::Color| {
                            cosmic_text::Color::rgba(color.r, color.g, color.b, color.a)
                        };
                        let syntax_theme = editor.theme();
                        let gutter = syntax_theme
                            .settings
                            .gutter
                            .map_or(editor.background_color(), convert_color);
                        let gutter_foreground = syntax_theme
                            .settings
                            .gutter_foreground
                            .map_or(editor.foreground_color(), convert_color);
                        (gutter, gutter_foreground)
                    };

                    // Ensure fill with gutter color
                    draw_rect(
                        pixels,
                        image_w,
                        image_h,
                        0,
                        0,
                        editor_offset_x,
                        image_h,
                        gutter,
                    );

                    // Draw line numbers
                    //TODO: move to cosmic-text?
                    editor.with_buffer(|buffer| {
                        let mut line_number_cache = LINE_NUMBER_CACHE.lock().unwrap();
                        let mut last_line_number = 0;
                        for run in buffer.layout_runs() {
                            let line_number = run.line_i.saturating_add(1);
                            if line_number == last_line_number {
                                // Skip duplicate lines
                                continue;
                            } else {
                                last_line_number = line_number;
                            }

                            if let Some(layout_line) = line_number_cache
                                .get(
                                    &mut font_system,
                                    LineNumberKey {
                                        number: line_number,
                                        width: line_number_chars,
                                    },
                                )
                                .first()
                            {
                                // These values must be scaled since layout is done at font size 1.0
                                let max_ascent = layout_line.max_ascent * metrics.font_size;
                                let max_descent = layout_line.max_descent * metrics.font_size;

                                // This code comes from cosmic_text::LayoutRunIter
                                let glyph_height = max_ascent + max_descent;
                                let centering_offset = (metrics.line_height - glyph_height) / 2.0;
                                let line_y = run.line_top + centering_offset + max_ascent;

                                for layout_glyph in layout_line.glyphs.iter() {
                                    let physical_glyph =
                                        layout_glyph.physical((0., line_y), metrics.font_size);

                                    swash_cache.with_pixels(
                                        &mut font_system,
                                        physical_glyph.cache_key,
                                        gutter_foreground,
                                        |x, y, color| {
                                            draw_rect(
                                                pixels,
                                                image_w,
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

                // Draw editor
                editor.draw(&mut font_system, &mut swash_cache, |x, y, w, h, color| {
                    draw_rect(
                        pixels,
                        image_w,
                        image_h,
                        editor_offset_x + x,
                        y,
                        w as i32,
                        h as i32,
                        color,
                    );
                });

                // Calculate scrollbar
                editor.with_buffer(|buffer| {
                    let mut start_line_opt = None;
                    let mut end_line = 0;
                    for run in buffer.layout_runs() {
                        end_line = run.line_i;
                        if start_line_opt.is_none() {
                            start_line_opt = Some(end_line);
                        }
                    }

                    let start_line = start_line_opt.unwrap_or(end_line);
                    let lines = buffer.lines.len();
                    let start_y = (start_line * image_h as usize) / lines;
                    let end_y = ((end_line + 1) * image_h as usize) / lines;

                    let rect = Rectangle::new(
                        [image_w as f32 / scale_factor, start_y as f32 / scale_factor].into(),
                        Size::new(
                            scrollbar_w as f32 / scale_factor,
                            (end_y as f32 - start_y as f32) / scale_factor,
                        ),
                    );
                    state.scrollbar_rect.set(rect);
                });
            }

            // Clear redraw flag
            editor.set_redraw(false);

            state.scale_factor.set(scale_factor);
            *handle_opt = Some(image::Handle::from_pixels(
                image_w as u32,
                image_h as u32,
                pixels_u8,
            ));
        }

        let image_position =
            layout.position() + [self.padding.left as f32, self.padding.top as f32].into();
        if let Some(ref handle) = *handle_opt {
            let image_size = image::Renderer::dimensions(renderer, &handle);
            image::Renderer::draw(
                renderer,
                handle.clone(),
                image::FilterMethod::Nearest,
                Rectangle::new(
                    image_position,
                    Size::new(
                        image_size.width as f32 / scale_factor,
                        image_size.height as f32 / scale_factor,
                    ),
                ),
                [0.0; 4],
            );
        }

        // Draw scrollbar
        let scrollbar_alpha = match &state.dragging {
            Some(Dragging::Scrollbar { .. }) => 0.5,
            _ => 0.25,
        };
        renderer.fill_quad(
            Quad {
                bounds: state.scrollbar_rect.get()
                    + Vector::new(image_position.x, image_position.y),
                border_radius: 0.0.into(),
                border_width: 0.0,
                border_color: Color::TRANSPARENT,
            },
            Color::new(1.0, 1.0, 1.0, scrollbar_alpha),
        );

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
        let scrollbar_rect = state.scrollbar_rect.get();
        let mut editor = self.editor.lock().unwrap();
        let buffer_size = editor.with_buffer(|buffer| buffer.size());
        let last_changed = editor.changed();
        let mut font_system = FONT_SYSTEM.lock().unwrap();
        let mut editor = editor.borrow_with(&mut font_system);

        let mut status = Status::Ignored;
        match event {
            Event::Keyboard(KeyEvent::KeyPressed {
                key_code,
                modifiers,
            }) => match key_code {
                KeyCode::Left => {
                    editor.action(Action::Motion(Motion::Left));
                    status = Status::Captured;
                }
                KeyCode::Right => {
                    editor.action(Action::Motion(Motion::Right));
                    status = Status::Captured;
                }
                KeyCode::Up => {
                    editor.action(Action::Motion(Motion::Up));
                    status = Status::Captured;
                }
                KeyCode::Down => {
                    editor.action(Action::Motion(Motion::Down));
                    status = Status::Captured;
                }
                KeyCode::Home => {
                    editor.action(Action::Motion(Motion::Home));
                    status = Status::Captured;
                }
                KeyCode::End => {
                    editor.action(Action::Motion(Motion::End));
                    status = Status::Captured;
                }
                KeyCode::PageUp => {
                    editor.action(Action::Motion(Motion::PageUp));
                    status = Status::Captured;
                }
                KeyCode::PageDown => {
                    editor.action(Action::Motion(Motion::PageDown));
                    status = Status::Captured;
                }
                KeyCode::Escape => {
                    editor.action(Action::Escape);
                    status = Status::Captured;
                }
                KeyCode::Enter => {
                    editor.action(Action::Enter);
                    status = Status::Captured;
                }
                KeyCode::Backspace => {
                    editor.action(Action::Backspace);
                    status = Status::Captured;
                }
                KeyCode::Delete => {
                    editor.action(Action::Delete);
                    status = Status::Captured;
                }
                KeyCode::Tab => {
                    if modifiers.shift() {
                        editor.action(Action::Unindent);
                    } else {
                        editor.action(Action::Indent);
                    }
                    status = Status::Captured;
                }
                _ => (),
            },
            Event::Keyboard(KeyEvent::ModifiersChanged(modifiers)) => {
                state.modifiers = modifiers;
            }
            Event::Keyboard(KeyEvent::CharacterReceived(character)) => {
                // Only parse keys when Super, Ctrl, and Alt are not pressed
                if !state.modifiers.logo() && !state.modifiers.control() && !state.modifiers.alt() {
                    if !character.is_control() {
                        editor.action(Action::Insert(character));
                    }
                    status = Status::Captured;
                }
            }
            Event::Mouse(MouseEvent::ButtonPressed(button)) => {
                if let Some(p) = cursor_position.position_in(layout.bounds()) {
                    // Handle left click drag
                    if let Button::Left = button {
                        let x_logical = p.x - self.padding.left;
                        let y_logical = p.y - self.padding.top;
                        let x = x_logical * scale_factor - editor_offset_x as f32;
                        let y = y_logical * scale_factor;
                        if x >= 0.0 && x < buffer_size.0 && y >= 0.0 && y < buffer_size.1 {
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
                        } else if scrollbar_rect.contains(Point::new(x_logical, y_logical)) {
                            state.dragging = Some(Dragging::Scrollbar {
                                start_y: y,
                                start_scroll: editor.with_buffer(|buffer| buffer.scroll()),
                            });
                        } else if x_logical >= scrollbar_rect.x
                            && x_logical < (scrollbar_rect.x + scrollbar_rect.width)
                        {
                            editor.with_buffer_mut(|buffer| {
                                let scroll_line =
                                    ((y / buffer.size().1) * buffer.lines.len() as f32) as i32;
                                buffer.set_scroll(Scroll::new(
                                    scroll_line.try_into().unwrap_or_default(),
                                    0,
                                ));
                                state.dragging = Some(Dragging::Scrollbar {
                                    start_y: y,
                                    start_scroll: buffer.scroll(),
                                });
                            });
                        }
                    }

                    // Update context menu state
                    if let Some(on_context_menu) = &self.on_context_menu {
                        shell.publish((on_context_menu)(match self.context_menu {
                            Some(_) => None,
                            None => match button {
                                Button::Right => Some(p),
                                _ => None,
                            },
                        }));
                    }

                    status = Status::Captured;
                }
            }
            Event::Mouse(MouseEvent::ButtonReleased(Button::Left)) => {
                state.dragging = None;
                status = Status::Captured;
            }
            Event::Mouse(MouseEvent::CursorMoved { .. }) => {
                if let Some(dragging) = &state.dragging {
                    if let Some(p) = cursor_position.position() {
                        let x_logical = (p.x - layout.bounds().x) - self.padding.left;
                        let y_logical = (p.y - layout.bounds().y) - self.padding.top;
                        let x = x_logical * scale_factor - editor_offset_x as f32;
                        let y = y_logical * scale_factor;
                        match dragging {
                            Dragging::Buffer => {
                                editor.action(Action::Drag {
                                    x: x as i32,
                                    y: y as i32,
                                });
                            }
                            Dragging::Scrollbar {
                                start_y,
                                start_scroll,
                            } => {
                                editor.with_buffer_mut(|buffer| {
                                    let scroll_offset = (((y - start_y) / buffer.size().1)
                                        * buffer.lines.len() as f32)
                                        as i32;
                                    buffer.set_scroll(Scroll::new(
                                        (start_scroll.line as i32 + scroll_offset)
                                            .try_into()
                                            .unwrap_or_default(),
                                        0,
                                    ));
                                });
                            }
                        }
                    }
                    status = Status::Captured;
                }
            }
            Event::Mouse(MouseEvent::WheelScrolled { delta }) => {
                if let Some(_p) = cursor_position.position_in(layout.bounds()) {
                    match delta {
                        ScrollDelta::Lines { x, y } => {
                            //TODO: this adjustment is just a guess!
                            state.scroll_pixels = 0.0;
                            let lines = (-y * 6.0) as i32;
                            if lines != 0 {
                                editor.action(Action::Scroll { lines });
                            }
                            status = Status::Captured;
                        }
                        ScrollDelta::Pixels { x, y } => {
                            //TODO: this adjustment is just a guess!
                            state.scroll_pixels -= y * 6.0;
                            let mut lines = 0;
                            let metrics = editor.with_buffer(|buffer| buffer.metrics());
                            while state.scroll_pixels <= -metrics.line_height {
                                lines -= 1;
                                state.scroll_pixels += metrics.line_height;
                            }
                            while state.scroll_pixels >= metrics.line_height {
                                lines += 1;
                                state.scroll_pixels -= metrics.line_height;
                            }
                            if lines != 0 {
                                editor.action(Action::Scroll { lines });
                            }
                            status = Status::Captured;
                        }
                    }
                }
            }
            _ => (),
        }

        if editor.changed() != last_changed {
            if let Some(on_changed) = &self.on_changed {
                shell.publish(on_changed.clone());
            }
        }

        status
    }
}

impl<'a, 'editor, Message, Renderer> From<TextBox<'a, Message>> for Element<'a, Message, Renderer>
where
    Message: Clone + 'a,
    Renderer: renderer::Renderer + image::Renderer<Handle = image::Handle>,
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
    Scrollbar { start_y: f32, start_scroll: Scroll },
}

pub struct State {
    modifiers: Modifiers,
    click: Option<(ClickKind, Instant)>,
    dragging: Option<Dragging>,
    editor_offset_x: Cell<i32>,
    scale_factor: Cell<f32>,
    scroll_pixels: f32,
    scrollbar_rect: Cell<Rectangle<f32>>,
    handle_opt: Mutex<Option<image::Handle>>,
}

impl State {
    /// Creates a new [`State`].
    pub fn new() -> State {
        State {
            modifiers: Modifiers::empty(),
            click: None,
            dragging: None,
            editor_offset_x: Cell::new(0),
            scale_factor: Cell::new(1.0),
            scroll_pixels: 0.0,
            scrollbar_rect: Cell::new(Rectangle::default()),
            handle_opt: Mutex::new(None),
        }
    }
}
