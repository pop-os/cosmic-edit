// SPDX-License-Identifier: GPL-3.0-only

use cosmic::{
    app::{Command, Core, Settings},
    executor,
    iced::{
        widget::{column, row, text},
        Alignment, Length, Limits,
    },
    widget::{icon, segmented_button, view_switcher},
    ApplicationExt, Element,
};
use cosmic_text::{Attrs, Buffer, Edit, FontSystem, Metrics, SyntaxEditor, SyntaxSystem};
use std::{env, fs, path::PathBuf, sync::Mutex};

use self::menu_list::MenuList;
mod menu_list;

use self::text_box::text_box;
mod text_box;

lazy_static::lazy_static! {
    static ref FONT_SYSTEM: Mutex<FontSystem> = Mutex::new(FontSystem::new());
    static ref SYNTAX_SYSTEM: SyntaxSystem = SyntaxSystem::new();
}

static FONT_SIZES: &'static [Metrics] = &[
    Metrics::new(10.0, 14.0), // Caption
    Metrics::new(14.0, 20.0), // Body
    Metrics::new(20.0, 28.0), // Title 4
    Metrics::new(24.0, 32.0), // Title 3
    Metrics::new(28.0, 36.0), // Title 2
    Metrics::new(32.0, 44.0), // Title 1
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let settings = Settings::default()
        .size_limits(Limits::NONE.min_width(400.0).min_height(200.0));
    let flags = ();
    cosmic::app::run::<App>(settings, flags)?;

    Ok(())
}

pub struct Tab {
    path_opt: Option<PathBuf>,
    attrs: Attrs<'static>,
    #[cfg(not(feature = "vi"))]
    editor: Mutex<SyntaxEditor<'static>>,
    #[cfg(feature = "vi")]
    editor: Mutex<cosmic_text::ViEditor<'static>>,
}

impl Tab {
    pub fn new() -> Self {
        let attrs = cosmic_text::Attrs::new().family(cosmic_text::Family::Monospace);

        let editor = SyntaxEditor::new(
            Buffer::new(&mut FONT_SYSTEM.lock().unwrap(), FONT_SIZES[1 /* Body */]),
            &SYNTAX_SYSTEM,
            "base16-eighties.dark",
        )
        .unwrap();

        #[cfg(feature = "vi")]
        let editor = cosmic_text::ViEditor::new(editor);

        Self {
            path_opt: None,
            attrs,
            editor: Mutex::new(editor),
        }
    }

    pub fn open(&mut self, path: PathBuf) {
        let mut editor = self.editor.lock().unwrap();
        let mut font_system = FONT_SYSTEM.lock().unwrap();
        let mut editor = editor.borrow_with(&mut font_system);
        match editor.load_text(&path, self.attrs) {
            Ok(()) => {
                log::info!("opened '{}'", path.display());
                self.path_opt = Some(path);
            }
            Err(err) => {
                log::error!("failed to open '{}': {}", path.display(), err);
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
                    log::info!("saved '{}'", path.display());
                }
                Err(err) => {
                    log::error!("failed to save '{}': {}", path.display(), err);
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
            "New document".to_string()
        }
    }
}

pub struct App {
    core: Core,
    tab_model: segmented_button::SingleSelectModel,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub enum Message {
    Open,
    Save,
    TabActivate(segmented_button::Entity),
    TabClose(segmented_button::Entity),
    Todo,
}

impl App {
    pub fn active_tab(&self) -> Option<&Tab> {
        self.tab_model.active_data()
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tab_model.active_data_mut()
    }

    pub fn open_tab(&mut self, path_opt: Option<PathBuf>) {
        let mut tab = Tab::new();
        if let Some(path) = path_opt {
            tab.open(path);
        }
        self.tab_model
            .insert()
            .text(tab.title())
            .icon(icon::from_name("text-x-generic").icon())
            .data(tab)
            .closable()
            .activate();
    }

