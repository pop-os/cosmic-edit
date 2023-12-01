// SPDX-License-Identifier: GPL-3.0-only

use cosmic::{
    app::{message, Command, Core, Settings},
    cosmic_config::{self, CosmicConfigEntry},
    cosmic_theme, executor,
    font::Font,
    iced::{
        clipboard, event,
        futures::{self, SinkExt},
        keyboard, subscription,
        widget::{row, text},
        window, Alignment, Length, Point,
    },
    style, theme,
    widget::{self, button, icon, nav_bar, segmented_button, view_switcher},
    Application, ApplicationExt, Apply, Element,
};
use cosmic_text::{Cursor, Edit, Family, FontSystem, Selection, SwashCache, SyntaxSystem, ViMode};
use std::{
    any::TypeId,
    env, fs,
    path::{Path, PathBuf},
    process,
    sync::Mutex,
};
use tokio::time;

use config::{Action, AppTheme, Config, CONFIG_VERSION};
mod config;

use icon_cache::IconCache;
mod icon_cache;

use line_number::LineNumberCache;
mod line_number;

mod localize;

pub use self::mime_icon::{mime_icon, FALLBACK_MIME_ICON};
mod mime_icon;

use self::menu::menu_bar;
mod menu;

use self::project::ProjectNode;
mod project;

use self::search::ProjectSearchResult;
mod search;

use self::tab::Tab;
mod tab;

use self::text_box::text_box;
mod text_box;

//TODO: re-use iced FONT_SYSTEM
lazy_static::lazy_static! {
    static ref FONT_SYSTEM: Mutex<FontSystem> = Mutex::new(FontSystem::new());
    static ref ICON_CACHE: Mutex<IconCache> = Mutex::new(IconCache::new());
    static ref LINE_NUMBER_CACHE: Mutex<LineNumberCache> = Mutex::new(LineNumberCache::new());
    static ref SWASH_CACHE: Mutex<SwashCache> = Mutex::new(SwashCache::new());
    static ref SYNTAX_SYSTEM: SyntaxSystem = {
        let lazy_theme_set = two_face::theme::LazyThemeSet::from(two_face::theme::extra());
        SyntaxSystem {
            //TODO: store newlines in buffer
            syntax_set: two_face::syntax::extra_no_newlines(),
            theme_set: syntect::highlighting::ThemeSet::from(&lazy_theme_set),
        }
    };
}

