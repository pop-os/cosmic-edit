use cosmic::iced::keyboard::{KeyCode, Modifiers};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fmt};

use crate::Action;

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

//TODO: load from config
pub fn key_binds() -> HashMap<KeyBind, Action> {
    let mut key_binds = HashMap::new();

    macro_rules! bind {
        ([$($modifier:ident),+ $(,)?], $key_code:ident, $action:ident) => {{
            key_binds.insert(
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
    bind!([Ctrl], F, Find);
    bind!([Ctrl], H, FindAndReplace);
    bind!([Ctrl], V, Paste);
    bind!([Ctrl], T, NewFile);
    bind!([Ctrl], N, NewWindow);
    bind!([Ctrl], O, OpenFileDialog);
    bind!([Ctrl, Shift], O, OpenProjectDialog);
    bind!([Ctrl], Q, Quit);
    bind!([Ctrl, Shift], Z, Redo);
    bind!([Ctrl], S, Save);
    bind!([Ctrl], A, SelectAll);
    bind!([Ctrl, Shift], G, ToggleGitManagement);
    bind!([Ctrl, Shift], F, ToggleProjectSearch);
    bind!([Ctrl], Comma, ToggleSettingsPage);
    bind!([Alt], Z, ToggleWordWrap);
    bind!([Ctrl], Z, Undo);

    key_binds
}
