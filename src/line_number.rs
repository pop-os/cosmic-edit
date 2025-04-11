use cosmic_text::{
    Align, Attrs, AttrsList, BufferLine, Family, FontSystem, LayoutLine, LineEnding, Shaping, Wrap,
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
}

impl LineNumberCache {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
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
            let mut buffer_line = BufferLine::new(
                text,
                LineEnding::default(),
                AttrsList::new(&attrs),
                Shaping::Advanced,
            );
            buffer_line.set_align(Some(Align::Left));
            buffer_line
                .layout(
                    font_system,
                    1.0, /* font size adjusted later */
                    None,
                    Wrap::None,
                    None,
                    8, /* default tab width */
                )
                .to_vec()
        })
    }
}
