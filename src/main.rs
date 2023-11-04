// SPDX-License-Identifier: GPL-3.0-only

use cosmic::{
    app::{message, Command, Core, Settings},
    cosmic_config::{self, CosmicConfigEntry},
    executor,
    iced::{
        clipboard, event, keyboard, subscription,
        widget::{row, text},
        window, Alignment, Length,
    },
    style,
    widget::{self, button, icon, nav_bar, segmented_button, view_switcher},
    ApplicationExt, Element,
};
use cosmic_text::{Edit, Family, FontSystem, SwashCache, SyntaxSystem, ViMode};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
    sync::Mutex,
};

use config::{Config, CONFIG_VERSION};
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
    Config(Config),
    Copy,
    Cut,
    DefaultFont(usize),
    DefaultFontSize(usize),
    Key(keyboard::Modifiers, keyboard::KeyCode),
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
    SyntaxTheme(usize, bool),
    TabActivate(segmented_button::Entity),
    TabClose(segmented_button::Entity),
    Todo,
    ToggleContextPage(ContextPage),
    ToggleWordWrap,
    VimBindings(bool),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextPage {
    DocumentStatistics,
    Settings,
}

impl ContextPage {
    fn title(&self) -> String {
        match self {
            Self::DocumentStatistics => fl!("document-statistics"),
            Self::Settings => fl!("settings"),
        }
    }
}

pub struct App {
    core: Core,
    nav_model: segmented_button::SingleSelectModel,
    tab_model: segmented_button::SingleSelectModel,
    config_handler: Option<cosmic_config::Config>,
    config: Config,
    font_names: Vec<String>,
    font_size_names: Vec<String>,
    font_sizes: Vec<u16>,
    theme_names: Vec<String>,
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
        let mut tab = Tab::new(&self.config);
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

