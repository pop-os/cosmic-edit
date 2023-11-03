use cosmic::iced::keyboard::{KeyCode, Modifiers};
use cosmic_text::Metrics;
use std::{collections::HashMap, fmt};

use crate::{ContextPage, Message};

// Makes key binding definitions simpler
const CTRL: Modifiers = Modifiers::CTRL;
const ALT: Modifiers = Modifiers::ALT;
const SHIFT: Modifiers = Modifiers::SHIFT;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct KeyBind {
    pub modifiers: Modifiers,
    pub key_code: KeyCode,
}

impl KeyBind {
    //TODO: load from config
    pub fn load() -> HashMap<KeyBind, Message> {
        let mut keybinds = HashMap::new();

        macro_rules! bind {
            ($modifiers:expr, $key_code:ident, $message:expr) => {{
                keybinds.insert(
                    KeyBind {
                        modifiers: $modifiers,
                        key_code: KeyCode::$key_code,
                    },
                    $message,
                );
            }};
        }

        bind!(CTRL, X, Message::Cut);
        bind!(CTRL, C, Message::Copy);
        bind!(CTRL, V, Message::Paste);
        bind!(CTRL, N, Message::NewFile);
        bind!(CTRL | SHIFT, N, Message::NewWindow);
        bind!(CTRL, O, Message::OpenFileDialog);
        bind!(CTRL, S, Message::Save);
        bind!(CTRL, Q, Message::Quit);
        bind!(
            CTRL,
            Comma,
            Message::ToggleContextPage(ContextPage::Settings)
        );
        bind!(ALT, Z, Message::ToggleWordWrap);

        keybinds
    }
}

impl fmt::Display for KeyBind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.modifiers.logo() {
            write!(f, "Super + ")?;
        }
        if self.modifiers.control() {
            write!(f, "Ctrl + ")?;
        }
        if self.modifiers.alt() {
            write!(f, "Alt + ")?;
        }
        if self.modifiers.shift() {
            write!(f, "Shift + ")?;
        }
        write!(f, "{:?}", self.key_code)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub font_size: u16,
    pub syntax_theme_dark: String,
    pub syntax_theme_light: String,
    pub vim_bindings: bool,
    pub word_wrap: bool,
    pub keybinds: HashMap<KeyBind, Message>,
}

impl Config {
    //TODO: load from cosmic-config
    pub fn load() -> Self {
        Self {
            font_size: 14,
            syntax_theme_dark: "base16-eighties.dark".to_string(),
            syntax_theme_light: "base16-ocean.light".to_string(),
            vim_bindings: false,
            word_wrap: false,
            keybinds: KeyBind::load(),
        }
    }

    // Calculate metrics from font size
    pub fn metrics(&self) -> Metrics {
        let font_size = self.font_size as f32;
        let line_height = (font_size * 1.4).ceil();
        Metrics::new(font_size, line_height)
    }

    // Get current syntax theme based on dark mode
    pub fn syntax_theme(&self, dark: bool) -> &str {
        if dark {
            &self.syntax_theme_dark
        } else {
            &self.syntax_theme_light
        }
    }
}
