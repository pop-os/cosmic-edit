use cosmic::iced::keyboard::{KeyCode, Modifiers};
use std::{collections::HashMap, fmt};

use crate::Message;

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
            ($modifiers:ident, $key_code:ident, $message:ident) => {{
                keybinds.insert(
                    KeyBind {
                        modifiers: Modifiers::$modifiers,
                        key_code: KeyCode::$key_code,
                    },
                    Message::$message,
                );
            }};
        }

        bind!(CTRL, X, Cut);
        bind!(CTRL, C, Copy);
        bind!(CTRL, V, Paste);
        bind!(CTRL, N, New);
        bind!(CTRL, O, OpenFileDialog);
        bind!(CTRL, S, Save);

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

#[derive(Clone, Debug)]
pub struct Config {
    pub wrap: bool,
    pub keybinds: HashMap<KeyBind, Message>,
}

impl Config {
    //TODO: load from cosmic-config
    pub fn load() -> Self {
        Self {
            wrap: false,
            keybinds: KeyBind::load(),
        }
    }
}
