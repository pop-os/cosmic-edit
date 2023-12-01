//TODO: try to use gitoxide

use std::{
    fs, io,
    path::{Path, PathBuf},
};
use tokio::process::Command;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitDiff {
    pub path: PathBuf,
    pub staged: bool,
    pub hunks: Vec<GitDiffHunk>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitDiffHunk {
    pub old_range: patch::Range,
    pub new_range: patch::Range,
    pub lines: Vec<GitDiffLine>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitDiffLine {
    Context {
        old_line: u64,
        new_line: u64,
        text: String,
    },
    Added {
        new_line: u64,
        text: String,
    },
    Deleted {
        old_line: u64,
        text: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitStatus {
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
    pub staged: GitStatusKind,
    pub unstaged: GitStatusKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitStatusKind {
    Unmodified,
    Modified,
    FileTypeChanged,
    Added,
    Deleted,
    Renamed,
    Copied,
    Updated,
    Untracked,
    SubmoduleModified,
}

impl TryFrom<char> for GitStatusKind {
    type Error = char;

    fn try_from(c: char) -> Result<Self, Self::Error> {
        // https://git-scm.com/docs/git-status#_short_format
        match c {
            ' ' => Ok(Self::Unmodified),
            'M' => Ok(Self::Modified),
            'T' => Ok(Self::FileTypeChanged),
            'A' => Ok(Self::Added),
            'D' => Ok(Self::Deleted),
            'R' => Ok(Self::Renamed),
            'C' => Ok(Self::Copied),
            'U' => Ok(Self::Updated),
            '?' => Ok(Self::Untracked),
            'm' => Ok(Self::SubmoduleModified),
            _ => Err(c),
        }
    }
}

pub struct GitRepository {
    path: PathBuf,
}

impl GitRepository {
    pub fn new<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = path.as_ref();
        if path.join(".git").exists() {
            let path = fs::canonicalize(path)?;
            Ok(Self { path })
        } else {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{:?} is not a git repository", path),
            ))
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new("git");
        command.arg("-C").arg(&self.path);
        command
    }

    async fn command_stdout(mut command: Command) -> io::Result<String> {
        log::info!("{:?}", command);
        let output = command.output().await?;
        if output.status.success() {
            String::from_utf8(output.stdout).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("failed to parse git stdout: {}", err),
                )
            })
        } else {
            let mut msg = format!("git exited with {}", output.status);
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                msg.push_str("\nstdout> ");
                msg.push_str(line);
            }
            for line in String::from_utf8_lossy(&output.stderr).lines() {
                msg.push_str("\nstderr> ");
                msg.push_str(line);
            }
            Err(io::Error::new(io::ErrorKind::Other, msg))
        }
    }

    pub async fn diff<P: AsRef<Path>>(&self, path: P, staged: bool) -> io::Result<GitDiff> {
        let path = path.as_ref();
        let mut command = self.command();
        command.arg("diff");
        if staged {
            command.arg("--staged");
        }
        command.arg("--").arg(path);
        let diff = Self::command_stdout(command).await?;
        let patch = patch::Patch::from_single(&diff).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to parse diff: {}", err),
            )
        })?;

        let mut hunks = Vec::with_capacity(patch.hunks.len());
        for hunk in patch.hunks.iter() {
            //TODO: validate range counts
            let mut old_line = hunk.old_range.start;
            let mut new_line = hunk.new_range.start;

            let mut lines = Vec::with_capacity(hunk.lines.len());
            for line in hunk.lines.iter() {
                match line {
                    patch::Line::Context(text) => {
                        lines.push(GitDiffLine::Context {
                            old_line,
                            new_line,
                            text: text.to_string(),
                        });
                        old_line += 1;
                        new_line += 1;
                    }
                    patch::Line::Add(text) => {
                        lines.push(GitDiffLine::Added {
                            new_line,
                            text: text.to_string(),
                        });
                        new_line += 1;
                    }
                    patch::Line::Remove(text) => {
                        lines.push(GitDiffLine::Deleted {
                            old_line,
                            text: text.to_string(),
                        });
                        old_line += 1;
                    }
                }
            }

            hunks.push(GitDiffHunk {
                old_range: hunk.old_range.clone(),
                new_range: hunk.new_range.clone(),
                lines,
            });
        }

        Ok(GitDiff {
            path: path.to_path_buf(),
            staged,
            hunks,
        })
    }

    pub async fn status(&self) -> io::Result<Vec<GitStatus>> {
        let mut command = self.command();
        command.arg("status").arg("-z");
        let stdout = Self::command_stdout(command).await?;

        let mut status = Vec::new();
        let mut lines = stdout.split('\0');
        while let Some(line) = lines.next() {
            macro_rules! invalid_line {
                () => {{
                    log::warn!("invalid git status line {:?}", line);
                    continue;
                }};
            }

            if line.is_empty() {
                // Ignore empty lines
                continue;
            }

            let mut chars = line.chars();

            // Get staged status
            let staged = match chars.next() {
                Some(some) => match GitStatusKind::try_from(some) {
                    Ok(ok) => ok,
                    Err(_) => invalid_line!(),
                },
                None => invalid_line!(),
            };

            // Get unstaged status
            let unstaged = match chars.next() {
                Some(some) => match GitStatusKind::try_from(some) {
                    Ok(ok) => ok,
                    Err(_) => invalid_line!(),
                },
                None => invalid_line!(),
            };

            // Skip space
            match chars.next() {
                Some(' ') => {}
                _ => invalid_line!(),
            }

            // The rest of the chars are in the path
            let relative_path: String = chars.collect();

            let old_path = if staged == GitStatusKind::Renamed || unstaged == GitStatusKind::Renamed
            {
                match lines.next() {
                    Some(old_relative_path) => Some(self.path.join(old_relative_path)),
                    None => invalid_line!(),
                }
            } else {
                None
            };

            status.push(GitStatus {
                path: self.path.join(relative_path),
                old_path,
                staged,
                unstaged,
            })
        }

        Ok(status)
    }
}
