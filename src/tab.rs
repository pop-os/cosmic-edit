// SPDX-License-Identifier: GPL-3.0-only

use cosmic_text::{Attrs, Buffer, Edit, Shaping, SyntaxEditor, ViEditor, Wrap};
use std::{fs, path::PathBuf, sync::Mutex};

use crate::{fl, Config, FONT_SYSTEM, SYNTAX_SYSTEM};

pub struct Tab {
    pub path_opt: Option<PathBuf>,
    attrs: Attrs<'static>,
    pub editor: Mutex<ViEditor<'static>>,
}

impl Tab {
    pub fn new(config: &Config) -> Self {
        //TODO: do not repeat, used in App::init
        let attrs = cosmic_text::Attrs::new().family(cosmic_text::Family::Monospace);

        let mut buffer = Buffer::new_empty(config.metrics());
        buffer.set_text(
            &mut FONT_SYSTEM.lock().unwrap(),
            "",
            attrs,
            Shaping::Advanced,
        );

        let editor = SyntaxEditor::new(
            buffer,
            &SYNTAX_SYSTEM,
            config.syntax_theme(cosmic::theme::is_dark()),
        )
        .unwrap();

        let mut tab = Self {
            path_opt: None,
            attrs,
            editor: Mutex::new(ViEditor::new(editor)),
        };

        // Update any other config settings
        tab.set_config(config);

        tab
    }

    pub fn set_config(&mut self, config: &Config) {
        let mut editor = self.editor.lock().unwrap();
        let mut font_system = FONT_SYSTEM.lock().unwrap();
        let mut editor = editor.borrow_with(&mut font_system);
        editor.set_passthrough(!config.vim_bindings);
        editor.buffer_mut().set_wrap(if config.word_wrap {
            Wrap::Word
        } else {
            Wrap::None
        });
        //TODO: dynamically discover light/dark changes
        editor.update_theme(if cosmic::theme::is_dark() {
            &config.syntax_theme_dark
        } else {
            &config.syntax_theme_light
        });
    }

    pub fn open(&mut self, path: PathBuf) {
        let mut editor = self.editor.lock().unwrap();
        let mut font_system = FONT_SYSTEM.lock().unwrap();
        let mut editor = editor.borrow_with(&mut font_system);
        match editor.load_text(&path, self.attrs) {
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

    pub fn save(&mut self) {
        if let Some(path) = &self.path_opt {
            let editor = self.editor.lock().unwrap();
            let mut text = String::new();
            for line in editor.buffer().lines.iter() {
                text.push_str(line.text());
                text.push('\n');
            }
            match fs::write(path, text) {
                Ok(()) => {
                    log::info!("saved {:?}", path);
                }
                Err(err) => {
                    log::error!("failed to save {:?}: {}", path, err);
                }
            }
        } else {
            log::warn!("tab has no path yet");
        }
    }

    pub fn title(&self) -> String {
        //TODO: show full title when there is a conflict
        if let Some(path) = &self.path_opt {
            match path.file_name() {
                Some(file_name_os) => match file_name_os.to_str() {
                    Some(file_name) => file_name.to_string(),
                    None => format!("{}", path.display()),
                },
                None => format!("{}", path.display()),
            }
        } else {
            fl!("new-document")
        }
    }
}