    fn save_config(&mut self) {
        match self.config_handler {
            Some(ref config_handler) => match self.config.write_entry(&config_handler) {
                Ok(()) => {}
                Err(err) => {
                    log::error!("failed to save config: {}", err);
                }
            },
            None => {
                //TODO: log that there is no handler?
            }
        }
        self.update_config();
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
        let (config_handler, config) =
            match cosmic_config::Config::new(Self::APP_ID, CONFIG_VERSION) {
                Ok(config_handler) => {
                    let config = match Config::get_entry(&config_handler) {
                        Ok(ok) => ok,
                        Err((errs, config)) => {
                            log::warn!("errors loading config: {:?}", errs);
                            config
                        }
                    };
                    (Some(config_handler), config)
                }
                Err(err) => {
                    log::error!("failed to create config handler: {}", err);
                    (None, Config::default())
                }
            };

        let font_names = {
            let mut font_names = Vec::new();
            let font_system = FONT_SYSTEM.lock().unwrap();
            //TODO: do not repeat, used in Tab::new
            let attrs = cosmic_text::Attrs::new().family(Family::Monospace);
            for face in font_system.db().faces() {
                if attrs.matches(face) && face.monospaced {
                    //TODO: get localized name if possible
                    let font_name = face
                        .families
                        .get(0)
                        .map_or_else(|| face.post_script_name.to_string(), |x| x.0.to_string());
                    font_names.push(font_name);
                }
            }
            font_names.sort();
            font_names
        };

        let mut font_size_names = Vec::new();
        let mut font_sizes = Vec::new();
        for font_size in 4..=32 {
            font_size_names.push(format!("{}px", font_size));
            font_sizes.push(font_size);
        }

        let mut theme_names = Vec::with_capacity(SYNTAX_SYSTEM.theme_set.themes.len());
        for (theme_name, _theme) in SYNTAX_SYSTEM.theme_set.themes.iter() {
            theme_names.push(theme_name.to_string());
        }

        let mut app = App {
            core,
            nav_model: nav_bar::Model::builder().build(),
            tab_model: segmented_button::Model::builder().build(),
            config_handler,
            config,
            font_names,
            font_size_names,
            font_sizes,
            theme_names,
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
            Message::Config(config) => {
                if config != self.config {
                    log::info!("update config");
                    //TODO: update syntax theme by clearing tabs, only if needed
                    self.config = config;
                    self.update_config();
                }
            }
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
            Message::DefaultFont(index) => {
                match self.font_names.get(index) {
                    Some(font_name) => {
                        let mut font_system = FONT_SYSTEM.lock().unwrap();
                        font_system.db_mut().set_monospace_family(font_name);
                        // This does a complete reset of shaping data!
                        let entities: Vec<_> = self.tab_model.iter().collect();
                        for entity in entities {
                            if let Some(tab) = self.tab_model.data_mut::<Tab>(entity) {
                                let mut editor = tab.editor.lock().unwrap();
                                for line in editor.buffer_mut().lines.iter_mut() {
                                    line.reset();
                                }
                            }
                        }
                    }
                    None => {
                        log::warn!("failed to find font with index {}", index);
                    }
                }
            }
            Message::DefaultFontSize(index) => match self.font_sizes.get(index) {
                Some(font_size) => {
                    self.config.font_size = *font_size;
                    self.save_config();
                }
                None => {
                    log::warn!("failed to find font with index {}", index);
                }
            },
            Message::Key(modifiers, key_code) => {
                for (key_bind, action) in self.config.keybinds.iter() {
                    if key_bind.matches(modifiers, key_code) {
                        return self.update(action.message());
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
                        Ok(_child) => {}
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
            Message::PasteValue(value) => match self.active_tab() {
                Some(tab) => {
                    let mut editor = tab.editor.lock().unwrap();
                    editor.insert_string(&value, None);
                }
                None => {}
            },
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
            Message::SyntaxTheme(index, dark) => match self.theme_names.get(index) {
                Some(theme_name) => {
                    if dark {
                        self.config.syntax_theme_dark = theme_name.to_string();
                    } else {
                        self.config.syntax_theme_light = theme_name.to_string();
                    }
                    self.save_config();
                }
                None => {
                    log::warn!("failed to find syntax theme with index {}", index);
                }
            },
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
                self.save_config();
            }
            Message::VimBindings(vim_bindings) => {
                self.config.vim_bindings = vim_bindings;
                self.save_config();
            }
        }

        Command::none()
    }

    fn context_drawer(&self) -> Option<Element<Message>> {
        if !self.core.window.show_context {
            return None;
        }

        Some(match self.context_page {
            ContextPage::DocumentStatistics => {
                //TODO: calculate in the background
                let mut character_count = 0;
                let mut character_count_no_spaces = 0;
                let line_count;
                match self.active_tab() {
                    Some(tab) => {
                        let editor = tab.editor.lock().unwrap();
                        let buffer = editor.buffer();

                        line_count = buffer.lines.len();
                        for line in buffer.lines.iter() {
                            //TODO: do graphemes?
                            for c in line.text().chars() {
                                character_count += 1;
                                if !c.is_whitespace() {
                                    character_count_no_spaces += 1;
                                }
                            }
                        }
                    }
                    None => {
                        return None;
                    }
                }

                widget::settings::view_column(vec![widget::settings::view_section("")
                    .add(widget::settings::item::builder(fl!("word-count")).control("TODO"))
                    .add(
                        widget::settings::item::builder(fl!("character-count"))
                            .control(widget::text(character_count.to_string())),
                    )
                    .add(
                        widget::settings::item::builder(fl!("character-count-no-spaces"))
                            .control(widget::text(character_count_no_spaces.to_string())),
                    )
                    .add(
                        widget::settings::item::builder(fl!("line-count"))
                            .control(widget::text(line_count.to_string())),
                    )
                    .into()])
                .into()
            }
            ContextPage::Settings => {
                let dark = cosmic::theme::is_dark();
                let current_theme_name = if dark {
                    &self.config.syntax_theme_dark
                } else {
                    &self.config.syntax_theme_light
                };
                let theme_selected = self
                    .theme_names
                    .iter()
                    .position(|theme_name| theme_name == current_theme_name);
                let font_selected = {
                    let font_system = FONT_SYSTEM.lock().unwrap();
                    let current_font_name = font_system.db().family_name(&Family::Monospace);
                    self.font_names
                        .iter()
                        .position(|font_name| font_name == current_font_name)
                };
                let font_size_selected = self
                    .font_sizes
                    .iter()
                    .position(|font_size| font_size == &self.config.font_size);
                widget::settings::view_column(vec![
                    widget::settings::view_section(fl!("appearance"))
                        .add(widget::settings::item::builder(fl!("theme")).control(
                            widget::dropdown(&self.theme_names, theme_selected, move |index| {
                                Message::SyntaxTheme(index, dark)
                            }),
                        ))
                        .add(
                            widget::settings::item::builder(fl!("default-font")).control(
                                widget::dropdown(&self.font_names, font_selected, |index| {
                                    Message::DefaultFont(index)
                                }),
                            ),
                        )
                        .add(
                            widget::settings::item::builder(fl!("default-font-size")).control(
                                widget::dropdown(
                                    &self.font_size_names,
                                    font_size_selected,
                                    |index| Message::DefaultFontSize(index),
                                ),
                            ),
                        )
                        .into(),
                    widget::settings::view_section(fl!("keyboard-shortcuts"))
                        .add(
                            widget::settings::item::builder(fl!("enable-vim-bindings"))
                                .toggler(self.config.vim_bindings, Message::VimBindings),
                        )
                        .into(),
                ])
                .into()
            }
        })
    }

    fn header_start(&self) -> Vec<Element<Message>> {
        vec![menu_bar(&self.config)]
    }

    fn view(&self) -> Element<Message> {
        let mut tab_column = widget::column::with_capacity(3).padding([0, 8]);

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
                tab_column =
                    tab_column.push(text_box(&tab.editor, self.config.metrics()).padding(8));
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
        subscription::Subscription::batch([
            subscription::events_with(|event, _status| match event {
                event::Event::Keyboard(keyboard::Event::KeyPressed {
                    modifiers,
                    key_code,
                }) => Some(Message::Key(modifiers, key_code)),
                _ => None,
            }),
            cosmic_config::config_subscription(0, Self::APP_ID.into(), CONFIG_VERSION).map(
                |(_, res)| match res {
                    Ok(config) => Message::Config(config),
                    Err((errs, config)) => {
                        log::warn!("errors loading config: {:#?}", errs);
                        Message::Config(config)
                    }
                },
            ),
        ])
    }
}
