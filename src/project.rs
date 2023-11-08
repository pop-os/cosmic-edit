// SPDX-License-Identifier: GPL-3.0-only

use std::{
    cmp::Ordering,
    fs, io,
    path::{Path, PathBuf},
};

use crate::name_comparaison::compare_names;

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

        compare_names(self.name(), other.name())
    }
}

impl PartialOrd for ProjectNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
