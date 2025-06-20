// SPDX-License-Identifier: GPL-3.0-only

use cosmic::{
    iced::{advanced::graphics::text::font_system, Point},
    widget::icon,
};
use cosmic_files::mime_icon::{mime_for_path, mime_icon, FALLBACK_MIME_ICON};
use cosmic_text::{Attrs, Buffer, Cursor, Edit, Selection, Shaping, SyntaxEditor, ViEditor, Wrap};
use notify::Watcher;
use regex::Regex;
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};

use crate::{fl, git::GitDiff, Config, SYNTAX_SYSTEM};

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
}

impl EditorTab {
    pub fn new(config: &Config) -> Self {
        //TODO: do not repeat, used in App::init
        let attrs = Attrs::new().family(cosmic_text::Family::Monospace);
        let zoom_adj = Default::default();
        let mut buffer = Buffer::new_empty(config.metrics(zoom_adj));
        buffer.set_text(
            font_system().write().unwrap().raw(),
            "",
            &attrs,
            Shaping::Advanced,
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
        match editor.load_text(&path, self.attrs.clone()) {
            Ok(()) => {
                log::info!("opened {:?}", path);
                self.path_opt = match fs::canonicalize(&path) {
                    Ok(ok) => Some(ok),
                    Err(err) => {
                        log::error!("failed to canonicalize {:?}: {}", path, err);
                        Some(path)
                    }
                };
            }
            Err(err) => {
                log::error!("failed to open {:?}: {}", path, err);
                self.path_opt = None;
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

            match editor.load_text(path, self.attrs.clone()) {
                Ok(()) => {
                    log::info!("reloaded {:?}", path);

                    // Clear changed state
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

    pub fn save(&mut self) {
        if let Some(path) = &self.path_opt {
            let mut editor = self.editor.lock().unwrap();
            let mut text = String::new();

            editor.with_buffer(|buffer| {
                for line in buffer.lines.iter() {
                    text.push_str(line.text());
                    text.push_str(line.ending().as_str());
                }
            });

            match fs::write(path, &text) {
                Ok(()) => {
                    editor.save_point();
                    log::info!("saved {:?}", path);
                }
                Err(err) => {
                    if err.kind() == std::io::ErrorKind::PermissionDenied {
                        log::warn!("Permission denied. Attempting to save with pkexec.");

                        if let Ok(mut output) = Command::new("pkexec")
                            .arg("tee")
                            .arg(path)
                            .stdin(Stdio::piped())
                            .stdout(Stdio::null()) // Redirect stdout to /dev/null
                            .stderr(Stdio::inherit()) // Retain stderr for error visibility
                            .spawn()
                        {
                            if let Some(mut stdin) = output.stdin.take() {
                                if let Err(e) = stdin.write_all(text.as_bytes()) {
                                    log::error!("Failed to write to stdin: {}", e);
                                }
                            } else {
                                log::error!("Failed to access stdin of pkexec process.");
                            }

                            // Ensure the child process is reaped
                            match output.wait() {
                                Ok(status) => {
                                    if status.success() {
                                        // Mark the editor's state as saved if the process succeeds
                                        editor.save_point();
                                        log::info!("File saved successfully with pkexec.");
                                    } else {
                                        log::error!(
                                            "pkexec process exited with a non-zero status: {:?}",
                                            status
                                        );
                                    }
                                }
                                Err(e) => {
                                    log::error!("Failed to wait on pkexec process: {}", e);
                                }
                            }
                        } else {
                            log::error!(
                                "Failed to spawn pkexec process. Check permissions or path."
                            );
                        }
                    }
                }
            }
        } else {
            log::warn!("tab has no path yet");
        }
    }

    pub fn watch(&self, watcher_opt: &mut Option<notify::RecommendedWatcher>) {
        if let Some(path) = &self.path_opt {
            if let Some(watcher) = watcher_opt {
                match watcher.watch(path, notify::RecursiveMode::NonRecursive) {
                    Ok(()) => {
                        log::info!("watching {:?} for changes", path);
                    }
                    Err(err) => {
                        log::warn!("failed to watch {:?} for changes: {:?}", path, err);
                    }
                }
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
