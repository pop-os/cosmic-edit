// SPDX-License-Identifier: GPL-3.0-only

use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub enum ProjectNode {
    Root {
        name: String,
        path: PathBuf,
        open: bool,
    },
    Folder {
        name: String,
        path: PathBuf,
        open: bool,
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
            }
        } else {
            Self::File { path, name }
        })
    }

    pub fn icon_name(&self) -> &str {
        match self {
            //TODO: different icon for project?
            ProjectNode::Root { open, .. } => {
                if *open {
                    "go-down-symbolic"
                } else {
                    "go-next-symbolic"
                }
            }
            ProjectNode::Folder { open, .. } => {
                if *open {
                    "go-down-symbolic"
                } else {
                    "go-next-symbolic"
                }
            }
            ProjectNode::File { .. } => "text-x-generic",
        }
    }

    pub fn name(&self) -> &str {
        match self {
            ProjectNode::Root { name, .. } => name,
            ProjectNode::Folder { name, .. } => name,
            ProjectNode::File { name, .. } => name,
        }
    }
}
