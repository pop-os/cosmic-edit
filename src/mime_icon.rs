// SPDX-License-Identifier: GPL-3.0-only

use cosmic::widget::icon;
use mime_guess::Mime;
use std::{
    collections::HashMap,
    path::Path,
    sync::{LazyLock, Mutex},
};
use xdg_mime::SharedMimeInfo;

pub const FALLBACK_MIME_ICON: &str = "text-x-generic";

static SHARED_MIME_INFO: LazyLock<SharedMimeInfo> = LazyLock::new(SharedMimeInfo::new);
static MIME_ICON_CACHE: LazyLock<Mutex<HashMap<(Mime, u16), Option<icon::Handle>>>> =
    LazyLock::new(Mutex::default);

pub fn mime_for_path(path: impl AsRef<Path>) -> Mime {
    let path = path.as_ref();
    let mut gb = SHARED_MIME_INFO.guess_mime_type();
    gb.zero_size(false);
    gb.path(path);
    let guess = gb.guess();
    if guess.uncertain() {
        // Platforms without shared-mime-info, or files xdg-mime cannot classify
        mime_guess::from_path(path).first_or_octet_stream()
    } else {
        guess.mime_type().clone()
    }
}

pub fn mime_icon(mime: Mime, size: u16) -> icon::Handle {
    let mut cache = MIME_ICON_CACHE.lock().unwrap();
    cache
        .entry((mime, size))
        .or_insert_with_key(|(mime, size)| {
            let mut names = SHARED_MIME_INFO.lookup_icon_names(mime);
            if names.is_empty() {
                return None;
            }
            let mut named = icon::from_name(names.remove(0))
                .prefer_svg(true)
                .size(*size);
            if !names.is_empty() {
                let fallbacks = names.into_iter().map(std::borrow::Cow::from).collect();
                named = named.fallback(Some(icon::IconFallback::Names(fallbacks)));
            }
            Some(named.handle())
        })
        .clone()
        .unwrap_or_else(|| {
            icon::from_name(FALLBACK_MIME_ICON)
                .prefer_svg(true)
                .size(size)
                .handle()
        })
}
