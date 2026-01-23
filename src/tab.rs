// SPDX-License-Identifier: GPL-3.0-only

use cosmic::{
    iced::{Point, advanced::graphics::text::font_system},
    widget::icon,
};
use cosmic_files::mime_icon::{FALLBACK_MIME_ICON, mime_for_path, mime_icon};
use cosmic_text::{Attrs, Buffer, Cursor, Edit, Selection, Shaping, SyntaxEditor, ViEditor, Wrap};
use regex::Regex;
use std::{
    fmt, fs,
    io::{self, Write},
    path::{self, Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
};

use crate::{Config, SYNTAX_SYSTEM, backup, fl, git::GitDiff};

/// File size threshold (in bytes) above which we set a minimal buffer height
/// before loading to prevent shaping all lines at once.
/// This prevents the 313x memory multiplier crash on large files.
/// 1MB = conservative threshold that prevents memory explosion.
const LARGE_FILE_THRESHOLD: u64 = 1024 * 1024;

/// Errors that can occur when saving a tab to disk.
#[derive(Debug)]
pub enum SaveError {
    /// Tab has no file path associated with it.
    NoPath,
    /// I/O error during save operation.
    Io(io::Error),
    /// pkexec process failed to spawn or execute.
    PkexecFailed {
        /// Exit status if the process ran but failed.
        status: Option<ExitStatus>,
        /// Error message describing the failure.
        message: String,
    },
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPath => write!(f, "tab has no file path"),
            Self::Io(e) => write!(f, "I/O error: {}", e),
            Self::PkexecFailed { status, message } => {
                if let Some(s) = status {
                    write!(f, "pkexec failed ({}): {}", s, message)
                } else {
                    write!(f, "pkexec failed: {}", message)
                }
            }
        }
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for SaveError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

/// Cursor position in a document.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CursorPosition {
    /// Line number (0-indexed).
    pub line: usize,
    /// Character index within the line.
    pub index: usize,
}

impl CursorPosition {
    /// Create a new cursor position.
    pub fn new(line: usize, index: usize) -> Self {
        Self { line, index }
    }
}

fn editor_text(editor: &ViEditor<'static, 'static>) -> String {
    editor.with_buffer(|buffer| {
        let mut text = String::new();
        for line in buffer.lines.iter() {
            text.push_str(line.text());
            text.push_str(line.ending().as_str());
        }
        text
    })
}

/// Resolve a path to its canonical form, falling back to absolute path if canonicalization fails.
fn resolve_path(path_in: &PathBuf) -> PathBuf {
    fs::canonicalize(path_in).unwrap_or_else(|err| {
        path::absolute(path_in).unwrap_or_else(|_| {
            log::error!("failed to canonicalize {:?}: {}", path_in, err);
            path_in.clone()
        })
    })
}

pub enum Tab {
    Editor(EditorTab),
    GitDiff(GitDiffTab),
}

impl Tab {
    pub fn title(&self) -> String {
        match self {
            Self::Editor(tab) => tab.title(),
            Self::GitDiff(tab) => tab.title.clone(),
        }
    }
}

pub struct GitDiffTab {
    pub title: String,
    pub diff: GitDiff,
}

pub struct EditorTab {
    pub path_opt: Option<PathBuf>,
    attrs: Attrs<'static>,
    pub editor: Mutex<ViEditor<'static, 'static>>,
    pub context_menu: Option<Point>,
    pub zoom_adj: i8,
    /// Backup ID for hot exit cache.
    ///
    /// # Lifecycle
    /// - Set when a backup is written for this tab
    /// - Cleared when the tab is saved to disk (backup no longer needed)
    /// - Persists across saves to the same backup file (reused for updates)
    pub backup_id: Option<String>,

    /// Hash of content at last backup write.
    ///
    /// # Lifecycle
    /// - Set when a backup is written
    /// - Used to skip unnecessary backup writes when content hasn't changed
    /// - Cleared when content reverts to saved state
    pub backup_content_hash: Option<u64>,

    /// Hash of the file content as saved on disk.
    ///
    /// # Lifecycle
    /// - Set when file is opened (hash of file content)
    /// - Updated after successful save() or save_with_pkexec()
    /// - Used by check_and_reset_if_unchanged() to detect when edits
    ///   have been undone back to the saved state
    pub saved_content_hash: Option<u64>,
}

