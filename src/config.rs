use cosmic::{
    cosmic_config::{self, cosmic_config_derive::CosmicConfigEntry, CosmicConfigEntry},
    iced::keyboard::{KeyCode, Modifiers},
    theme,
};
use cosmic_text::Metrics;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fmt};

use crate::{ContextPage, Message};

pub const CONFIG_VERSION: u64 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Action {
    CloseFile,
    CloseProject,
    Copy,
    Cut,
    NewFile,
    NewWindow,
    OpenFileDialog,
    OpenProjectDialog,
    Paste,
    Quit,
    Redo,
    Save,
    SelectAll,
    ToggleSettingsPage,
    ToggleWordWrap,
    Undo,
}

impl Action {
    pub fn message(&self) -> Message {
        match self {
            Self::CloseFile => Message::CloseFile,
            Self::CloseProject => Message::CloseProject,
            Self::Copy => Message::Copy,
            Self::Cut => Message::Cut,
            Self::NewFile => Message::NewFile,
            Self::NewWindow => Message::NewWindow,
            Self::OpenFileDialog => Message::OpenFileDialog,
            Self::OpenProjectDialog => Message::OpenProjectDialog,
            Self::Paste => Message::Paste,
            Self::Quit => Message::Quit,
            Self::Redo => Message::Redo,
            Self::Save => Message::Save,
            Self::SelectAll => Message::SelectAll,
            Self::ToggleSettingsPage => Message::ToggleContextPage(ContextPage::Settings),
            Self::ToggleWordWrap => Message::ToggleWordWrap,
            Self::Undo => Message::Undo,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum AppTheme {
    Dark,
    Light,
    System,
}

impl AppTheme {
    pub fn theme(&self) -> theme::Theme {
        match self {
            Self::Dark => theme::Theme::dark(),
            Self::Light => theme::Theme::light(),
            Self::System => theme::system_preference(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Modifier {
    Super,
    Ctrl,
    Alt,
    Shift,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct KeyBind {
    pub modifiers: Vec<Modifier>,
    pub key_code: KeyCode,
}

impl KeyBind {
    //TODO: load from config
    pub fn load() -> HashMap<KeyBind, Action> {
        let mut keybinds = HashMap::new();

        macro_rules! bind {
            ([$($modifier:ident),+ $(,)?], $key_code:ident, $action:ident) => {{
                keybinds.insert(
                    KeyBind {
                        modifiers: vec![$(Modifier::$modifier),+],
                        key_code: KeyCode::$key_code,
                    },
                    Action::$action,
                );
            }};
        }

        bind!([Ctrl], W, CloseFile);
        bind!([Ctrl], X, Cut);
        bind!([Ctrl], C, Copy);
        bind!([Ctrl], V, Paste);
        bind!([Ctrl], T, NewFile);
        bind!([Ctrl], N, NewWindow);
        bind!([Ctrl], O, OpenFileDialog);
        bind!([Ctrl, Shift], O, OpenProjectDialog);
        bind!([Ctrl], Q, Quit);
        bind!([Ctrl, Shift], Z, Redo);
        bind!([Ctrl], S, Save);
        bind!([Ctrl], A, SelectAll);
        bind!([Ctrl], Comma, ToggleSettingsPage);
        bind!([Alt], Z, ToggleWordWrap);
        bind!([Ctrl], Z, Undo);

        keybinds
    }

    pub fn matches(&self, modifiers: Modifiers, key_code: KeyCode) -> bool {
        self.key_code == key_code
            && modifiers.logo() == self.modifiers.contains(&Modifier::Super)
            && modifiers.control() == self.modifiers.contains(&Modifier::Ctrl)
            && modifiers.alt() == self.modifiers.contains(&Modifier::Alt)
            && modifiers.shift() == self.modifiers.contains(&Modifier::Shift)
    }
}

impl fmt::Display for KeyBind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for modifier in self.modifiers.iter() {
            write!(f, "{:?} + ", modifier)?;
        }
        write!(f, "{:?}", self.key_code)
    }
}

#[derive(Clone, CosmicConfigEntry, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Config {
    pub app_theme: AppTheme,
    pub font_name: String,
    pub font_size: u16,
    pub syntax_theme_dark: String,
    pub syntax_theme_light: String,
    pub tab_width: u16,
    pub vim_bindings: bool,
    pub word_wrap: bool,
    pub keybinds: HashMap<KeyBind, Action>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            app_theme: AppTheme::System,
            font_name: "Fira Mono".to_string(),
            font_size: 14,
            syntax_theme_dark: "gruvbox-dark".to_string(),
            syntax_theme_light: "gruvbox-light".to_string(),
            tab_width: 4,
            vim_bindings: false,
            word_wrap: false,
            keybinds: KeyBind::load(),
        }
    }
}

impl Config {
    // Calculate metrics from font size
    pub fn metrics(&self) -> Metrics {
        let font_size = self.font_size.max(1) as f32;
        let line_height = (font_size * 1.4).ceil();
        Metrics::new(font_size, line_height)
    }

    // Get current syntax theme based on dark mode
    pub fn syntax_theme(&self) -> &str {
        let dark = self.app_theme.theme().theme_type.is_dark();
        if dark {
            &self.syntax_theme_dark
        } else {
            &self.syntax_theme_light
        }
    }
}
