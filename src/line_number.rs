use cosmic_text::{
    Align, Attrs, AttrsList, BufferLine, Family, FontSystem, LayoutLine, ShapeBuffer, Shaping, Wrap,
};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LineNumberKey {
    pub number: usize,
    pub width: usize,
}

#[derive(Debug)]
pub struct LineNumberCache {
    cache: HashMap<LineNumberKey, Vec<LayoutLine>>,
    scratch: ShapeBuffer,
}

impl LineNumberCache {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            scratch: ShapeBuffer::default(),
        }
    }

    pub fn clear(&mut self) {
        self.cache.clear();
    }

    pub fn get(&mut self, font_system: &mut FontSystem, key: LineNumberKey) -> &Vec<LayoutLine> {
        self.cache.entry(key).or_insert_with(|| {
            //TODO: do not repeat, used in App::init
            let attrs = Attrs::new().family(Family::Monospace);
            let text = format!("{:width$}", key.number, width = key.width);
            let mut buffer_line = BufferLine::new(text, AttrsList::new(attrs), Shaping::Advanced);
            buffer_line.set_align(Some(Align::Left));
            buffer_line
                .layout_in_buffer(
                    &mut self.scratch,
                    font_system,
                    1.0,    /* font size adjusted later */
                    1000.0, /* dummy width */
                    Wrap::None,
                    None,
                )
                .to_vec()
        })
    }
}