    pub fn update_title(&mut self) -> Command<Message> {
        let title = match self.active_tab() {
            Some(tab) => tab.title(),
            None => format!("No Open File"),
        };
        let window_title = format!("{title} - COSMIC Text Editor");
        self.core.window.header_title = title.clone();
        self.set_title(window_title)
    }
}

/// Implement [`cosmic::Application`] to integrate with COSMIC.
impl cosmic::Application for App {
    /// Default async executor to use with the app.
    type Executor = executor::Default;

    /// Argument received [`cosmic::Application::new`].
    type Flags = ();

    /// Message type specific to our [`App`].
    type Message = Message;

    /// The unique application ID to supply to the window manager.
    const APP_ID: &'static str = "com.system76.CosmicTextEditor";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    /// Creates the application, and optionally emits command on initialize.
    fn init(core: Core, _flags: Self::Flags) -> (Self, Command<Self::Message>) {
        let mut app = App {
            core,
            tab_model: segmented_button::Model::builder().build(),
        };

        for path in env::args().skip(1) {
            app.open_tab(Some(PathBuf::from(path)));
        }

        // Open an empty file if no arguments provided
        if app.tab_model.iter().next().is_none() {
            app.open_tab(None);
        }

        let command = app.update_title();
        (app, command)
    }

    fn update(&mut self, message: Message) -> Command<Self::Message> {
        match message {
            Message::Open => {
                if let Some(path) = rfd::FileDialog::new().pick_file() {
                    self.open_tab(Some(path));
                    return self.update_title();
                }
            }
            Message::Save => {
                let mut title_opt = None;

                match self.active_tab_mut() {
                    Some(tab) => {
                        if tab.path_opt.is_none() {
                            tab.path_opt = rfd::FileDialog::new().save_file();
                            title_opt = Some(tab.title());
                        }
                        tab.save();
                    }
                    None => {
                        log::warn!("TODO: NO TAB OPEN");
                    }
                }

                if let Some(title) = title_opt {
                    self.tab_model.text_set(self.tab_model.active(), title);
                }
            }
            Message::TabActivate(entity) => {
                self.tab_model.activate(entity);
                return self.update_title();
            }
            Message::TabClose(entity) => {
                // Activate closest item
                if let Some(position) = self.tab_model.position(entity) {
                    if position > 0 {
                        self.tab_model.activate_position(position - 1);
                    } else {
                        self.tab_model.activate_position(position + 1);
                    }
                }

                // Remove item
                self.tab_model.remove(entity);

                // If that was the last tab, make a new empty one
                if self.tab_model.iter().next().is_none() {
                    self.open_tab(None);
                }

                return self.update_title();
            }
            Message::Todo => {
                log::warn!("TODO");
            }
        }

        Command::none()
    }

    fn view(&self) -> Element<Message> {
        let menu_bar = row![
            MenuList::new(vec!["Open", "Save"], None, |item| {
                match item {
                    "Open" => Message::Open,
                    "Save" => Message::Save,
                    _ => Message::Todo,
                }
            })
            .padding(8)
            .placeholder("File"),
            MenuList::new(vec!["Todo"], None, |_| Message::Todo).placeholder("Edit"),
            MenuList::new(vec!["Todo"], None, |_| Message::Todo).placeholder("View"),
            MenuList::new(vec!["Todo"], None, |_| Message::Todo).placeholder("Help"),
        ]
        .align_items(Alignment::Start)
        .padding(4)
        .spacing(16);

        let tab_bar = view_switcher::horizontal(&self.tab_model)
            .on_activate(Message::TabActivate)
            .on_close(Message::TabClose)
            .width(Length::Shrink);

        let active_tab: Element<_> = match self.active_tab() {
            Some(tab) => text_box(&tab.editor).padding(8).into(),
            None => {
                log::warn!("TODO: No tab open");
                text("no tab active").into()
            }
        };

        let content: Element<_> =
            column![menu_bar, column![tab_bar, active_tab,].padding([0, 16])].into();

        // Uncomment to debug layout:
        //content.explain(Color::WHITE)
        content
    }
}
