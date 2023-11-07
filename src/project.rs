// SPDX-License-Identifier: GPL-3.0-only

use std::{
    cmp::Ordering,
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectNode {
    Folder {
        name: String,
        path: PathBuf,
        open: bool,
        root: bool,
    },
    File {
        name: String,
        path: PathBuf,
    },
}

impl ProjectNode {
    pub fn new<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = fs::canonicalize(path)?;
        let name = path
            .file_name()
            .ok_or(io::Error::new(
                io::ErrorKind::Other,
                format!("path {:?} has no file name", path),
            ))?
            .to_str()
            .ok_or(io::Error::new(
                io::ErrorKind::Other,
                format!("path {:?} is not valid UTF-8", path),
            ))?
            .to_string();
        Ok(if path.is_dir() {
            Self::Folder {
                path,
                name,
                open: false,
                root: false,
            }
        } else {
            Self::File { path, name }
        })
    }

    pub fn icon_name(&self) -> &str {
        match self {
            //TODO: different icon for project root?
            Self::Folder { open, .. } => {
                if *open {
                    "go-down-symbolic"
                } else {
                    "go-next-symbolic"
                }
            }
            Self::File { .. } => "text-x-generic",
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Folder { name, .. } => name,
            Self::File { name, .. } => name,
        }
    }
}

impl Ord for ProjectNode {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (ProjectNode::Folder { .. }, ProjectNode::Folder { .. }) => {}
            (ProjectNode::Folder { .. }, ProjectNode::File { .. }) => return Ordering::Less,
            (ProjectNode::File { .. }, ProjectNode::Folder { .. }) => return Ordering::Greater,
            (ProjectNode::File { .. }, ProjectNode::File { .. }) => {}
        }

        parse_to_lexemes(self.name()).cmp(&parse_to_lexemes(other.name()))
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum Lexeme {
    String(String),
    Number(i32),
    Special(String),
}

impl Ord for Lexeme {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Lexeme::String(str1), Lexeme::String(str2)) => {
                for (chr1, chr2) in str1.chars().zip(str2.chars()) {
                    let cmp = chr1.to_ascii_lowercase().cmp(&chr2.to_ascii_lowercase());
                    if cmp == Ordering::Equal {
                        if chr1.is_ascii_lowercase() != chr2.is_ascii_lowercase() {
                            if chr1.is_ascii_uppercase() {
                                return Ordering::Less;
                            } else {
                                return Ordering::Greater;
                            }
                        } else {
                            continue;
                        }
                    }
                    return cmp;
                }
                // should be unreachable
                return Ordering::Equal;
            }
            (Lexeme::Number(num1), Lexeme::Number(num2)) => num1.cmp(num2),
            (Lexeme::String(_), Lexeme::Number(_)) => std::cmp::Ordering::Greater,
            (Lexeme::Number(_), Lexeme::String(_)) => std::cmp::Ordering::Less,
            (Lexeme::String(_), Lexeme::Special(_)) => Ordering::Greater,
            (Lexeme::Number(_), Lexeme::Special(_)) => Ordering::Greater,
            (Lexeme::Special(_), Lexeme::String(_)) => Ordering::Less,
            (Lexeme::Special(_), Lexeme::Number(_)) => Ordering::Less,
            (Lexeme::Special(spe1), Lexeme::Special(spe2)) => spe1.cmp(spe2),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct Lexemes(Vec<Lexeme>);

impl Ord for Lexemes {
    fn cmp(&self, other: &Self) -> Ordering {
        for (lexeme1, lexeme2) in self.0.iter().zip(other.0.iter()) {
            let cmp = lexeme1.cmp(lexeme2);
            if cmp != Ordering::Equal {
                return cmp;
            }
        }

        self.0.len().cmp(&other.0.len())
    }
}

fn parse_to_lexemes(name: &str) -> Lexemes {
    enum State {
        String,
        Number,
        Special,
    }

    impl From<&char> for State {
        fn from(value: &char) -> Self {
            if value.is_ascii_digit() {
                State::Number
            } else if value.is_alphabetic() {
                State::String
            } else {
                State::Special
            }
        }
    }

    let mut lexemes = Vec::new();
    let mut chars = name.chars();

    let first_char = chars.next().unwrap();

    let mut current_state: State = State::from(&first_char);
    let mut current_letters = first_char.to_string();

    for c in chars {
        match current_state {
            State::String => {
                let state = State::from(&c);
                match state {
                    State::String => {}
                    State::Number | State::Special => {
                        lexemes.push(Lexeme::String(current_letters.clone()));
                        current_letters.clear();
                        current_state = state;
                    }
                }
            }
            State::Number => {
                let state = State::from(&c);
                match state {
                    State::Number => {}
                    State::Special | State::String => {
                        lexemes.push(Lexeme::Number(current_letters.parse::<i32>().unwrap()));
                        current_letters.clear();
                        current_state = state;
                    }
                }
            }
            State::Special => {
                let state = State::from(&c);
                match state {
                    State::Special => {
                        lexemes.push(Lexeme::Special(current_letters.clone()));
                        current_letters.clear();
                    }
                    State::Number | State::String => {
                        lexemes.push(Lexeme::Special(current_letters.clone()));
                        current_letters.clear();
                        current_state = state;
                    }
                }
            }
        }
        current_letters.push(c);
    }
    match current_state {
        State::String => {
            lexemes.push(Lexeme::String(current_letters));
        }
        State::Number => {
            lexemes.push(Lexeme::Number(current_letters.parse::<i32>().unwrap()));
        }
        State::Special => {
            lexemes.push(Lexeme::Special(current_letters));
        }
    }

    Lexemes(lexemes)
}

impl PartialOrd for ProjectNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialOrd for Lexemes {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialOrd for Lexeme {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
