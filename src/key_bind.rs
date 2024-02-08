use cosmic::{
    iced::keyboard::{Key, Modifiers},
    iced_core::keyboard::key::Named,
};
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
    pub key: Key,
}

impl KeyBind {
    pub fn matches(&self, modifiers: Modifiers, key: Key) -> bool {
        self.key == key
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
        match &self.key {
            Key::Character(c) => write!(f, "{}", c.to_uppercase()),
            Key::Named(named) => write!(f, "{:?}", named),
            other => write!(f, "{:?}", other),
        }
    }
}

//TODO: load from config
pub fn key_binds() -> HashMap<KeyBind, Action> {
    let mut key_binds = HashMap::new();

    macro_rules! bind {
        ([$($modifier:ident),+ $(,)?], $key:expr, $action:ident) => {{
            key_binds.insert(
                KeyBind {
                    modifiers: vec![$(Modifier::$modifier),+],
                    key: $key,
                },
                Action::$action,
            );
        }};
    }

    bind!([Ctrl], Key::Character("w".into()), CloseFile);
    bind!([Ctrl], Key::Character("x".into()), Cut);
    bind!([Ctrl], Key::Character("c".into()), Copy);
    bind!([Ctrl], Key::Character("f".into()), Find);
    bind!([Ctrl], Key::Character("h".into()), FindAndReplace);
    bind!([Ctrl], Key::Character("v".into()), Paste);
    bind!([Ctrl], Key::Character("t".into()), NewFile);
    bind!([Ctrl], Key::Character("n".into()), NewWindow);
    bind!([Ctrl], Key::Character("o".into()), OpenFileDialog);
    bind!([Ctrl, Shift], Key::Character("O".into()), OpenProjectDialog);
    bind!([Ctrl], Key::Character("q".into()), Quit);
    bind!([Ctrl, Shift], Key::Character("z".into()), Redo);
    bind!([Ctrl], Key::Character("s".into()), Save);
    bind!([Ctrl], Key::Character("a".into()), SelectAll);
    bind!([Ctrl], Key::Character("1".into()), TabActivate0);
    bind!([Ctrl], Key::Character("2".into()), TabActivate1);
    bind!([Ctrl], Key::Character("3".into()), TabActivate2);
    bind!([Ctrl], Key::Character("4".into()), TabActivate3);
    bind!([Ctrl], Key::Character("5".into()), TabActivate4);
    bind!([Ctrl], Key::Character("6".into()), TabActivate5);
    bind!([Ctrl], Key::Character("7".into()), TabActivate6);
    bind!([Ctrl], Key::Character("8".into()), TabActivate7);
    bind!([Ctrl], Key::Character("9".into()), TabActivate8);
    bind!([Ctrl], Key::Named(Named::Tab), TabNext);
    bind!([Ctrl, Shift], Key::Named(Named::Tab), TabPrev);
    bind!(
        [Ctrl, Shift],
        Key::Character("G".into()),
        ToggleGitManagement
    );
    bind!(
        [Ctrl, Shift],
        Key::Character("F".into()),
        ToggleProjectSearch
    );
    bind!([Ctrl], Key::Character(",".into()), ToggleSettingsPage);
    bind!([Alt], Key::Character("z".into()), ToggleWordWrap);
    bind!([Ctrl], Key::Character("z".into()), Undo);

    key_binds
}