impl EditorTab {
    pub fn new(config: &Config) -> Self {
        let attrs = crate::monospace_attrs();
        let zoom_adj = Default::default();
        let mut buffer = Buffer::new_empty(config.metrics(zoom_adj));
        buffer.set_text(
            font_system().write().unwrap().raw(),
            "",
            &attrs,
            Shaping::Advanced,
            None,
        );

        let editor = SyntaxEditor::new(
            Arc::new(buffer),
            SYNTAX_SYSTEM.get().unwrap(),
            config.syntax_theme(),
        )
        .unwrap();

        let mut tab = Self {
            path_opt: None,
            attrs,
            editor: Mutex::new(ViEditor::new(editor)),
            context_menu: None,
            zoom_adj,
            backup_id: None,
            backup_content_hash: None,
            saved_content_hash: None,
        };

        // Update any other config settings
        tab.set_config(config);

        tab
    }

    pub fn set_config(&mut self, config: &Config) {
        let mut editor = self.editor.lock().unwrap();
        let mut font_system = font_system().write().unwrap();
        let mut editor = editor.borrow_with(font_system.raw());
        editor.set_auto_indent(config.auto_indent);
        editor.set_passthrough(!config.vim_bindings);
        editor.set_tab_width(config.tab_width);
        editor.with_buffer_mut(|buffer| {
            buffer.set_wrap(if config.word_wrap {
                Wrap::WordOrGlyph
            } else {
                Wrap::None
            })
        });
        //TODO: dynamically discover light/dark changes
        editor.update_theme(config.syntax_theme());
    }

    pub fn open(&mut self, path: PathBuf) {
        let mut editor = self.editor.lock().unwrap();
        let mut font_system = font_system().write().unwrap();
        let mut editor = editor.borrow_with(font_system.raw());
        let absolute = match fs::canonicalize(&path) {
            Ok(ok) => ok,
            Err(err) => match path::absolute(&path) {
                Ok(ok) => ok,
                Err(_) => {
                    log::error!("failed to canonicalize {:?}: {}", path, err);
                    path
                }
            },
        };

        // Check file size and set minimal buffer height for large files
        // This prevents cosmic-text from shaping ALL lines at once
        // (which causes 200+ bytes per character memory usage)
        let is_large_file = fs::metadata(&absolute)
            .map(|m| m.len() > LARGE_FILE_THRESHOLD)
            .unwrap_or(false);

        if is_large_file {
            log::info!(
                "Large file detected (>{}KB), setting minimal buffer height before load",
                LARGE_FILE_THRESHOLD / 1024
            );
            // Set a small buffer height to limit initial shaping to ~5 lines
            // The real height will be set during rendering
            editor.with_buffer_mut(|buffer| {
                buffer.set_size(None, Some(100.0));
            });
        }

        match editor.load_text(&absolute, self.attrs.clone()) {
            Ok(()) => {
                log::info!("opened {:?}", absolute);
                self.path_opt = Some(absolute);
                // Store hash of the saved content for change detection
                self.saved_content_hash = Some(backup::compute_content_hash(&editor_text(&editor)));
            }
            Err(err) => {
                if err.kind() == io::ErrorKind::NotFound {
                    log::warn!("opened non-existant file {:?}", absolute);
                    self.path_opt = Some(absolute);
                    editor.set_changed(true);
                    // New file - hash of empty content
                    self.saved_content_hash = Some(backup::compute_content_hash(""));
                } else {
                    log::error!("failed to open {:?}: {}", absolute, err);
                    self.path_opt = None;
                }
            }
        }
    }

    pub fn reload(&mut self) {
        let mut editor = self.editor.lock().unwrap();
        let mut font_system = font_system().write().unwrap();
        let mut editor = editor.borrow_with(font_system.raw());
        if let Some(path) = &self.path_opt {
            // Save scroll
            let scroll = editor.with_buffer(|buffer| buffer.scroll());
            //TODO: save/restore more?

            match std::fs::read_to_string(path) {
                Ok(file_content) => {
                    log::info!("reloaded {:?}", path);

                    //TODO: compare using line iterator to prevent allocations
                    if file_content == editor_text(&editor) {
                        log::info!("text not changed");
                        return;
                    }

                    // Store the entire operation as a single change for undo
                    editor.start_change();

                    // Grab everything in the buffer
                    let cursor_start: Cursor = cosmic_text::Cursor::new(0, 0);
                    let cursor_end = editor.with_buffer(|buffer| {
                        let last_line = buffer.lines.len().saturating_sub(1);
                        cosmic_text::Cursor::new(
                            last_line,
                            buffer
                                .lines
                                .get(last_line)
                                .map(|line| line.text().len())
                                .unwrap_or(0),
                        )
                    });

                    // Replace everything in the buffer with the content from disk
                    editor.delete_range(cursor_start, cursor_end);
                    editor.insert_at(cursor_start, &file_content, None);

                    // Adjust cursor to closest position
                    let mut cursor = editor.cursor();
                    editor.with_buffer(|buffer| {
                        cursor.line = cursor.line.min(buffer.lines.len().saturating_sub(1));
                        cursor.index = if let Some(line) = buffer.lines.get(cursor.line) {
                            let mut closest = line.text().len();
                            for (i, _) in line.text().char_indices().rev() {
                                if i >= cursor.index {
                                    closest = i;
                                } else {
                                    // i < cursor.index
                                    if cursor.index - i < closest - cursor.index {
                                        closest = i;
                                    }
                                    break;
                                }
                            }
                            closest
                        } else {
                            0
                        }
                    });
                    editor.set_cursor(cursor);

                    editor.finish_change();
                    editor.set_changed(false);
                }
                Err(err) => {
                    log::error!("failed to reload {:?}: {}", path, err);
                }
            }

            // Restore scroll
            editor.with_buffer_mut(|buffer| buffer.set_scroll(scroll));
        } else {
            log::warn!("tried to reload with no path");
        }
    }

