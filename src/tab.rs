// SPDX-License-Identifier: GPL-3.0-only

use cosmic::{
    iced::{advanced::graphics::text::font_system, Point},
    widget,
};
use cosmic_files::mime_icon::{mime_for_path, mime_icon, FALLBACK_MIME_ICON};
use cosmic_text::{
    Attrs, AttrsList, Buffer, Cursor, Edit, Selection, Shaping, SyntaxEditor, ViEditor, Wrap,
};
use regex::Regex;
use std::{
    fs,
    io,
    path::{self, PathBuf},
    sync::{Arc, Mutex},
};

use crate::{config::Config, git::GitDiff, SYNTAX_SYSTEM};

/// One tab in the editor UI.
pub enum Tab {
    Editor(EditorTab),
    GitDiff(GitDiffTab),
}

impl Tab {
    /// Display title used by the segmented tab bar.
    pub fn title(&self) -> String {
        match self {
            Tab::Editor(tab) => tab.title(),
            Tab::GitDiff(tab) => tab.title.clone(),
        }
    }
}

/// An editor tab holding a cosmic_text ViEditor.
pub struct EditorTab {
    pub path_opt: Option<PathBuf>,
    attrs: Attrs<'static>,
    pub editor: Mutex<ViEditor<'static, 'static>>,
    pub context_menu: Option<Point>,
    zoom_adj: i8,
}

fn resolve_path(path_in: &PathBuf) -> PathBuf {
    // Prefer canonical paths when possible, but gracefully fall back for non-existent files.
    fs::canonicalize(path_in).unwrap_or_else(|err| {
        path::absolute(path_in).unwrap_or_else(|_| {
            log::error!("failed to canonicalize {:?}: {}", path_in, err);
            path_in.clone()
        })
    })
}

impl EditorTab {
    pub fn new(config: &Config) -> Self {
        let attrs = crate::monospace_attrs();

        let mut buffer = Buffer::new_empty(config.metrics(0));

        // In this build, these buffer methods require a FontSystem.
        {
            let mut fs_guard = font_system().write().expect("font system write");
            let fs = fs_guard.raw();

            buffer.set_wrap(
                fs,
                if config.word_wrap {
                    Wrap::WordOrGlyph
                } else {
                    Wrap::None
                },
            );
            buffer.set_tab_width(fs, config.tab_width);
            buffer.set_text(fs, "", &attrs, Shaping::Advanced, None);
        }

        let syntax_editor = SyntaxEditor::new(
            Arc::new(buffer),
            SYNTAX_SYSTEM.get().expect("SYNTAX_SYSTEM not initialized"),
            config.syntax_theme(),
        )
        .expect("SyntaxEditor::new failed");

        let mut tab = Self {
            path_opt: None,
            attrs,
            editor: Mutex::new(ViEditor::new(syntax_editor)),
            context_menu: None,
            zoom_adj: 0,
        };

        tab.set_config(config);
        tab
    }

