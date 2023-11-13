use cosmic::widget::icon;
use std::path::Path;

pub const FALLBACK_MIME_ICON: &str = "text-x-generic";

pub fn mime_icon<P: AsRef<Path>>(path: P, size: u16) -> icon::Icon {
    let path = path.as_ref();
    for mime in mime_guess::from_path(path).iter() {
        //TODO: correct some common issues (like application/x-sh not being found)
        let icon_name = mime.essence_str().replace("/", "-");
        let named = icon::from_name(icon_name).size(size);
        if named.clone().path().is_some() {
            return named.icon();
        }
    }
    icon::from_name(FALLBACK_MIME_ICON).size(size).icon()
}