pub fn icon_cache_get(name: &'static str, size: u16) -> icon::Icon {
    let mut icon_cache = ICON_CACHE.lock().unwrap();
    icon_cache.get(name, size)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(unix)]
    match fork::daemon(true, true) {
        Ok(fork::Fork::Child) => (),
        Ok(fork::Fork::Parent(_child_pid)) => process::exit(0),
        Err(err) => {
            eprintln!("failed to daemonize: {:?}", err);
            process::exit(1);
        }
    }

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    localize::localize();

    let (config_handler, config) = match cosmic_config::Config::new(App::APP_ID, CONFIG_VERSION) {
        Ok(config_handler) => {
            let config = match Config::get_entry(&config_handler) {
                Ok(ok) => ok,
                Err((errs, config)) => {
                    log::info!("errors loading config: {:?}", errs);
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

    let mut settings = Settings::default();
    settings = settings.theme(config.app_theme.theme());

    #[cfg(target_os = "redox")]
    {
        // Redox does not support resize if doing CSDs
        settings = settings.client_decorations(false);
    }

    //TODO: allow size limits on iced_winit
    //settings = settings.size_limits(Limits::NONE.min_width(400.0).min_height(200.0));

    let flags = Flags {
        config_handler,
        config,
    };
    cosmic::app::run::<App>(settings, flags)?;

    Ok(())
}

#[derive(Clone, Debug)]
pub struct Flags {
    config_handler: Option<cosmic_config::Config>,
    config: Config,
}

#[derive(Debug)]
pub struct WatcherWrapper {
    watcher_opt: Option<notify::RecommendedWatcher>,
}

impl Clone for WatcherWrapper {
    fn clone(&self) -> Self {
        Self { watcher_opt: None }
    }
}

impl PartialEq for WatcherWrapper {
    fn eq(&self, _other: &Self) -> bool {
        false
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    AppTheme(AppTheme),
    Config(Config),
    CloseFile,
    CloseProject,
    Copy,
    Cut,
    DefaultFont(usize),
    DefaultFontSize(usize),
    Key(keyboard::Modifiers, keyboard::KeyCode),
    NewFile,
    NewWindow,
    NotifyEvent(notify::Event),
    NotifyWatcher(WatcherWrapper),
    OpenFileDialog,
    OpenFile(PathBuf),
    OpenProjectDialog,
    OpenProject(PathBuf),
    OpenSearchResult(usize, usize),
    Paste,
    PasteValue(String),
    ProjectSearchResult(ProjectSearchResult),
    ProjectSearchSubmit,
    ProjectSearchValue(String),
    Quit,
    Redo,
    Save,
    SelectAll,
    SystemThemeModeChange(cosmic_theme::ThemeMode),
    SyntaxTheme(usize, bool),
    TabActivate(segmented_button::Entity),
    TabChanged(segmented_button::Entity),
    TabClose(segmented_button::Entity),
    TabContextAction(segmented_button::Entity, Action),
    TabContextMenu(segmented_button::Entity, Option<Point>),
    TabSetCursor(segmented_button::Entity, Cursor),
    TabWidth(u16),
    Todo,
    ToggleAutoIndent,
    ToggleContextPage(ContextPage),
    ToggleLineNumbers,
    ToggleWordWrap,
    Undo,
    VimBindings(bool),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextPage {
    DocumentStatistics,
    //TODO: Move search to pop-up
    ProjectSearch,
    Settings,
}

impl ContextPage {
    fn title(&self) -> String {
        match self {
            Self::DocumentStatistics => fl!("document-statistics"),
            Self::ProjectSearch => fl!("project-search"),
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
    app_themes: Vec<String>,
    font_names: Vec<String>,
    font_size_names: Vec<String>,
    font_sizes: Vec<u16>,
    theme_names: Vec<String>,
    context_page: ContextPage,
    project_search_id: widget::Id,
    project_search_value: String,
    project_search_result: Option<ProjectSearchResult>,
    watcher_opt: Option<notify::RecommendedWatcher>,
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
                .icon(node.icon(16))
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
            .icon(node.icon(16))
            .text(node.name().to_string())
            .data(node)
            .id();

        let position = self.nav_model.position(id).unwrap_or(0);

        self.open_folder(&path, position + 1, 1);
    }

    pub fn open_tab(&mut self, path_opt: Option<PathBuf>) -> Option<segmented_button::Entity> {
        let tab = match path_opt {
            Some(path) => {
                let canonical = match fs::canonicalize(&path) {
                    Ok(ok) => ok,
                    Err(err) => {
                        log::error!("failed to canonicalize {:?}: {}", path, err);
                        return None;
                    }
                };

                //TODO: allow files to be open multiple times
                let mut activate_opt = None;
                for entity in self.tab_model.iter() {
                    match self.tab_model.data::<Tab>(entity) {
                        Some(tab) => {
                            if tab.path_opt.as_ref() == Some(&canonical) {
                                activate_opt = Some(entity);
                                break;
                            }
                        }
                        None => {}
                    }
                }
                if let Some(entity) = activate_opt {
                    self.tab_model.activate(entity);
                    return Some(entity);
                }

                let mut tab = Tab::new(&self.config);
                tab.open(canonical);
                tab.watch(&mut self.watcher_opt);
                tab
            }
            None => Tab::new(&self.config),
        };

        Some(
            self.tab_model
                .insert()
                .text(tab.title())
                .icon(tab.icon(16))
                .data::<Tab>(tab)
                .closable()
                .activate()
                .id(),
        )
    }

    fn update_config(&mut self) -> Command<Message> {
        //TODO: provide iterator over data
        let entities: Vec<_> = self.tab_model.iter().collect();
        for entity in entities {
            if let Some(tab) = self.tab_model.data_mut::<Tab>(entity) {
                tab.set_config(&self.config);
            }
        }
        cosmic::app::command::set_theme(self.config.app_theme.theme())
    }

    fn save_config(&mut self) -> Command<Message> {
        match self.config_handler {
            Some(ref config_handler) => match self.config.write_entry(&config_handler) {
                Ok(()) => {}
                Err(err) => {
                    log::error!("failed to save config: {}", err);
                }
            },
            None => {}
        }
        self.update_config()
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
                            let _ = self.on_nav_select(id);
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
                // Force redraw on tab switches
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
impl Application for App {
    /// Default async executor to use with the app.
    type Executor = executor::Default;

    /// Argument received [`cosmic::Application::new`].
    type Flags = Flags;

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
    fn init(core: Core, flags: Self::Flags) -> (Self, Command<Self::Message>) {
        // Update font name from config
        {
            let mut font_system = FONT_SYSTEM.lock().unwrap();
            font_system
                .db_mut()
                .set_monospace_family(&flags.config.font_name);
        }

        let app_themes = vec![fl!("match-desktop"), fl!("dark"), fl!("light")];

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
            config_handler: flags.config_handler,
            config: flags.config,
            app_themes,
            font_names,
            font_size_names,
            font_sizes,
            theme_names,
            context_page: ContextPage::Settings,
            project_search_id: widget::Id::unique(),
            project_search_value: String::new(),
            project_search_result: None,
            watcher_opt: None,
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
                .icon(icon_cache_get("folder-open-symbolic", 16))
                .text(fl!("open-project"));
        }

        // Open an empty file if no arguments provided
        if app.tab_model.iter().next().is_none() {
            app.open_tab(None);
        }

        //TODO: try update_config here? It breaks loading system theme by default
        let command = app.update_tab();
        (app, command)
    }

    // The default nav_bar widget needs to be condensed for cosmic-edit
    fn nav_bar(&self) -> Option<Element<message::Message<Self::Message>>> {
        if !self.core().nav_bar_active() {
            return None;
        }

        let nav_model = self.nav_model()?;

        let cosmic_theme::Spacing {
            space_none,
            space_s,
            space_xxxs,
            ..
        } = self.core().system_theme().cosmic().spacing;

        let mut nav = segmented_button::vertical(nav_model)
            .button_height(space_xxxs + 20 /* line height */ + space_xxxs)
            .button_padding([space_s, space_xxxs, space_s, space_xxxs])
            .button_spacing(space_xxxs)
            .on_activate(|entity| message::cosmic(cosmic::app::cosmic::Message::NavBar(entity)))
            .spacing(space_none)
            .style(theme::SegmentedButton::ViewSwitcher)
            .apply(widget::container)
            .padding(space_s)
            .width(Length::Fill);

        if !self.core().is_condensed() {
            nav = nav.max_width(300);
        }

        Some(
            nav.apply(widget::scrollable)
                .apply(widget::container)
                .height(Length::Fill)
                .style(theme::Container::custom(nav_bar::nav_bar_style))
                .into(),
        )
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
                self.nav_model.icon_set(id, node.icon(16));

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
            Message::AppTheme(app_theme) => {
                self.config.app_theme = app_theme;
                return self.save_config();
            }
            Message::Config(config) => {
                if config != self.config {
                    log::info!("update config");
                    //TODO: update syntax theme by clearing tabs, only if needed
                    self.config = config;
                    return self.update_config();
                }
            }
            Message::CloseFile => {
                return self.update(Message::TabClose(self.tab_model.active()));
            }
            Message::CloseProject => {
                log::info!("TODO");
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
                        if font_name != &self.config.font_name {
                            // Update font name from config
                            {
                                let mut font_system = FONT_SYSTEM.lock().unwrap();
                                font_system.db_mut().set_monospace_family(font_name);
                            }

                            // Reset line number cache
                            {
                                let mut line_number_cache = LINE_NUMBER_CACHE.lock().unwrap();
                                line_number_cache.clear();
                            }

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

                            self.config.font_name = font_name.to_string();
                            return self.save_config();
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
                    return self.save_config();
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
            Message::NotifyEvent(event) => {
                let mut needs_reload = Vec::new();
                for entity in self.tab_model.iter() {
                    match self.tab_model.data::<Tab>(entity) {
                        Some(tab) => {
                            if let Some(path) = &tab.path_opt {
                                if event.paths.contains(&path) {
                                    if tab.changed() {
                                        log::warn!(
                                            "file changed externally before being saved: {:?}",
                                            path
                                        );
                                    } else {
                                        needs_reload.push(entity);
                                    }
                                }
                            }
                        }
                        None => {}
                    }
                }

                for entity in needs_reload {
                    match self.tab_model.data_mut::<Tab>(entity) {
                        Some(tab) => {
                            tab.reload();
                        }
                        None => {
                            log::warn!("failed to find tab {:?} that needs reload", entity);
                        }
                    }
                }
            }
            Message::NotifyWatcher(mut watcher_wrapper) => match watcher_wrapper.watcher_opt.take()
            {
                Some(watcher) => {
                    self.watcher_opt = Some(watcher);

                    for entity in self.tab_model.iter() {
                        match self.tab_model.data::<Tab>(entity) {
                            Some(tab) => {
                                tab.watch(&mut self.watcher_opt);
                            }
                            None => {}
                        }
                    }
                }
                None => {
                    log::warn!("message did not contain notify watcher");
                }
            },
            Message::OpenFileDialog => {
                #[cfg(feature = "rfd")]
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
                #[cfg(feature = "rfd")]
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
            Message::OpenSearchResult(file_i, line_i) => {
                let path_cursor_opt = match &self.project_search_result {
                    Some(project_search_result) => match project_search_result.files.get(file_i) {
                        Some(file_search_result) => match file_search_result.lines.get(line_i) {
                            Some(line_search_result) => Some((
                                file_search_result.path.to_path_buf(),
                                Cursor::new(
                                    line_search_result.number.saturating_sub(1),
                                    line_search_result.first.start(),
                                ),
                            )),
                            None => {
                                log::warn!("failed to find search result {}, {}", file_i, line_i);
                                None
                            }
                        },
                        None => {
                            log::warn!("failed to find search result {}", file_i);
                            None
                        }
                    },
                    None => None,
                };

                if let Some((path, cursor)) = path_cursor_opt {
                    if let Some(entity) = self.open_tab(Some(path)) {
                        return Command::batch([
                            //TODO: why must this be done in a command?
                            Command::perform(
                                async move { message::app(Message::TabSetCursor(entity, cursor)) },
                                |x| x,
                            ),
                            self.update_tab(),
                        ]);
                    }
                }
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
            Message::ProjectSearchResult(project_search_result) => {
                self.project_search_result = Some(project_search_result);

                // Ensure input remains focused
                return widget::text_input::focus(self.project_search_id.clone());
            }
            Message::ProjectSearchSubmit => {
                //TODO: Figure out length requirements?
                if !self.project_search_value.is_empty() {
                    //TODO: cache projects outside of nav model?
                    let mut project_paths = Vec::new();
                    for id in self.nav_model.iter() {
                        match self.nav_model.data(id) {
                            Some(ProjectNode::Folder { path, root, .. }) => {
                                if *root {
                                    project_paths.push(path.clone())
                                }
                            }
                            _ => {}
                        }
                    }

                    let project_search_value = self.project_search_value.clone();
                    let mut project_search_result = ProjectSearchResult {
                        value: project_search_value.clone(),
                        in_progress: true,
                        files: Vec::new(),
                    };
                    self.project_search_result = Some(project_search_result.clone());
                    return Command::perform(
                        async move {
                            let task_res = tokio::task::spawn_blocking(move || {
                                project_search_result.search_projects(project_paths);
                                message::app(Message::ProjectSearchResult(project_search_result))
                            })
                            .await;
                            match task_res {
                                Ok(message) => message,
                                Err(err) => {
                                    log::error!("failed to run search task: {}", err);
                                    message::none()
                                }
                            }
                        },
                        |x| x,
                    );
                }
            }
            Message::ProjectSearchValue(value) => {
                self.project_search_value = value;
            }
            Message::Quit => {
                //TODO: prompt for save?
                return window::close();
            }
            Message::Redo => match self.active_tab() {
                Some(tab) => {
                    let mut editor = tab.editor.lock().unwrap();
                    editor.redo();
                }
                None => {}
            },
            Message::Save => {
                let mut title_opt = None;

                match self.active_tab_mut() {
                    Some(tab) => {
                        #[cfg(feature = "rfd")]
                        if tab.path_opt.is_none() {
                            //TODO: use async file dialog
                            tab.path_opt = rfd::FileDialog::new().save_file();
                        }
                        title_opt = Some(tab.title());
                        tab.save();
                    }
                    None => {
                        //TODO: disable save button?
                        log::warn!("TODO: NO TAB OPEN");
                    }
                }

                if let Some(title) = title_opt {
                    self.tab_model.text_set(self.tab_model.active(), title);
                }
            }
            Message::SelectAll => {
                match self.active_tab_mut() {
                    Some(tab) => {
                        let mut editor = tab.editor.lock().unwrap();

                        // Set cursor to lowest possible value
                        editor.set_cursor(Cursor::new(0, 0));

                        // Set selection end to highest possible value
                        let buffer = editor.buffer();
                        let last_line = buffer.lines.len().saturating_sub(1);
                        let last_index = buffer.lines[last_line].text().len();
                        editor.set_selection(Selection::Normal(Cursor::new(last_line, last_index)));
                    }
                    None => {}
                }
            }
            Message::SystemThemeModeChange(_theme_mode) => {
                return self.update_config();
            }
            Message::SyntaxTheme(index, dark) => match self.theme_names.get(index) {
                Some(theme_name) => {
                    if dark {
                        self.config.syntax_theme_dark = theme_name.to_string();
                    } else {
                        self.config.syntax_theme_light = theme_name.to_string();
                    }
                    return self.save_config();
                }
                None => {
                    log::warn!("failed to find syntax theme with index {}", index);
                }
            },
            Message::TabActivate(entity) => {
                self.tab_model.activate(entity);
                return self.update_tab();
            }
            Message::TabChanged(entity) => match self.tab_model.data::<Tab>(entity) {
                Some(tab) => {
                    let mut title = tab.title();
                    //TODO: better way of adding change indicator
                    title.push_str(" \u{2022}");
                    self.tab_model.text_set(entity, title);
                }
                None => {}
            },
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
            Message::TabContextAction(entity, action) => {
                match self.tab_model.data_mut::<Tab>(entity) {
                    Some(tab) => {
                        // Close context menu
                        tab.context_menu = None;
                        // Run action's message
                        return self.update(action.message());
                    }
                    None => {}
                }
            }
            Message::TabContextMenu(entity, position_opt) => {
                match self.tab_model.data_mut::<Tab>(entity) {
                    Some(tab) => {
                        // Update context menu
                        tab.context_menu = position_opt;
                    }
                    None => {}
                }
            }
            Message::TabSetCursor(entity, cursor) => match self.tab_model.data::<Tab>(entity) {
                Some(tab) => {
                    let mut editor = tab.editor.lock().unwrap();
                    editor.set_cursor(cursor);
                }
                None => {}
            },
            Message::TabWidth(tab_width) => {
                self.config.tab_width = tab_width;
                return self.save_config();
            }
            Message::Todo => {
                log::warn!("TODO");
            }
            Message::ToggleAutoIndent => {
                self.config.auto_indent = !self.config.auto_indent;
                return self.save_config();
            }
            Message::ToggleContextPage(context_page) => {
                if self.context_page == context_page {
                    self.core.window.show_context = !self.core.window.show_context;
                } else {
                    self.context_page = context_page;
                    self.core.window.show_context = true;
                }
                self.set_context_title(context_page.title());

                // Ensure focus of correct input
                if self.core.window.show_context {
                    match self.context_page {
                        ContextPage::ProjectSearch => {
                            return widget::text_input::focus(self.project_search_id.clone());
                        }
                        _ => {}
                    }
                }
            }
            Message::ToggleLineNumbers => {
                self.config.line_numbers = !self.config.line_numbers;

                // This forces a redraw of all buffers
                let entities: Vec<_> = self.tab_model.iter().collect();
                for entity in entities {
                    if let Some(tab) = self.tab_model.data_mut::<Tab>(entity) {
                        let mut editor = tab.editor.lock().unwrap();
                        editor.buffer_mut().set_redraw(true);
                    }
                }

                return self.save_config();
            }
            Message::ToggleWordWrap => {
                self.config.word_wrap = !self.config.word_wrap;
                return self.save_config();
            }
            Message::Undo => match self.active_tab() {
                Some(tab) => {
                    let mut editor = tab.editor.lock().unwrap();
                    editor.undo();
                }
                None => {}
            },
            Message::VimBindings(vim_bindings) => {
                self.config.vim_bindings = vim_bindings;
                return self.save_config();
            }
        }

        Command::none()
    }

    fn context_drawer(&self) -> Option<Element<Message>> {
        if !self.core.window.show_context {
            return None;
        }

        let cosmic_theme::Spacing {
            space_none,
            space_s,
            space_xs,
            space_xxs,
            ..
        } = self.core().system_theme().cosmic().spacing;

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
            ContextPage::ProjectSearch => {
                let search_input = widget::text_input::search_input(
                    &fl!("project-search"),
                    &self.project_search_value,
                )
                .id(self.project_search_id.clone());

                let items = match &self.project_search_result {
                    Some(project_search_result) => {
                        let mut items =
                            Vec::with_capacity(project_search_result.files.len().saturating_add(1));

                        if project_search_result.in_progress {
                            items.push(search_input.into());
                        } else {
                            items.push(
                                search_input
                                    .on_input(Message::ProjectSearchValue)
                                    .on_submit(Message::ProjectSearchSubmit)
                                    .into(),
                            );
                        }

                        for (file_i, file_search_result) in
                            project_search_result.files.iter().enumerate()
                        {
                            let mut column =
                                widget::column::with_capacity(file_search_result.lines.len());
                            let mut line_number_width = 1;
                            if let Some(line_search_result) = file_search_result.lines.last() {
                                let mut number = line_search_result.number;
                                while number >= 10 {
                                    number /= 10;
                                    line_number_width += 1;
                                }
                            }
                            for (line_i, line_search_result) in
                                file_search_result.lines.iter().enumerate()
                            {
                                column = column.push(
                                    widget::button(
                                        widget::row::with_children(vec![
                                            widget::text(format!(
                                                "{:width$}",
                                                line_search_result.number,
                                                width = line_number_width,
                                            ))
                                            .font(Font::MONOSPACE)
                                            .into(),
                                            widget::text(format!("{}", line_search_result.text))
                                                .font(Font::MONOSPACE)
                                                .into(),
                                        ])
                                        .spacing(space_xs),
                                    )
                                    .on_press(Message::OpenSearchResult(file_i, line_i))
                                    .width(Length::Fill)
                                    .style(theme::Button::AppletMenu),
                                );
                            }

                            items.push(
                                widget::settings::view_section(format!(
                                    "{}",
                                    file_search_result.path.display(),
                                ))
                                .add(column)
                                .into(),
                            );
                        }

                        items
                    }
                    None => {
                        vec![search_input
                            .on_input(Message::ProjectSearchValue)
                            .on_submit(Message::ProjectSearchSubmit)
                            .into()]
                    }
                };

                widget::column::with_children(items)
                    .spacing(space_s)
                    .padding([space_xxs, space_none])
                    .into()
            }
            ContextPage::Settings => {
                let app_theme_selected = match self.config.app_theme {
                    AppTheme::Dark => 1,
                    AppTheme::Light => 2,
                    AppTheme::System => 0,
                };
                let dark_selected = self
                    .theme_names
                    .iter()
                    .position(|theme_name| theme_name == &self.config.syntax_theme_dark);
                let light_selected = self
                    .theme_names
                    .iter()
                    .position(|theme_name| theme_name == &self.config.syntax_theme_light);
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
                            widget::dropdown(
                                &self.app_themes,
                                Some(app_theme_selected),
                                move |index| {
                                    Message::AppTheme(match index {
                                        1 => AppTheme::Dark,
                                        2 => AppTheme::Light,
                                        _ => AppTheme::System,
                                    })
                                },
                            ),
                        ))
                        .add(widget::settings::item::builder(fl!("syntax-dark")).control(
                            widget::dropdown(&self.theme_names, dark_selected, move |index| {
                                Message::SyntaxTheme(index, true)
                            }),
                        ))
                        .add(
                            widget::settings::item::builder(fl!("syntax-light")).control(
                                widget::dropdown(&self.theme_names, light_selected, move |index| {
                                    Message::SyntaxTheme(index, false)
                                }),
                            ),
                        )
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
        let cosmic_theme::Spacing {
            space_none,
            space_xxs,
            ..
        } = self.core().system_theme().cosmic().spacing;

        let mut tab_column = widget::column::with_capacity(3).padding([space_none, space_xxs]);

        tab_column = tab_column.push(
            row![
                view_switcher::horizontal(&self.tab_model)
                    .button_height(32)
                    .button_spacing(space_xxs)
                    .close_icon(icon_cache_get("window-close-symbolic", 16))
                    .on_activate(Message::TabActivate)
                    .on_close(Message::TabClose)
                    .width(Length::Shrink),
                button(icon_cache_get("list-add-symbolic", 16))
                    .on_press(Message::NewFile)
                    .padding(space_xxs)
                    .style(style::Button::Icon)
            ]
            .align_items(Alignment::Center),
        );

        let tab_id = self.tab_model.active();
        match self.tab_model.data::<Tab>(tab_id) {
            Some(tab) => {
                let status = {
                    let editor = tab.editor.lock().unwrap();
                    let parser = editor.parser();
                    match &parser.mode {
                        ViMode::Normal => {
                            format!("{}", parser.cmd)
                        }
                        ViMode::Insert => {
                            format!("-- INSERT --")
                        }
                        ViMode::Extra(extra) => {
                            format!("{}{}", parser.cmd, extra)
                        }
                        ViMode::Replace => {
                            format!("-- REPLACE --")
                        }
                        ViMode::Visual => {
                            format!("-- VISUAL -- {}", parser.cmd)
                        }
                        ViMode::VisualLine => {
                            format!("-- VISUAL LINE -- {}", parser.cmd)
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
                    }
                };
                let mut text_box = text_box(&tab.editor, self.config.metrics())
                    .on_changed(Message::TabChanged(tab_id))
                    .on_context_menu(move |position_opt| {
                        Message::TabContextMenu(tab_id, position_opt)
                    });
                if self.config.line_numbers {
                    text_box = text_box.line_numbers();
                }
                let tab_element: Element<'_, Message> = match tab.context_menu {
                    Some(position) => widget::popover(
                        text_box.context_menu(position),
                        menu::context_menu(&self.config, tab_id),
                    )
                    .position(position)
                    .into(),
                    None => text_box.into(),
                };
                tab_column = tab_column.push(tab_element);
                tab_column = tab_column.push(text(status).font(Font::MONOSPACE));
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
        struct WatcherSubscription;
        struct ConfigSubscription;
        struct ThemeSubscription;

        subscription::Subscription::batch([
            subscription::events_with(|event, _status| match event {
                event::Event::Keyboard(keyboard::Event::KeyPressed {
                    modifiers,
                    key_code,
                }) => Some(Message::Key(modifiers, key_code)),
                _ => None,
            }),
            subscription::channel(
                TypeId::of::<WatcherSubscription>(),
                100,
                |mut output| async move {
                    let watcher_res = {
                        let mut output = output.clone();
                        notify::recommended_watcher(
                            move |event_res: Result<notify::Event, notify::Error>| match event_res {
                                Ok(event) => {
                                    match &event.kind {
                                        notify::EventKind::Access(_)
                                        | notify::EventKind::Modify(
                                            notify::event::ModifyKind::Metadata(_),
                                        ) => {
                                            // Data not mutated
                                            return;
                                        }
                                        _ => {}
                                    }

                                    match futures::executor::block_on(async {
                                        output.send(Message::NotifyEvent(event)).await
                                    }) {
                                        Ok(()) => {}
                                        Err(err) => {
                                            log::warn!("failed to send notify event: {:?}", err);
                                        }
                                    }
                                }
                                Err(err) => {
                                    log::warn!("failed to watch files: {:?}", err);
                                }
                            },
                        )
                    };

                    match watcher_res {
                        Ok(watcher) => {
                            match output
                                .send(Message::NotifyWatcher(WatcherWrapper {
                                    watcher_opt: Some(watcher),
                                }))
                                .await
                            {
                                Ok(()) => {}
                                Err(err) => {
                                    log::warn!("failed to send notify watcher: {:?}", err);
                                }
                            }
                        }
                        Err(err) => {
                            log::warn!("failed to create file watcher: {:?}", err);
                        }
                    }

                    //TODO: how to properly kill this task?
                    loop {
                        time::sleep(time::Duration::new(1, 0)).await;
                    }
                },
            ),
            cosmic_config::config_subscription(
                TypeId::of::<ConfigSubscription>(),
                Self::APP_ID.into(),
                CONFIG_VERSION,
            )
            .map(|(_, res)| match res {
                Ok(config) => Message::Config(config),
                Err((errs, config)) => {
                    log::info!("errors loading config: {:?}", errs);
                    Message::Config(config)
                }
            }),
            cosmic_config::config_subscription::<_, cosmic_theme::ThemeMode>(
                TypeId::of::<ThemeSubscription>(),
                cosmic_theme::THEME_MODE_ID.into(),
                cosmic_theme::ThemeMode::version(),
            )
            .map(|(_, u)| match u {
                Ok(t) => Message::SystemThemeModeChange(t),
                Err((errs, t)) => {
                    log::info!("errors loading theme mode: {:?}", errs);
                    Message::SystemThemeModeChange(t)
                }
            }),
        ])
    }
}
