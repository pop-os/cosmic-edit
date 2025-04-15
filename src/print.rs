use std::process::Command;
use std::io::{self, Write};
use tempfile::NamedTempFile;

pub fn print_text(text: &str) -> io::Result<()> {
    let mut file = NamedTempFile::new()?;
    write!(file, "{}", text)?;
    file.flush()?;

    let path = file.path().to_str().unwrap();

    let status = Command::new("lp")
        .arg(path)
        .status()?;

    if !status.success() {
        eprintln!("lp failed with status: {:?}", status);
    }

    Ok(())
}


