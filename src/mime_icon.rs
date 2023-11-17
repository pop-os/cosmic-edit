use cosmic::widget::icon;
use std::path::Path;

pub const FALLBACK_MIME_ICON: &str = "text-x-generic";

lazy_static::lazy_static! {
    static ref SHARED_MIME_INFO: xdg_mime::SharedMimeInfo = xdg_mime::SharedMimeInfo::new();
}

pub fn mime_icon<P: AsRef<Path>>(path: P, size: u16) -> icon::Icon {
    let path = path.as_ref();
    //TODO: SHARED_MIME_INFO.get_mime_types_from_file_name(path)
    for mime_type in mime_guess::from_path(path) {
        for icon_name in SHARED_MIME_INFO.lookup_icon_names(&mime_type) {
            let named = icon::from_name(icon_name).size(size);
            if named.clone().path().is_some() {
                return named.icon();
            }
        }

        let icon_name = mime_type.essence_str().replace("/", "-");
        let named = icon::from_name(icon_name).size(size);
        if named.clone().path().is_some() {
            return named.icon();
        }
    }
    icon::from_name(FALLBACK_MIME_ICON).size(size).icon()
}
