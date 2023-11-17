use cosmic::widget::icon;
use std::{collections::HashMap, path::Path, sync::Mutex};

pub const FALLBACK_MIME_ICON: &str = "text-x-generic";

#[derive(Debug, Eq, Hash, PartialEq)]
struct MimeIconKey {
    path: String,
    size: u16,
}

struct MimeIconCache {
    cache: HashMap<MimeIconKey, Option<icon::Handle>>,
}

impl MimeIconCache {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    pub fn get(&mut self, key: MimeIconKey) -> Option<icon::Handle> {
        self.cache
            .entry(key)
            .or_insert_with_key(|key| match systemicons::get_icon(&key.path, key.size) {
                Ok(icon_kind) => match icon_kind {
                    systemicons::Icon::Png(bytes) => Some(icon::from_raster_bytes(bytes)),
                    systemicons::Icon::Svg(bytes) => Some(icon::from_svg_bytes(bytes)),
                },
                Err(err) => {
                    log::warn!("failed to get icon for {:?}: {:?}", key, err);
                    None
                }
            })
            .clone()
    }
}

lazy_static::lazy_static! {
    static ref MIME_ICON_CACHE: Mutex<MimeIconCache> = Mutex::new(MimeIconCache::new());
}

pub fn mime_icon<P: AsRef<Path>>(path: P, size: u16) -> icon::Icon {
    //TODO: smarter path handling
    let path = path
        .as_ref()
        .to_str()
        .expect("failed to convert path to UTF-8")
        .to_owned();
    let mut mime_icon_cache = MIME_ICON_CACHE.lock().unwrap();
    match mime_icon_cache.get(MimeIconKey { path, size }) {
        Some(handle) => icon::icon(handle).size(size),
        None => icon::from_name(FALLBACK_MIME_ICON).size(size).icon(),
    }
}
