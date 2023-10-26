// SPDX-License-Identifier: GPL-3.0-only

use cosmic::{
    app::{message, Command, Core, Settings},
    executor,
    iced::{
        widget::{column, horizontal_rule, horizontal_space, row, text},
        Alignment, Length, Limits,
    },
    theme,
    widget::{
        self, button, icon,
        menu::{ItemHeight, ItemWidth, MenuBar, MenuTree},
        segmented_button, view_switcher,
    },
    ApplicationExt, Element,
};
use cosmic_text::{
    Attrs, Buffer, Edit, FontSystem, Metrics, SyntaxEditor, SyntaxSystem, ViEditor, ViMode,
};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    sync::Mutex,
};

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

    let settings = Settings::default().size_limits(Limits::NONE.min_width(400.0).min_height(200.0));
    let flags = ();
    cosmic::app::run::<App>(settings, flags)?;

    Ok(())
}

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

pub struct Tab {
    path_opt: Option<PathBuf>,
    attrs: Attrs<'static>,
    editor: Mutex<ViEditor<'static>>,
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

        let mut editor = ViEditor::new(editor);
        editor.set_passthrough(false);

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
    projects: Vec<Project>,
    tab_model: segmented_button::SingleSelectModel,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum Message {
    New,
    OpenDialog,
    Open(PathBuf),
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

    pub fn open_project<P: AsRef<Path>>(&mut self, path: P) {
        match Project::new(&path) {
            Ok(project) => self.projects.push(project),
            Err(err) => {
                log::error!("failed to open '{}': {}", path.as_ref().display(), err);
            }
        }
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
        self.set_header_title(title.clone());
        self.set_window_title(window_title)
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
            projects: Vec::new(),
            tab_model: segmented_button::Model::builder().build(),
        };

        for arg in env::args().skip(1) {
            let path = PathBuf::from(arg);
            if path.is_dir() {
                app.open_project(path);
            } else {
                app.open_tab(Some(path));
            }
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
            Message::New => {
                self.open_tab(None);
                return self.update_title();
            }
            Message::OpenDialog => {
                return Command::perform(
                    async {
                        if let Some(handle) = rfd::AsyncFileDialog::new().pick_file().await {
                            println!("{}", handle.path().display());
                            message::app(Message::Open(handle.path().to_owned()))
                        } else {
                            message::none()
                        }
                    },
                    |x| x,
                );
            }
            Message::Open(path) => {
                self.open_tab(Some(path));
                return self.update_title();
            }
            Message::Save => {
                let mut title_opt = None;

                match self.active_tab_mut() {
                    Some(tab) => {
                        if tab.path_opt.is_none() {
                            //TODO: use async file dialog
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
        /*
        let menu_bar = row![
            MenuList::new(
                vec![
                    "New file",
                    "New window",
                    "Open file...",
                    "Save",
                    "Save as..."
                ],
                None,
                |item| {
                    match item {
                        "Open" => Message::OpenDialog,
                        "Save" => Message::Save,
                        _ => Message::Todo,
                    }
                }
            )
            .padding(8)
            .placeholder("File"),
            MenuList::new(vec!["Todo"], None, |_| Message::Todo).placeholder("Edit"),
            MenuList::new(vec!["Todo"], None, |_| Message::Todo).placeholder("View"),
            MenuList::new(vec!["Todo"], None, |_| Message::Todo).placeholder("Help"),
        ]
        .align_items(Alignment::Start)
        .padding(4)
        .spacing(16);
        */

        //TODO: port to libcosmic
        let menu_root = |label| {
            button(label)
                .padding([4, 12])
                .style(theme::Button::MenuRoot)
        };
        let menu_folder = |label| {
            button(
                row![text(label), horizontal_space(Length::Fill), text(">")]
                    .align_items(Alignment::Center),
            )
            .height(Length::Fixed(32.0))
            .padding([4, 12])
            .width(Length::Fill)
            .style(theme::Button::MenuItem)
        };
        let menu_item = |label, message| {
            MenuTree::new(
                button(row![label].align_items(Alignment::Center))
                    .height(Length::Fixed(32.0))
                    .on_press(message)
                    .padding([4, 12])
                    .width(Length::Fill)
                    .style(theme::Button::MenuItem),
            )
        };
        let menu_key = |label, key, message| {
            MenuTree::new(
                button(
                    row![text(label), horizontal_space(Length::Fill), text(key)]
                        .align_items(Alignment::Center),
                )
                .height(Length::Fixed(32.0))
                .on_press(message)
                .padding([4, 12])
                .style(theme::Button::MenuItem),
            )
        };
        let menu_bar: Element<_> = MenuBar::new(vec![
            MenuTree::with_children(
                menu_root("File"),
                vec![
                    menu_key("New file", "Ctrl + N", Message::New),
                    menu_key("New window", "Ctrl + Shift + N", Message::Todo),
                    MenuTree::new(horizontal_rule(1)),
                    menu_key("Open file...", "Ctrl + O", Message::OpenDialog),
                    MenuTree::with_children(
                        menu_folder("Open recent"),
                        vec![menu_item("TODO", Message::Todo)],
                    ),
                    MenuTree::new(horizontal_rule(1)),
                    menu_key("Save", "Ctrl + S", Message::Save),
                    menu_key("Save as...", "Ctrl + Shift + S", Message::Todo),
                    MenuTree::new(horizontal_rule(1)),
                    menu_item("Revert all changes", Message::Todo),
                    MenuTree::new(horizontal_rule(1)),
                    menu_item("Document statistics...", Message::Todo),
                    menu_item("Document type...", Message::Todo),
                    menu_item("Encoding...", Message::Todo),
                    menu_item("Print", Message::Todo),
                    MenuTree::new(horizontal_rule(1)),
                    menu_key("Quit", "Ctrl + Q", Message::Todo),
                ],
            ),
            MenuTree::with_children(
                menu_root("Edit"),
                vec![
                    menu_key("Undo", "Ctrl + Z", Message::Todo),
                    menu_key("Redo", "Ctrl + Shift + Z", Message::Todo),
                    MenuTree::new(horizontal_rule(1)),
                    menu_key("Cut", "Ctrl + X", Message::Todo),
                    menu_key("Copy", "Ctrl + C", Message::Todo),
                    menu_key("Paste", "Ctrl + V", Message::Todo),
                    MenuTree::new(horizontal_rule(1)),
                    menu_key("Find", "Ctrl + F", Message::Todo),
                    menu_key("Replace", "Ctrl + H", Message::Todo),
                    MenuTree::new(horizontal_rule(1)),
                    menu_item("Spell check...", Message::Todo),
                ],
            ),
            MenuTree::with_children(
                menu_root("View"),
                vec![
                    MenuTree::with_children(
                        menu_folder("Indentation"),
                        vec![
                            menu_item("Automatic indentation", Message::Todo),
                            MenuTree::new(horizontal_rule(1)),
                            menu_item("Tab width: 1", Message::Todo),
                            menu_item("Tab width: 2", Message::Todo),
                            menu_item("Tab width: 4", Message::Todo),
                            menu_item("Tab width: 8", Message::Todo),
                            MenuTree::new(horizontal_rule(1)),
                            menu_item("Convert indentation to spaces", Message::Todo),
                            menu_item("Convert indentation to tabs", Message::Todo),
                        ],
                    ),
                    MenuTree::new(horizontal_rule(1)),
                    menu_item("Word wrap", Message::Todo),
                    menu_item("Show line numbers", Message::Todo),
                    menu_item("Highlight current line", Message::Todo),
                    menu_item("Syntax highlighting...", Message::Todo),
                    MenuTree::new(horizontal_rule(1)),
                    menu_key("Settings...", "Ctrl + ,", Message::Todo),
                    MenuTree::new(horizontal_rule(1)),
                    menu_item("Keyboard shortcuts...", Message::Todo),
                    MenuTree::new(horizontal_rule(1)),
                    menu_item("About COSMIC Text Editor", Message::Todo),
                ],
            ),
        ])
        .cross_offset(0)
        .item_height(ItemHeight::Dynamic(40))
        .item_width(ItemWidth::Uniform(240))
        .main_offset(0)
        .padding(8)
        .spacing(4.0)
        .into();

        let mut tab_column = widget::column::with_capacity(3).padding([0, 16]);

        tab_column = tab_column.push(
            view_switcher::horizontal(&self.tab_model)
                .on_activate(Message::TabActivate)
                .on_close(Message::TabClose)
                .width(Length::Shrink),
        );

        match self.active_tab() {
            Some(tab) => {
                tab_column = tab_column.push(text_box(&tab.editor).padding(8));
                let status = match tab.editor.lock().unwrap().mode() {
                    ViMode::Passthrough => {
                        //TODO: status line
                        String::new()
                    }
                    ViMode::Normal => {
                        //TODO: status line
                        String::new()
                    }
                    ViMode::Insert => {
                        format!("-- INSERT --")
                    }
                    ViMode::Command { value } => {
                        format!(":{value}|")
                    }
                    ViMode::Search { value, forwards } => {
                        if *forwards {
                            format!("/{value}|")
                        } else {
                            format!("?{value}|")
                        }
                    }
                };
                tab_column = tab_column.push(text(status).font(cosmic::font::Font::MONOSPACE));
            }
            None => {
                log::warn!("TODO: No tab open");
            }
        };

        let mut project_row = widget::row::with_capacity(2);
        if !self.projects.is_empty() {
            /*TODO: project tree view
            let mut project_list = widget::column::with_capacity(self.projects.len());
            for project in self.projects.iter() {
                project_list = project_list.push(widget::text(&project.name));
            }
            project_row = project_row.push(project_list);
            */
        }
        project_row = project_row.push(tab_column);

        let content: Element<_> = column![menu_bar, project_row].into();

        // Uncomment to debug layout:
        //content.explain(cosmic::iced::Color::WHITE)
        content
    }
}
