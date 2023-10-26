// SPDX-License-Identifier: GPL-3.0-only

use cosmic::{
    app::{message, Command, Core, Settings},
    executor,
    iced::{
        widget::{column, text},
        Length, Limits,
    },
    widget::{self, icon, segmented_button, view_switcher},
    ApplicationExt, Element,
};
use cosmic_text::{FontSystem, SyntaxSystem, ViMode};
use std::{
    env,
    path::{Path, PathBuf},
    sync::Mutex,
};

use self::menu::menu_bar;
mod menu;

use self::project::Project;
mod project;

use self::tab::Tab;
mod tab;

use self::text_box::text_box;
mod text_box;

//TODO: re-use iced FONT_SYSTEM
lazy_static::lazy_static! {
    static ref FONT_SYSTEM: Mutex<FontSystem> = Mutex::new(FontSystem::new());
    static ref SYNTAX_SYSTEM: SyntaxSystem = SyntaxSystem::new();
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let settings = Settings::default().size_limits(Limits::NONE.min_width(400.0).min_height(200.0));
    let flags = ();
    cosmic::app::run::<App>(settings, flags)?;

    Ok(())
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
        let menu_bar = menu_bar();

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