    /// Set the file path for this tab, resolving it to a canonical form.
    pub fn set_path(&mut self, path: PathBuf) {
        self.path_opt = Some(resolve_path(&path));
    }

    /// Save the tab content to a new path (Save As).
    /// Sets the path and then saves.
    pub fn save_as(&mut self, path: PathBuf) -> Result<(), SaveError> {
        self.set_path(path);
        self.save()
    }

    /// Save the tab content to disk.
    ///
    /// Returns `Ok(())` on success, or a `SaveError` describing the failure.
    /// If the initial write fails with permission denied, attempts to save
    /// using `pkexec` for elevated permissions.
    pub fn save(&mut self) -> Result<(), SaveError> {
        let path = match &self.path_opt {
            Some(p) => p.clone(),
            None => {
                log::warn!("tab has no path yet");
                return Err(SaveError::NoPath);
            }
        };

        let text = {
            let editor = self.editor.lock().unwrap();
            editor_text(&editor)
        };
        let content_hash = backup::compute_content_hash(&text);

        match fs::write(&path, &text) {
            Ok(()) => {
                let mut editor = self.editor.lock().unwrap();
                editor.save_point();
                self.saved_content_hash = Some(content_hash);
                log::info!("saved {:?}", path);
                Ok(())
            }
            Err(err) => {
                if err.kind() != io::ErrorKind::PermissionDenied {
                    log::error!("failed to save {:?}: {}", path, err);
                    return Err(SaveError::Io(err));
                }

                // Try to save with elevated permissions via pkexec
                log::warn!("Permission denied. Attempting to save with pkexec.");
                self.save_with_pkexec(&path, &text, content_hash)
            }
        }
    }

    /// Attempt to save using pkexec for elevated permissions.
    fn save_with_pkexec(
        &mut self,
        path: &Path,
        text: &str,
        content_hash: u64,
    ) -> Result<(), SaveError> {
        let mut child = Command::new("pkexec")
            .arg("tee")
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| {
                log::error!("Failed to spawn pkexec process: {}", e);
                SaveError::PkexecFailed {
                    status: None,
                    message: format!("failed to spawn: {}", e),
                }
            })?;

        // Write content to stdin
        if let Some(mut stdin) = child.stdin.take() {
            if let Err(e) = stdin.write_all(text.as_bytes()) {
                log::error!("Failed to write to pkexec stdin: {}", e);
                let _ = child.wait();
                return Err(SaveError::PkexecFailed {
                    status: None,
                    message: format!("failed to write to stdin: {}", e),
                });
            }
        } else {
            log::error!("Failed to access stdin of pkexec process.");
            let _ = child.wait();
            return Err(SaveError::PkexecFailed {
                status: None,
                message: "stdin not available".to_string(),
            });
        }

