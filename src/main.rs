// SPDX-License-Identifier: GPL-3.0-only

use cosmic::{
    app::{message, Command, Core, Settings},
    executor,
    iced::{
        clipboard, event, keyboard, subscription,
        widget::{row, text},
        window, Alignment, Length, Limits,
    },
    style,
    widget::{self, button, icon, nav_bar, segmented_button, view_switcher},
    ApplicationExt, Element,
};
use cosmic_text::{Edit, FontSystem, SwashCache, SyntaxSystem, ViMode};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
    sync::Mutex,
};

use config::{Config, KeyBind};
mod config;

mod localize;

use self::menu::menu_bar;
mod menu;

use self::project::ProjectNode;
mod project;

use self::tab::Tab;
mod tab;

use self::text_box::text_box;
mod text_box;

//TODO: re-use iced FONT_SYSTEM
lazy_static::lazy_static! {
    static ref FONT_SYSTEM: Mutex<FontSystem> = Mutex::new(FontSystem::new());
    static ref SWASH_CACHE: Mutex<SwashCache> = Mutex::new(SwashCache::new());
    static ref SYNTAX_SYSTEM: SyntaxSystem = SyntaxSystem::new();
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    localize::localize();

    let settings = Settings::default();

    //TODO: allow size limits on iced_winit
    //settings = settings.size_limits(Limits::NONE.min_width(400.0).min_height(200.0));

    let flags = ();
    cosmic::app::run::<App>(settings, flags)?;

    Ok(())
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Message {
    Cut,
    Copy,
    KeyBind(KeyBind),
    NewFile,
    NewWindow,
    OpenFileDialog,
    OpenFile(PathBuf),
    OpenProjectDialog,
    OpenProject(PathBuf),
    Paste,
    PasteValue(String),
    Quit,
    Save,
    TabActivate(segmented_button::Entity),
    TabClose(segmented_button::Entity),
    Todo,
    ToggleContextPage(ContextPage),
    ToggleWordWrap,
    VimBindings(bool),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextPage {
    Settings,
}

impl ContextPage {
    fn title(&self) -> String {
        match self {
            Self::Settings => fl!("settings"),
        }
    }
}

pub struct App {
    core: Core,
    nav_model: segmented_button::SingleSelectModel,
    tab_model: segmented_button::SingleSelectModel,
    config: Config,
    context_page: ContextPage,
}

impl App {
    pub fn active_tab(&self) -> Option<&Tab> {
        self.tab_model.active_data()
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tab_model.active_data_mut()
    }

    fn open_folder<P: AsRef<Path>>(&mut self, path: P, mut position: u16, indent: u16) {
        let read_dir = match fs::read_dir(&path) {
            Ok(ok) => ok,
            Err(err) => {
                log::error!("failed to read directory {:?}: {}", path.as_ref(), err);
                return;
            }
        };

        let mut nodes = Vec::new();
        for entry_res in read_dir {
            let entry = match entry_res {
                Ok(ok) => ok,
                Err(err) => {
                    log::error!(
                        "failed to read entry in directory {:?}: {}",
                        path.as_ref(),
                        err
                    );
                    continue;
                }
            };

            let entry_path = entry.path();
            let node = match ProjectNode::new(&entry_path) {
                Ok(ok) => ok,
                Err(err) => {
                    log::error!(
                        "failed to open directory {:?} entry {:?}: {}",
                        path.as_ref(),
                        entry_path,
                        err
                    );
                    continue;
                }
            };
            nodes.push(node);
        }

        nodes.sort();

        for node in nodes {
            self.nav_model
                .insert()
                .position(position)
                .indent(indent)
                .icon(icon::from_name(node.icon_name()).size(16).icon())
                .text(node.name().to_string())
                .data(node);

            position += 1;
        }
    }

    pub fn open_project<P: AsRef<Path>>(&mut self, path: P) {
        let node = match ProjectNode::new(&path) {
            Ok(mut node) => {
                match &mut node {
                    ProjectNode::Folder { open, root, .. } => {
                        *open = true;
                        *root = true;
                    }
                    _ => {
                        log::error!(
                            "failed to open project {:?}: not a directory",
                            path.as_ref()
                        );
                        return;
                    }
                }
                node
            }
            Err(err) => {
                log::error!("failed to open project {:?}: {}", path.as_ref(), err);
                return;
            }
        };

        let id = self
            .nav_model
            .insert()
            .icon(icon::from_name(node.icon_name()).size(16).icon())
            .text(node.name().to_string())
            .data(node)
            .id();

        let position = self.nav_model.position(id).unwrap_or(0);

        self.open_folder(&path, position + 1, 1);
    }

    pub fn open_tab(&mut self, path_opt: Option<PathBuf>) {
        let mut tab = Tab::new();
        tab.set_config(&self.config);
        if let Some(path) = path_opt {
            tab.open(path);
        }
        self.tab_model
            .insert()
            .text(tab.title())
            .icon(icon::from_name("text-x-generic").size(16).icon())
            .data::<Tab>(tab)
            .closable()
            .activate();
    }

    fn update_config(&mut self) {
        //TODO: provide iterator over data
        let entities: Vec<_> = self.tab_model.iter().collect();
        for entity in entities {
            if let Some(tab) = self.tab_model.data_mut::<Tab>(entity) {
                tab.set_config(&self.config);
            }
        }
    }

    fn update_nav_bar_active(&mut self) {
        let tab_path_opt = match self.active_tab() {
            Some(tab) => tab.path_opt.clone(),
            None => None,
        };

        // Locate tree node to activate
        let mut active_id = segmented_button::Entity::default();
        match tab_path_opt {
            Some(tab_path) => {
                // Automatically expand tree to find and select active file
                loop {
                    let mut expand_opt = None;
                    for id in self.nav_model.iter() {
                        match self.nav_model.data(id) {
                            Some(node) => match node {
                                ProjectNode::Folder { path, open, .. } => {
                                    if tab_path.starts_with(path) && !*open {
                                        expand_opt = Some(id);
                                        break;
                                    }
                                }
                                ProjectNode::File { path, .. } => {
                                    if path == &tab_path {
                                        active_id = id;
                                        break;
                                    }
                                }
                            },
                            None => {}
                        }
                    }
                    match expand_opt {
                        Some(id) => {
                            //TODO: can this be optimized?
                            // Command not used becuase opening a folder just returns Command::none
                            let _ = cosmic::Application::on_nav_select(self, id);
                        }
                        None => {
                            break;
                        }
                    }
                }
            }
            None => {}
        }
        self.nav_model.activate(active_id);
    }

    // Call this any time the tab changes
    pub fn update_tab(&mut self) -> Command<Message> {
        self.update_nav_bar_active();

        let title = match self.active_tab() {
            Some(tab) => {
                // Hack to ensure redraw on changing tabs
                tab.editor.lock().unwrap().buffer_mut().set_redraw(true);
                tab.title()
            }
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
    const APP_ID: &'static str = "com.system76.CosmicEdit";

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
            nav_model: nav_bar::Model::builder().build(),
            tab_model: segmented_button::Model::builder().build(),
            config: Config::load(),
            context_page: ContextPage::Settings,
        };

        for arg in env::args().skip(1) {
            let path = PathBuf::from(arg);
            if path.is_dir() {
                app.open_project(path);
            } else {
                app.open_tab(Some(path));
            }
        }

        // Show nav bar only if project is provided
        if app.core.nav_bar_active() != app.nav_model.iter().next().is_some() {
            app.core.nav_bar_toggle();
            app.nav_model
                .insert()
                .icon(icon::from_name("folder-open-symbolic").size(16).icon())
                .text(fl!("open-project"));
        }

        // Open an empty file if no arguments provided
        if app.tab_model.iter().next().is_none() {
            app.open_tab(None);
        }

        let command = app.update_tab();
        (app, command)
    }

    fn nav_model(&self) -> Option<&nav_bar::Model> {
        Some(&self.nav_model)
    }

    fn on_nav_select(&mut self, id: nav_bar::Id) -> Command<Message> {
        // Toggle open state and get clone of node data
        let node_opt = match self.nav_model.data_mut::<ProjectNode>(id) {
            Some(node) => {
                match node {
                    ProjectNode::Folder { open, .. } => {
                        *open = !*open;
                    }
                    _ => {}
                }
                Some(node.clone())
            }
            None => None,
        };

        match node_opt {
            Some(node) => {
                // Update icon
                self.nav_model
                    .icon_set(id, icon::from_name(node.icon_name()).size(16).icon());

                match node {
                    ProjectNode::Folder { path, open, .. } => {
                        let position = self.nav_model.position(id).unwrap_or(0);
                        let indent = self.nav_model.indent(id).unwrap_or(0);
                        if open {
                            // Open folder
                            self.open_folder(path, position + 1, indent + 1);
                        } else {
                            // Close folder
                            loop {
                                let child_id = match self.nav_model.entity_at(position + 1) {
                                    Some(some) => some,
                                    None => break,
                                };

                                if self.nav_model.indent(child_id).unwrap_or(0) > indent {
                                    self.nav_model.remove(child_id);
                                } else {
                                    break;
                                }
                            }
                        }
                        Command::none()
                    }
                    ProjectNode::File { path, .. } => {
                        //TODO: go to already open file if possible
                        self.update(Message::OpenFile(path))
                    }
                }
            }
            None => {
                // Open project
                self.update(Message::OpenProjectDialog)
            }
        }
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::Cut => match self.active_tab() {
                Some(tab) => {
                    let mut editor = tab.editor.lock().unwrap();
                    let selection_opt = editor.copy_selection();
                    editor.delete_selection();
                    if let Some(selection) = selection_opt {
                        return clipboard::write(selection);
                    }
                }
                None => {}
            },
            Message::Copy => match self.active_tab() {
                Some(tab) => {
                    let editor = tab.editor.lock().unwrap();
                    let selection_opt = editor.copy_selection();
                    if let Some(selection) = selection_opt {
                        return clipboard::write(selection);
                    }
                }
                None => {}
            },
            Message::KeyBind(key_bind) => {
                for (config_key_bind, config_message) in self.config.keybinds.iter() {
                    if config_key_bind == &key_bind {
                        return self.update(config_message.clone());
                    }
                }
            }
            Message::NewFile => {
                self.open_tab(None);
                return self.update_tab();
            }
            Message::NewWindow => {
                //TODO: support multi-window in winit
                match env::current_exe() {
                    Ok(exe) => match process::Command::new(&exe).spawn() {
                        Ok(child) => {}
                        Err(err) => {
                            log::error!("failed to execute {:?}: {}", exe, err);
                        }
                    },
                    Err(err) => {
                        log::error!("failed to get current executable path: {}", err);
                    }
                }
            }
            Message::OpenFileDialog => {
                return Command::perform(
                    async {
                        if let Some(handle) = rfd::AsyncFileDialog::new().pick_file().await {
                            message::app(Message::OpenFile(handle.path().to_owned()))
                        } else {
                            message::none()
                        }
                    },
                    |x| x,
                );
            }
            Message::OpenFile(path) => {
                self.open_tab(Some(path));
                return self.update_tab();
            }
            Message::OpenProjectDialog => {
                return Command::perform(
                    async {
                        if let Some(handle) = rfd::AsyncFileDialog::new().pick_folder().await {
                            message::app(Message::OpenProject(handle.path().to_owned()))
                        } else {
                            message::none()
                        }
                    },
                    |x| x,
                );
            }
            Message::OpenProject(path) => {
                self.open_project(path);
            }
            Message::Paste => {
                return clipboard::read(|value_opt| match value_opt {
                    Some(value) => message::app(Message::PasteValue(value)),
                    None => message::none(),
                });
            }
            Message::PasteValue(value) => {
                println!("Paste {:?}", value);
                match self.active_tab() {
                    Some(tab) => {
                        let mut editor = tab.editor.lock().unwrap();
                        editor.insert_string(&value, None);
                    }
                    None => {}
                }
            }
            Message::Quit => {
                //TODO: prompt for save?
                return window::close();
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
                return self.update_tab();
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

                return self.update_tab();
            }
            Message::Todo => {
                log::warn!("TODO");
            }
            Message::ToggleContextPage(context_page) => {
                if self.context_page == context_page {
                    self.core.window.show_context = !self.core.window.show_context;
                } else {
                    self.context_page = context_page;
                    self.core.window.show_context = true;
                }
                self.set_context_title(context_page.title());

                // Hack to ensure tab redraws.
                //TODO: tab does not redraw when using Close button!
                return self.update_tab();
            }
            Message::ToggleWordWrap => {
                self.config.word_wrap = !self.config.word_wrap;
                self.update_config();
            }
            Message::VimBindings(vim_bindings) => {
                self.config.vim_bindings = vim_bindings;
                self.update_config();
            }
        }

        Command::none()
    }

    fn context_drawer(&self) -> Option<Element<Message>> {
        if !self.core.window.show_context {
            return None;
        }

        Some(match self.context_page {
            ContextPage::Settings => {
                widget::settings::view_column(vec![widget::settings::view_section(fl!(
                    "keyboard-shortcuts"
                ))
                .add(
                    widget::settings::item::builder(fl!("enable-vim-bindings"))
                        .toggler(self.config.vim_bindings, Message::VimBindings),
                )
                .into()])
                .into()
            }
        })
    }

    fn header_start(&self) -> Vec<Element<Message>> {
        vec![menu_bar(&self.config)]
    }

    fn view(&self) -> Element<Message> {
        let mut tab_column = widget::column::with_capacity(3).padding([0, 16]);

        tab_column = tab_column.push(
            row![
                view_switcher::horizontal(&self.tab_model)
                    .on_activate(Message::TabActivate)
                    .on_close(Message::TabClose)
                    .width(Length::Shrink),
                button(icon::from_name("list-add-symbolic").size(16).icon())
                    .on_press(Message::NewFile)
                    .padding(8)
                    .style(style::Button::Icon)
            ]
            .align_items(Alignment::Center),
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

        let content: Element<_> = tab_column.into();

        // Uncomment to debug layout:
        //content.explain(cosmic::iced::Color::WHITE)
        content
    }

    fn subscription(&self) -> subscription::Subscription<Message> {
        subscription::events_with(|event, status| match event {
            event::Event::Keyboard(keyboard::Event::KeyPressed {
                modifiers,
                key_code,
            }) => Some(Message::KeyBind(KeyBind {
                modifiers,
                key_code,
            })),
            _ => None,
        })
    }
}