    pub fn title(&self) -> String {
        self.path_opt
            .as_ref()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "Untitled".to_string())
    }

    pub fn icon(&self, size: u16) -> cosmic::widget::Icon {
        match self.path_opt.as_ref() {
            Some(path) => {
                let mime = mime_for_path(path, None, false);
                widget::icon(mime_icon(mime, size)).size(size)
            }
            None => crate::icon_cache_get(FALLBACK_MIME_ICON, size),
        }
    }

    pub fn zoom_adj(&self) -> i8 {
        self.zoom_adj
    }

    pub fn set_zoom_adj(&mut self, zoom_adj: i8) {
        self.zoom_adj = zoom_adj;
    }

    pub fn changed(&self) -> bool {
        self.editor.lock().expect("editor lock").changed()
    }

    pub fn set_config(&mut self, config: &Config) {
        let mut editor = self.editor.lock().expect("editor lock");

        editor.set_auto_indent(config.auto_indent);
        editor.set_passthrough(!config.vim_bindings);

        // In this build, borrow_with requires a FontSystem.
        let mut fs_guard = font_system().write().expect("font system write");
        let fs_raw = fs_guard.raw();
        let mut ed = editor.borrow_with(fs_raw);

        ed.update_theme(config.syntax_theme());

        ed.with_buffer_mut(|buffer| {
            buffer.set_metrics(config.metrics(self.zoom_adj));

            // In this build, these variants do NOT take a FontSystem.
            buffer.set_wrap(if config.word_wrap {
                Wrap::WordOrGlyph
            } else {
                Wrap::None
            });
            buffer.set_tab_width(config.tab_width);
        });
    }

    pub fn set_path(&mut self, path: PathBuf) {
        self.path_opt = Some(resolve_path(&path));
    }

    pub fn save_as(&mut self, path: PathBuf) {
        self.set_path(path);
        self.save();
    }

    pub fn open(&mut self, path: PathBuf) {
        let absolute = resolve_path(&path);

        let mut editor = self.editor.lock().expect("editor lock");
        let mut fs_guard = font_system().write().expect("font system write");
        let mut ed = editor.borrow_with(fs_guard.raw());

        match ed.load_text(&absolute, self.attrs.clone()) {
            Ok(()) => {
                self.path_opt = Some(absolute);
                ed.set_changed(false);
            }
            Err(err) => {
                if err.kind() == io::ErrorKind::NotFound {
                    // Treat as "new file": keep the chosen path and mark as changed.
                    self.path_opt = Some(absolute);
                    ed.with_buffer_mut(|buffer| {
                        buffer.set_text("", &self.attrs, Shaping::Advanced, None);
                    });
                    ed.set_changed(true);
                } else {
                    log::error!("failed to open {:?}: {}", absolute, err);
                }
            }
        }
    }

    pub fn reload(&mut self) {
        let Some(path) = self.path_opt.clone() else { return };

        let mut editor = self.editor.lock().expect("editor lock");
        let mut fs_guard = font_system().write().expect("font system write");
        let mut ed = editor.borrow_with(fs_guard.raw());

        match ed.load_text(&path, self.attrs.clone()) {
            Ok(()) => ed.set_changed(false),
            Err(err) => log::error!("failed to reload {:?}: {}", path, err),
        }
    }

    fn get_text_locked(editor: &mut ViEditor<'static, 'static>) -> String {
        // Safest way in this build: borrow with FontSystem and rebuild a String.
        let mut fs_guard = font_system().write().expect("font system write");
        let ed = editor.borrow_with(fs_guard.raw());

        ed.with_buffer(|buffer| {
            let mut out = String::new();
            for (i, line) in buffer.lines.iter().enumerate() {
                if i > 0 {
                    out.push('\n');
                }
                out.push_str(line.text());
            }
            out
        })
    }

    pub fn save(&mut self) {
        let Some(path) = self.path_opt.clone() else { return };

        let mut editor = self.editor.lock().expect("editor lock");
        let text = Self::get_text_locked(&mut editor);

        match fs::write(&path, text) {
            Ok(()) => {
                let mut fs_guard = font_system().write().expect("font system write");
                let mut ed = editor.borrow_with(fs_guard.raw());
                ed.set_changed(false);
            }
            Err(err) => log::error!("failed to save {:?}: {}", path, err),
        }
    }

    // --- Find / replace (used by main.rs) ---

    /// Searches for `regex` and highlights the match.
    /// Returns true if a match is found.
    pub fn search(&self, regex: &Regex, forwards: bool, wrap_around: bool) -> bool {
        let mut editor = self.editor.lock().expect("editor lock");
        let mut cursor = editor.cursor();
        let current_selection = editor.selection();
        let start_line = cursor.line;
        let start_index = cursor.index;
        let mut wrapped = false;

        loop {
            if forwards {
                while cursor.line < editor.with_buffer(|b| b.lines.len()) {
                    let (line_text, line_len) = editor.with_buffer(|b| {
                        if let Some(line) = b.lines.get(cursor.line) {
                            (line.text().to_string(), line.text().len())
                        } else {
                            (String::new(), 0)
                        }
                    });

                    let from = if cursor.line == start_line {
                        // If we already have a selection, allow matching at cursor.index too.
                        if current_selection == Selection::None {
                            cursor.index.saturating_add(1).min(line_len)
                        } else {
                            cursor.index.min(line_len)
                        }
                    } else {
                        0
                    };

                    if from <= line_text.len() {
                        if let Some(m) = regex.find(&line_text[from..]) {
                            let start = from + m.start();
                            let end = from + m.end();

                            let anchor = Cursor::new(cursor.line, start);
                            let end_cur = Cursor::new(cursor.line, end);

                            editor.set_cursor(end_cur);
                            editor.set_selection(Selection::Normal(anchor));
                            return true;
                        }
                    }

                    cursor.line += 1;
                    cursor.index = 0;
                }
            } else {
                // backwards
                loop {
                    let (line_text, line_len) = editor.with_buffer(|b| {
                        if let Some(line) = b.lines.get(cursor.line) {
                            (line.text().to_string(), line.text().len())
                        } else {
                            (String::new(), 0)
                        }
                    });

                    let to = if cursor.line == start_line {
                        if current_selection == Selection::None {
                            cursor.index.saturating_sub(1).min(line_len)
                        } else {
                            cursor.index.min(line_len)
                        }
                    } else {
                        line_len
                    };

                    let to = to.min(line_text.len());
                    let hay = &line_text[..to];

                    if let Some(m) = regex.find_iter(hay).last() {
                        let anchor = Cursor::new(cursor.line, m.start());
                        let end_cur = Cursor::new(cursor.line, m.end());

                        editor.set_cursor(end_cur);
                        editor.set_selection(Selection::Normal(anchor));
                        return true;
                    }

                    if cursor.line == 0 {
                        break;
                    }
                    cursor.line -= 1;
                    cursor.index = usize::MAX; // will be clamped next iteration
                }
            }

            if !wrap_around || wrapped {
                return false;
            }

            wrapped = true;
            if forwards {
                cursor = Cursor::new(0, 0);
            } else {
                cursor = editor.with_buffer(|b| {
                    let last = b.lines.len().saturating_sub(1);
                    let last_len = b.lines.get(last).map(|l| l.text().len()).unwrap_or(0);
                    Cursor::new(last, last_len)
                });
            }

            if cursor.line == start_line && cursor.index == start_index {
                return false;
            }
        }
    }

    /// Replaces the next match (searching forward) with `replacement`.
    /// Returns true if a replacement happened.
    pub fn replace(&self, regex: &Regex, replacement: &str, wrap_around: bool) -> bool {
        // Ensure we have an active match selected; otherwise search forward.
        let editor = self.editor.lock().expect("editor lock");
        let selection = editor.selection();

        let mut should_search = true;
        if let Selection::Normal(anchor) = selection {
            let end = editor.cursor();
            if anchor.line == end.line && anchor.index <= end.index {
                // Verify that the selected text actually matches (avoid replacing arbitrary selection).
                let selected = editor.with_buffer(|b| {
                    if let Some(line) = b.lines.get(anchor.line) {
                        let txt = line.text();
                        let start = anchor.index.min(txt.len());
                        let endi = end.index.min(txt.len());
                        txt[start..endi].to_string()
                    } else {
                        String::new()
                    }
                });
                if regex.is_match(&selected) {
                    should_search = false;
                }
            }
        }

        drop(editor);

        if should_search && !self.search(regex, true, wrap_around) {
            return false;
        }

        let mut editor = self.editor.lock().expect("editor lock");
        let selection = editor.selection();

        let (start_cur, end_cur) = match selection {
            Selection::Normal(anchor) => {
                let end = editor.cursor();
                (anchor, end)
            }
            _ => return false,
        };

        let mut fs_guard = font_system().write().expect("font system write");
        let mut ed = editor.borrow_with(fs_guard.raw());

        ed.delete_range(start_cur, end_cur);
        let new_cur = ed.insert_at(start_cur, replacement, None::<AttrsList>);

        editor.set_cursor(new_cur);
        editor.set_selection(Selection::None);
        true
    }
}

/// A "diff tab" is rendered by main.rs (not by a ViEditor),
/// so it just stores the precomputed diff and a title.
pub struct GitDiffTab {
    pub title: String,
    pub diff: GitDiff,
}