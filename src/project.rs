use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub struct Project {
    path: PathBuf,
    name: String,
}

impl Project {
    pub fn new<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = fs::canonicalize(path)?;
        let name = path
            .file_name()
            .ok_or(io::Error::new(
                io::ErrorKind::Other,
                format!("Path {:?} has no file name", path),
            ))?
            .to_str()
            .ok_or(io::Error::new(
                io::ErrorKind::Other,
                format!("Path {:?} is not valid UTF-8", path),
            ))?
            .to_string();
        Ok(Self { path, name })
    }
}