        // Wait for process and check result
        match child.wait() {
            Ok(status) if status.success() => {
                let mut editor = self.editor.lock().unwrap();
                editor.save_point();
                self.saved_content_hash = Some(content_hash);
                log::info!("File saved successfully with pkexec.");
                Ok(())
            }
            Ok(status) => {
                log::error!("pkexec process exited with status: {:?}", status);
                Err(SaveError::PkexecFailed {
                    status: Some(status),
                    message: "process exited with non-zero status".to_string(),
                })
            }
            Err(e) => {
                log::error!("Failed to wait on pkexec process: {}", e);
                Err(SaveError::PkexecFailed {
                    status: None,
                    message: format!("failed to wait on process: {}", e),
                })
            }
        }
    }

    pub fn changed(&self) -> bool {
        let editor = self.editor.lock().unwrap();
        editor.changed()
    }

    pub fn icon(&self, size: u16) -> icon::Icon {
        match &self.path_opt {
            Some(path) => icon::icon(mime_icon(mime_for_path(path, None, false), size)).size(size),
            None => icon::from_name(FALLBACK_MIME_ICON).size(size).icon(),
        }
    }

    pub fn title(&self) -> String {
        //TODO: show full title when there is a conflict
        if let Some(path) = &self.path_opt {
            match path.file_name() {
                Some(file_name_os) => match file_name_os.to_str() {
                    Some(file_name) => match file_name {
                        "mod.rs" => title_with_parent(path, file_name),
                        _ => file_name.to_string(),
                    },
                    None => format!("{}", path.display()),
                },
                None => format!("{}", path.display()),
            }
        } else {
            fl!("new-document")
        }
    }

    pub fn replace(&self, regex: &Regex, replace: &str, wrap_around: bool) -> bool {
        let mut editor = self.editor.lock().unwrap();
        let mut cursor = editor.cursor();
        let mut wrapped = false; // Keeps track of whether the search has wrapped around yet.
        let start_line = cursor.line;
        while cursor.line < editor.with_buffer(|buffer| buffer.lines.len()) {
            if let Some((index, len)) = editor.with_buffer(|buffer| {
                regex
                    .find_iter(buffer.lines[cursor.line].text())
                    .filter_map(|m| {
                        if cursor.line != start_line
                            || m.start() >= cursor.index
                            || m.start() < cursor.index && wrapped == true
                        {
                            Some((m.start(), m.len()))
                        } else {
                            None
                        }
                    })
                    .next()
            }) {
                cursor.index = index;
                let mut end = cursor;
                end.index = index + len;

                editor.start_change();
                // if index = 0 and len = 0, we are targeting and deleting an empty line
                // we'll move either cursor or end to delete the newline
                if index == 0 && len == 0 {
                    if cursor.line > 0 {
                        // move the cursor up one line
                        cursor.line -= 1;
                        cursor.index =
                            editor.with_buffer(|buffer| buffer.lines[cursor.line].text().len());
                    } else if cursor.line + 1 < editor.with_buffer(|buffer| buffer.lines.len()) {
                        // move the end down one line
                        end.line += 1;
                        end.index = 0;
                    }
                }
                editor.delete_range(cursor, end);
                cursor = editor.insert_at(cursor, replace, None);
                editor.set_cursor(cursor);
                // Need to disable selection to prevent the new cursor showing selection to old location
                editor.set_selection(Selection::None);
                editor.finish_change();
                return true;
            }

            cursor.line += 1;

            // If we haven't wrapped yet and we've reached the last line, reset cursor line to 0 and
            // set wrapped to true so we don't wrap again
            if wrap_around
                && !wrapped
                && cursor.line == editor.with_buffer(|buffer| buffer.lines.len())
            {
                cursor.line = 0;
                wrapped = true;
            }
        }
        false
    }

    pub fn zoom_adj(&self) -> i8 {
        self.zoom_adj
    }

    pub fn set_zoom_adj(&mut self, value: i8) {
        self.zoom_adj = value;
    }

    /// Get the text content of the editor
    pub fn text(&self) -> String {
        let editor = self.editor.lock().unwrap();
        editor_text(&editor)
    }

    /// Check if current content matches the saved file content.
    /// If it does, reset the changed indicator and return true.
    /// This handles the case where user makes changes then undoes them.
    pub fn check_and_reset_if_unchanged(&mut self) -> bool {
        let current_hash = backup::compute_content_hash(&self.text());
        if self.saved_content_hash == Some(current_hash) {
            // Content matches saved state - reset changed indicator
            let mut editor = self.editor.lock().unwrap();
            editor.set_changed(false);
            true
        } else {
            false
        }
    }

    /// Get the current cursor position.
    pub fn cursor(&self) -> CursorPosition {
        let editor = self.editor.lock().unwrap();
        let cursor = editor.cursor();
        CursorPosition::new(cursor.line, cursor.index)
    }

    /// Load content from a string (used for restoring cached documents)
    pub fn load_text(&mut self, content: &str) {
        let mut editor = self.editor.lock().unwrap();
        let mut font_system = font_system().write().unwrap();
        let mut editor = editor.borrow_with(font_system.raw());

        // Select all content and replace it with new content
        let cursor_start = Cursor::new(0, 0);
        let cursor_end = editor.with_buffer(|buffer| {
            let last_line = buffer.lines.len().saturating_sub(1);
            Cursor::new(
                last_line,
                buffer
                    .lines
                    .get(last_line)
                    .map(|line| line.text().len())
                    .unwrap_or(0),
            )
        });

        editor.start_change();
        editor.delete_range(cursor_start, cursor_end);
        editor.insert_at(cursor_start, content, None);
        editor.finish_change();

        // Mark as changed since this is unsaved cached content
        editor.set_changed(true);
    }

    /// Set the cursor position (clamped to valid range)
    pub fn set_cursor(&mut self, line: usize, index: usize) {
        let mut editor = self.editor.lock().unwrap();
        // Clamp cursor to valid range to avoid panics
        let cursor_opt = editor.with_buffer(|buffer| {
            // Handle empty buffer case
            if buffer.lines.is_empty() {
                return None;
            }
            let max_line = buffer.lines.len().saturating_sub(1);
            let safe_line = line.min(max_line);
            let line_len = buffer
                .lines
                .get(safe_line)
                .map(|l| l.text().len())
                .unwrap_or(0);
            Some((safe_line, index.min(line_len)))
        });
        if let Some((safe_line, safe_index)) = cursor_opt {
            editor.set_cursor(Cursor::new(safe_line, safe_index));
        }
    }

    // Code adapted from cosmic-text ViEditor search
    pub fn search(&self, regex: &Regex, forwards: bool, wrap_around: bool) -> bool {
        let mut editor = self.editor.lock().unwrap();
        let mut cursor = editor.cursor();
        let mut wrapped = false; // Keeps track of whether the search has wrapped around yet.
        let start_line = cursor.line;
        let current_selection = editor.selection();

        if forwards {
            while cursor.line < editor.with_buffer(|buffer| buffer.lines.len()) {
                if let Some((start, end)) = editor.with_buffer(|buffer| {
                    regex
                        .find_iter(buffer.lines[cursor.line].text())
                        .filter_map(|m| {
                            if cursor.line != start_line
                                || m.start() > cursor.index
                                || m.start() == cursor.index && current_selection == Selection::None
                                || m.start() < cursor.index && wrapped == true
                            {
                                Some((m.start(), m.end()))
                            } else {
                                None
                            }
                        })
                        .next()
                }) {
                    cursor.index = start;
                    editor.set_cursor(cursor);

                    // Highlight searched text
                    let selection = Selection::Normal(Cursor::new(cursor.line, end));
                    editor.set_selection(selection);

                    return true;
                }

                cursor.line += 1;

                // If we haven't wrapped yet and we've reached the last line, reset cursor line to 0 and
                // set wrapped to true so we don't wrap again
                if wrap_around
                    && !wrapped
                    && cursor.line == editor.with_buffer(|buffer| buffer.lines.len())
                {
                    cursor.line = 0;
                    wrapped = true;
                }
            }
        } else {
            cursor.line += 1;
            while cursor.line > 0 {
                cursor.line -= 1;

                if let Some((start, end)) = editor.with_buffer(|buffer| {
                    regex
                        .find_iter(buffer.lines[cursor.line].text())
                        .filter_map(|m| {
                            if cursor.line != start_line
                                || m.start() < cursor.index
                                || m.start() == cursor.index && current_selection == Selection::None
                                || m.start() > cursor.index && wrapped == true
                            {
                                Some((m.start(), m.end()))
                            } else {
                                None
                            }
                        })
                        .last()
                }) {
                    cursor.index = start;
                    editor.set_cursor(cursor);

                    // Highlight searched text
                    let selection = Selection::Normal(Cursor::new(cursor.line, end));
                    editor.set_selection(selection);

                    return true;
                }

                // If we haven't wrapped yet and we've reached the first line, reset cursor line to the
                // last line and set wrapped to true so we don't wrap again
                if wrap_around && !wrapped && cursor.line == 0 {
                    cursor.line = editor.with_buffer(|buffer| buffer.lines.len());
                    wrapped = true;
                }
            }
        }
        false
    }
}

/// Includes parent name in tab title
///
/// Useful for distinguishing between Rust modules named `mod.rs`
fn title_with_parent(path: &std::path::Path, file_name: &str) -> String {
    let parent_name = path
        .parent()
        .and_then(|path| path.file_name())
        .and_then(|os_str| os_str.to_str());

    match parent_name {
        Some(parent) => [parent, "/", file_name].concat(),
        None => file_name.to_string(),
    }
}
