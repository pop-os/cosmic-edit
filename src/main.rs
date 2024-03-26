// SPDX-License-Identifier: GPL-3.0-only

use cosmic::widget::menu::action::MenuAction;
use cosmic::widget::menu::key_bind::KeyBind;
use cosmic::widget::segmented_button::Entity;
use cosmic::{
    app::{message, Command, Core, Settings},
    cosmic_config::{self, CosmicConfigEntry},
    cosmic_theme, executor,
    font::Font,
    iced::{
        advanced::graphics::text::font_system,
        clipboard, event,
        futures::{self, SinkExt},
        keyboard::{self, Modifiers},
        subscription,
        widget::text,
        window, Alignment, Background, Color, Length, Limits, Point,
    },
    prelude::CollectionWidget,
    style, theme,
    widget::{self, button, icon, nav_bar, segmented_button},
    Application, ApplicationExt, Apply, Element,
};
use cosmic_files::{
    dialog::{Dialog, DialogKind, DialogMessage, DialogResult},
    mime_icon::{mime_for_path, mime_icon},
};
use cosmic_text::{Cursor, Edit, Family, Selection, SwashCache, SyntaxSystem, ViMode};
use serde::{Deserialize, Serialize};
use std::{
    any::TypeId,
    collections::HashMap,
    env, fs, io,
    path::{Path, PathBuf},
    process,
    sync::{Mutex, OnceLock},
};
use tokio::time;

use config::{AppTheme, Config, ConfigState, CONFIG_VERSION};
mod config;

use git::{GitDiff, GitDiffLine, GitRepository, GitStatus, GitStatusKind};
mod git;

use icon_cache::IconCache;
mod icon_cache;

use key_bind::key_binds;
mod key_bind;

use line_number::LineNumberCache;
mod line_number;

mod localize;

use self::menu::menu_bar;
mod menu;

use self::project::ProjectNode;
mod project;

use self::search::ProjectSearchResult;
mod search;

use self::tab::{EditorTab, GitDiffTab, Tab};
mod tab;

use self::text_box::text_box;
mod text_box;

static ICON_CACHE: OnceLock<Mutex<IconCache>> = OnceLock::new();
static LINE_NUMBER_CACHE: OnceLock<Mutex<LineNumberCache>> = OnceLock::new();
static SWASH_CACHE: OnceLock<Mutex<SwashCache>> = OnceLock::new();
static SYNTAX_SYSTEM: OnceLock<SyntaxSystem> = OnceLock::new();

pub fn icon_cache_get(name: &'static str, size: u16) -> icon::Icon {
    let mut icon_cache = ICON_CACHE.get().unwrap().lock().unwrap();
    icon_cache.get(name, size)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    ICON_CACHE.get_or_init(|| Mutex::new(IconCache::new()));
    LINE_NUMBER_CACHE.get_or_init(|| Mutex::new(LineNumberCache::new()));
    SWASH_CACHE.get_or_init(|| Mutex::new(SwashCache::new()));
    SYNTAX_SYSTEM.get_or_init(|| {
        let lazy_theme_set = two_face::theme::LazyThemeSet::from(two_face::theme::extra());
        let mut theme_set = syntect::highlighting::ThemeSet::from(&lazy_theme_set);
        // Hardcoded COSMIC themes
        for (theme_name, theme_data) in &[
            ("COSMIC Dark", cosmic_syntax_theme::COSMIC_DARK_TM_THEME),
            ("COSMIC Light", cosmic_syntax_theme::COSMIC_LIGHT_TM_THEME),
        ] {
            let mut cursor = io::Cursor::new(theme_data);
            match syntect::highlighting::ThemeSet::load_from_reader(&mut cursor) {
                Ok(mut theme) => {
                    // Use libcosmic theme for background and gutter
                    theme.settings.background = Some(syntect::highlighting::Color {
                        r: 0,
                        g: 0,
                        b: 0,
                        a: 0,
                    });
                    theme.settings.gutter = Some(syntect::highlighting::Color {
                        r: 0,
                        g: 0,
                        b: 0,
                        a: 0,
                    });
                    theme_set.themes.insert(theme_name.to_string(), theme);
                }
                Err(err) => {
                    eprintln!("failed to load {:?} syntax theme: {}", theme_name, err);
                }
            }
        }
        SyntaxSystem {
            //TODO: store newlines in buffer
            syntax_set: two_face::syntax::extra_no_newlines(),
            theme_set,
        }
    });

    #[cfg(all(unix, not(target_os = "redox")))]
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
            let config = Config::get_entry(&config_handler).unwrap_or_else(|(errs, config)| {
                log::info!("errors loading config: {:?}", errs);
                config
            });
            (Some(config_handler), config)
        }
        Err(err) => {
            log::error!("failed to create config handler: {}", err);
            (None, Config::default())
        }
    };

    let (config_state_handler, config_state) =
        match cosmic_config::Config::new_state(App::APP_ID, CONFIG_VERSION) {
            Ok(config_state_handler) => {
                let config_state = ConfigState::get_entry(&config_state_handler).unwrap_or_else(
                    |(errs, config_state)| {
                        log::info!("errors loading config_state: {:?}", errs);
                        config_state
                    },
                );
                (Some(config_state_handler), config_state)
            }
            Err(err) => {
                log::error!("failed to create config_state handler: {}", err);
                (None, ConfigState::default())
            }
        };

    let mut settings = Settings::default();
    settings = settings.theme(config.app_theme.theme());
    settings = settings.size_limits(Limits::NONE.min_width(360.0).min_height(180.0));

    let flags = Flags {
        config_handler,
        config,
        config_state_handler,
        config_state,
    };
    cosmic::app::run::<App>(settings, flags)?;

    Ok(())
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Action {
    Todo,
    About,
    CloseFile,
    CloseProject(usize),
    Copy,
    Cut,
    Find,
    FindAndReplace,
    NewFile,
    NewWindow,
    OpenFileDialog,
    OpenProjectDialog,
    OpenRecentFile(usize),
    OpenRecentProject(usize),
    Paste,
    Quit,
    Redo,
    Save,
    SaveAsDialog,
    SelectAll,
    TabActivate0,
    TabActivate1,
    TabActivate2,
    TabActivate3,
    TabActivate4,
    TabActivate5,
    TabActivate6,
    TabActivate7,
    TabActivate8,
    TabNext,
    TabPrev,
    TabWidth(u16),
    ToggleAutoIndent,
    ToggleDocumentStatistics,
    ToggleGitManagement,
    ToggleHighlightCurrentLine,
    ToggleLineNumbers,
    ToggleProjectSearch,
    ToggleSettingsPage,
    ToggleWordWrap,
    Undo,
}

impl MenuAction for Action {
    type Message = Message;
    fn message(&self, _entity: Option<Entity>) -> Message {
        match self {
            Self::Todo => Message::Todo,
            Self::About => Message::ToggleContextPage(ContextPage::About),
            Self::CloseFile => Message::CloseFile,
            Self::CloseProject(project_i) => Message::CloseProject(*project_i),
            Self::Copy => Message::Copy,
            Self::Cut => Message::Cut,
            Self::Find => Message::Find(Some(false)),
            Self::FindAndReplace => Message::Find(Some(true)),
            Self::NewFile => Message::NewFile,
            Self::NewWindow => Message::NewWindow,
            Self::OpenFileDialog => Message::OpenFileDialog,
            Self::OpenProjectDialog => Message::OpenProjectDialog,
            Self::OpenRecentFile(index) => Message::OpenRecentFile(*index),
            Self::OpenRecentProject(index) => Message::OpenRecentProject(*index),
            Self::Paste => Message::Paste,
            Self::Quit => Message::Quit,
            Self::Redo => Message::Redo,
            Self::Save => Message::Save,
            Self::SaveAsDialog => Message::SaveAsDialog,
            Self::SelectAll => Message::SelectAll,
            Self::TabActivate0 => Message::TabActivateJump(0),
            Self::TabActivate1 => Message::TabActivateJump(1),
            Self::TabActivate2 => Message::TabActivateJump(2),
            Self::TabActivate3 => Message::TabActivateJump(3),
            Self::TabActivate4 => Message::TabActivateJump(4),
            Self::TabActivate5 => Message::TabActivateJump(5),
            Self::TabActivate6 => Message::TabActivateJump(6),
            Self::TabActivate7 => Message::TabActivateJump(7),
            Self::TabActivate8 => Message::TabActivateJump(8),
            Self::TabNext => Message::TabNext,
            Self::TabPrev => Message::TabPrev,
            Self::TabWidth(tab_width) => Message::TabWidth(*tab_width),
            Self::ToggleAutoIndent => Message::ToggleAutoIndent,
            Self::ToggleDocumentStatistics => {
                Message::ToggleContextPage(ContextPage::DocumentStatistics)
            }
            Self::ToggleGitManagement => Message::ToggleContextPage(ContextPage::GitManagement),
            Self::ToggleHighlightCurrentLine => Message::ToggleHighlightCurrentLine,
            Self::ToggleLineNumbers => Message::ToggleLineNumbers,
            Self::ToggleProjectSearch => Message::ToggleContextPage(ContextPage::ProjectSearch),
            Self::ToggleSettingsPage => Message::ToggleContextPage(ContextPage::Settings),
            Self::ToggleWordWrap => Message::ToggleWordWrap,
            Self::Undo => Message::Undo,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Flags {
    config_handler: Option<cosmic_config::Config>,
    config: Config,
    config_state_handler: Option<cosmic_config::Config>,
    config_state: ConfigState,
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
#[derive(Clone, Debug)]
pub enum Message {
    AppTheme(AppTheme),
    Config(Config),
    ConfigState(ConfigState),
    CloseFile,
    CloseProject(usize),
    Copy,
    Cut,
    DefaultFont(usize),
    DefaultFontSize(usize),
    DialogMessage(DialogMessage),
    Find(Option<bool>),
    FindCaseSensitive(bool),
    FindNext,
    FindPrevious,
    FindReplace,
    FindReplaceAll,
    FindReplaceValueChanged(String),
    FindSearchValueChanged(String),
    FindUseRegex(bool),
    GitProjectStatus(Vec<(String, PathBuf, Vec<GitStatus>)>),
    Key(Modifiers, keyboard::Key),
    LaunchUrl(String),
    Modifiers(Modifiers),
    NewFile,
    NewWindow,
    NotifyEvent(notify::Event),
    NotifyWatcher(WatcherWrapper),
    OpenFile(PathBuf),
    OpenFileDialog,
    OpenFileResult(DialogResult),
    OpenGitDiff(PathBuf, GitDiff),
    OpenProjectDialog,
    OpenProjectResult(DialogResult),
    OpenRecentFile(usize),
    OpenRecentProject(usize),
    OpenSearchResult(usize, usize),
    Paste,
    PasteValue(String),
    PrepareGitDiff(PathBuf, PathBuf, bool),
    ProjectSearchResult(ProjectSearchResult),
    ProjectSearchSubmit,
    ProjectSearchValue(String),
    Quit,
    Redo,
    Save,
    SaveAsDialog,
    SaveAsResult(segmented_button::Entity, DialogResult),
    SelectAll,
    SystemThemeModeChange(cosmic_theme::ThemeMode),
    SyntaxTheme(usize, bool),
    TabActivate(segmented_button::Entity),
    TabActivateJump(usize),
    TabChanged(segmented_button::Entity),
    TabClose(segmented_button::Entity),
    TabCloseForce(segmented_button::Entity),
    TabContextAction(segmented_button::Entity, Action),
    TabContextMenu(segmented_button::Entity, Option<Point>),
    TabNext,
    TabPrev,
    TabSetCursor(segmented_button::Entity, Cursor),
    TabWidth(u16),
    Todo,
    ToggleAutoIndent,
    ToggleContextPage(ContextPage),
    ToggleHighlightCurrentLine,
    ToggleLineNumbers,
    ToggleWordWrap,
    Undo,
    VimBindings(bool),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextPage {
    About,
    DocumentStatistics,
    GitManagement,
    //TODO: Move search to pop-up
    ProjectSearch,
    PromptSaveChanges(segmented_button::Entity),
    Settings,
}

impl ContextPage {
    fn title(&self) -> String {
        match self {
            Self::About => String::new(),
            Self::DocumentStatistics => fl!("document-statistics"),
            Self::GitManagement => fl!("git-management"),
            Self::ProjectSearch => fl!("project-search"),
            Self::PromptSaveChanges(_) => fl!("prompt-save-changes-title"),
            Self::Settings => fl!("settings"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Find {
    None,
    Find,
    FindAndReplace,
}

pub struct App {
    core: Core,
    nav_model: segmented_button::SingleSelectModel,
    tab_model: segmented_button::SingleSelectModel,
    config_handler: Option<cosmic_config::Config>,
    config: Config,
    config_state_handler: Option<cosmic_config::Config>,
    config_state: ConfigState,
    key_binds: HashMap<KeyBind, Action>,
    app_themes: Vec<String>,
    font_names: Vec<String>,
    font_size_names: Vec<String>,
    font_sizes: Vec<u16>,
    theme_names: Vec<String>,
    context_page: ContextPage,
    text_box_id: widget::Id,
    dialog_opt: Option<Dialog<Message>>,
    find_opt: Option<bool>,
    find_replace_id: widget::Id,
    find_replace_value: String,
    find_search_id: widget::Id,
    find_search_value: String,
    git_project_status: Option<Vec<(String, PathBuf, Vec<GitStatus>)>>,
    projects: Vec<(String, PathBuf)>,
    project_search_id: widget::Id,
    project_search_value: String,
    project_search_result: Option<ProjectSearchResult>,
    watcher_opt: Option<notify::RecommendedWatcher>,
    modifiers: Modifiers,
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
        let path = path.as_ref();
        let node = match ProjectNode::new(path) {
            Ok(mut node) => {
                match &mut node {
                    ProjectNode::Folder {
                        name,
                        path,
                        open,
                        root,
                    } => {
                        *open = true;
                        *root = true;

                        for (_project_name, project_path) in self.projects.iter() {
                            if project_path == path {
                                // Project already open
                                return;
                            }
                        }

                        // Save the absolute path
                        self.projects.push((name.to_string(), path.to_path_buf()));

                        // Add to recent projects, ensuring only one entry
                        self.config_state.recent_projects.retain(|x| x != path);
                        self.config_state
                            .recent_projects
                            .push_front(path.to_path_buf());
                        self.config_state.recent_projects.truncate(10);
                        self.save_config_state();

                        // Open nav bar
                        self.core.nav_bar_set_toggled(true);
                    }
                    _ => {
                        log::error!("failed to open project {:?}: not a directory", path);
                        return;
                    }
                }
                node
            }
            Err(err) => {
                log::error!("failed to open project {:?}: {}", path, err);
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

        self.open_folder(path, position + 1, 1);
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
                    if let Some(Tab::Editor(tab)) = self.tab_model.data::<Tab>(entity) {
                        if tab.path_opt.as_ref() == Some(&canonical) {
                            activate_opt = Some(entity);
                            break;
                        }
                    }
                }
                if let Some(entity) = activate_opt {
                    self.tab_model.activate(entity);
                    return Some(entity);
                }

                // Add to recent files, ensuring only one entry
                self.config_state.recent_files.retain(|x| x != &canonical);
                self.config_state
                    .recent_files
                    .push_front(canonical.to_path_buf());
                self.config_state.recent_files.truncate(10);
                self.save_config_state();

                let mut tab = EditorTab::new(&self.config);
                tab.open(canonical);
                tab.watch(&mut self.watcher_opt);
                tab
            }
            None => EditorTab::new(&self.config),
        };

        Some(
            self.tab_model
                .insert()
                .text(tab.title())
                .icon(tab.icon(16))
                .data::<Tab>(Tab::Editor(tab))
                .closable()
                .activate()
                .id(),
        )
    }

    fn update_config(&mut self) -> Command<Message> {
        //TODO: provide iterator over data
        let entities: Vec<_> = self.tab_model.iter().collect();
        for entity in entities {
            if let Some(Tab::Editor(tab)) = self.tab_model.data_mut::<Tab>(entity) {
                tab.set_config(&self.config);
            }
        }
        cosmic::app::command::set_theme(self.config.app_theme.theme())
    }

    fn save_config(&mut self) -> Command<Message> {
        if let Some(ref config_handler) = self.config_handler {
            if let Err(err) = self.config.write_entry(config_handler) {
                log::error!("failed to save config: {}", err);
            }
        }
        self.update_config()
    }

    fn save_config_state(&mut self) {
        if let Some(ref config_state_handler) = self.config_state_handler {
            if let Err(err) = self.config_state.write_entry(config_state_handler) {
                log::error!("failed to save config_state: {}", err);
            }
        }
    }

    fn update_focus(&self) -> Command<Message> {
        if self.core.window.show_context {
            match self.context_page {
                ContextPage::ProjectSearch => {
                    widget::text_input::focus(self.project_search_id.clone())
                }
                _ => Command::none(),
            }
        } else if self.find_opt.is_some() {
            widget::text_input::focus(self.find_search_id.clone())
        } else {
            widget::text_input::focus(self.text_box_id.clone())
        }
    }

    fn update_nav_bar_active(&mut self) {
        let tab_path_opt = match self.active_tab() {
            Some(Tab::Editor(tab)) => tab.path_opt.clone(),
            Some(Tab::GitDiff(tab)) => Some(tab.diff.path.clone()),
            None => None,
        };

        // Locate tree node to activate
        let mut active_id = segmented_button::Entity::default();

        if let Some(tab_path) = tab_path_opt {
            // Automatically expand tree to find and select active file
            loop {
                let mut expand_opt = None;
                for id in self.nav_model.iter() {
                    if let Some(node) = self.nav_model.data(id) {
                        match node {
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
                        }
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
        self.nav_model.activate(active_id);
    }

    // Call this any time the tab changes
    pub fn update_tab(&mut self) -> Command<Message> {
        self.update_nav_bar_active();

        let title = match self.active_tab() {
            Some(tab) => {
                if let Tab::Editor(inner) = tab {
                    // Force redraw on tab switches
                    inner.editor.lock().unwrap().set_redraw(true);
                }
                tab.title()
            }
            None => "No Open File".to_string(),
        };

        let window_title = format!("{title} - {}", fl!("cosmic-text-editor"));
        Command::batch([
            self.set_window_title(window_title, self.main_window_id()),
            self.update_focus(),
        ])
    }

    fn about(&self) -> Element<Message> {
        let cosmic_theme::Spacing { space_xxs, .. } = self.core().system_theme().cosmic().spacing;
        let repository = "https://github.com/pop-os/cosmic-edit";
        let hash = env!("VERGEN_GIT_SHA");
        let short_hash: String = hash.chars().take(7).collect();
        let date = env!("VERGEN_GIT_COMMIT_DATE");
        widget::column::with_children(vec![
                widget::svg(widget::svg::Handle::from_memory(
                    &include_bytes!(
                        "../res/icons/hicolor/128x128/apps/com.system76.CosmicEdit.svg"
                    )[..],
                ))
                .into(),
                widget::text::title3(fl!("cosmic-text-editor")).into(),
                widget::button::link(repository)
                    .on_press(Message::LaunchUrl(repository.to_string()))
                    .padding(0)
                    .into(),
                widget::button::link(fl!(
                    "git-description",
                    hash = short_hash.as_str(),
                    date = date
                ))
                    .on_press(Message::LaunchUrl(format!("{}/commits/{}", repository, hash)))
                    .padding(0)
                .into(),
            ])
        .align_items(Alignment::Center)
        .spacing(space_xxs)
        .into()
    }

    fn document_statistics(&self) -> Element<Message> {
        //TODO: calculate in the background
        let mut character_count = 0;
        let mut character_count_no_spaces = 0;
        let mut line_count = 0;
        let mut word_count = 0;

        if let Some(Tab::Editor(tab)) = self.active_tab() {
            let editor = tab.editor.lock().unwrap();
            editor.with_buffer(|buffer| {
                line_count = buffer.lines.len();
                for line in buffer.lines.iter() {
                    let mut last_whitespace = true;
                    //TODO: do graphemes?
                    for c in line.text().chars() {
                        character_count += 1;
                        let is_whitespace = c.is_whitespace();
                        if !is_whitespace {
                            character_count_no_spaces += 1;
                            if last_whitespace {
                                word_count += 1;
                            }
                        }
                        last_whitespace = is_whitespace;
                    }
                }
            });
        }

        widget::settings::view_column(vec![widget::settings::view_section("")
            .add(
                widget::settings::item::builder(fl!("word-count"))
                    .control(widget::text(word_count.to_string())),
            )
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

    fn git_management(&self) -> Element<Message> {
        let spacing = self.core().system_theme().cosmic().spacing;

        if let Some(project_status) = &self.git_project_status {
            let (success_color, destructive_color, warning_color) = {
                let cosmic_theme = self.core().system_theme().cosmic();
                (
                    cosmic_theme.success_color(),
                    cosmic_theme.destructive_color(),
                    cosmic_theme.warning_color(),
                )
            };
            let added = || widget::text("[+]").style(theme::Text::Color(success_color.into()));
            let deleted =
                || widget::text("[-]").style(theme::Text::Color(destructive_color.into()));
            let modified = || widget::text("[*]").style(theme::Text::Color(warning_color.into()));

            let mut items =
                Vec::with_capacity(project_status.len().saturating_mul(3).saturating_add(1));
            items.push(widget::text(fl!("git-management-description")).into());

            for (project_name, project_path, status) in project_status.iter() {
                let mut unstaged_items = Vec::with_capacity(status.len());
                let mut staged_items = Vec::with_capacity(status.len());
                for item in status.iter() {
                    let relative_path = match item.path.strip_prefix(project_path) {
                        Ok(ok) => ok,
                        Err(err) => {
                            log::warn!(
                                "failed to find relative path of {:?} in project {:?}: {}",
                                item.path,
                                project_path,
                                err
                            );
                            &item.path
                        }
                    };

                    let text = match &item.old_path {
                        Some(old_path) => {
                            let old_relative_path = match old_path.strip_prefix(project_path) {
                                Ok(ok) => ok,
                                Err(err) => {
                                    log::warn!(
                                        "failed to find relative path of {:?} in project {:?}: {}",
                                        old_path,
                                        project_path,
                                        err
                                    );
                                    old_path
                                }
                            };
                            format!(
                                "{} -> {}",
                                old_relative_path.display(),
                                relative_path.display()
                            )
                        }
                        None => format!("{}", relative_path.display()),
                    };

                    let unstaged_opt = match item.unstaged {
                        GitStatusKind::Unmodified => None,
                        GitStatusKind::Modified => Some(modified()),
                        GitStatusKind::FileTypeChanged => Some(modified()),
                        GitStatusKind::Added => Some(added()),
                        GitStatusKind::Deleted => Some(deleted()),
                        GitStatusKind::Renamed => Some(modified()), //TODO
                        GitStatusKind::Copied => Some(modified()),  // TODO
                        GitStatusKind::Updated => Some(modified()),
                        GitStatusKind::Untracked => Some(added()),
                        GitStatusKind::SubmoduleModified => Some(modified()),
                    };

                    if let Some(icon) = unstaged_opt {
                        unstaged_items.push(
                            widget::button(
                                widget::row::with_children(vec![
                                    icon.into(),
                                    widget::text(text.clone()).into(),
                                ])
                                .spacing(spacing.space_xs),
                            )
                            .on_press(Message::PrepareGitDiff(
                                project_path.clone(),
                                item.path.clone(),
                                false,
                            ))
                            .style(theme::Button::AppletMenu)
                            .width(Length::Fill)
                            .into(),
                        );
                    }

                    let staged_opt = match item.staged {
                        GitStatusKind::Unmodified => None,
                        GitStatusKind::Modified => Some(modified()),
                        GitStatusKind::FileTypeChanged => Some(modified()),
                        GitStatusKind::Added => Some(added()),
                        GitStatusKind::Deleted => Some(deleted()),
                        GitStatusKind::Renamed => Some(modified()), //TODO
                        GitStatusKind::Copied => Some(modified()),  // TODO
                        GitStatusKind::Updated => Some(modified()),
                        GitStatusKind::Untracked => None,
                        GitStatusKind::SubmoduleModified => Some(modified()),
                    };

                    if let Some(icon) = staged_opt {
                        staged_items.push(
                            widget::button(
                                widget::row::with_children(vec![
                                    icon.into(),
                                    widget::text(text.clone()).into(),
                                ])
                                .spacing(spacing.space_xs),
                            )
                            .on_press(Message::PrepareGitDiff(
                                project_path.clone(),
                                item.path.clone(),
                                true,
                            ))
                            .style(theme::Button::AppletMenu)
                            .width(Length::Fill)
                            .into(),
                        );
                    }
                }

                items.push(widget::text::heading(project_name.clone()).into());

                if !unstaged_items.is_empty() {
                    items.push(
                        widget::settings::view_section(fl!("unstaged-changes"))
                            .add(widget::column::with_children(unstaged_items))
                            .into(),
                    );
                }

                if !staged_items.is_empty() {
                    items.push(
                        widget::settings::view_section(fl!("staged-changes"))
                            .add(widget::column::with_children(staged_items))
                            .into(),
                    );
                }
            }

            widget::column::with_children(items)
                .spacing(spacing.space_s)
                .padding([spacing.space_xxs, spacing.space_none])
                .into()
        } else {
            widget::column::with_children(vec![
                widget::text(fl!("git-management-description")).into(),
                widget::text(fl!("git-management-loading")).into(),
            ])
            .spacing(spacing.space_s)
            .padding([spacing.space_xxs, spacing.space_none])
            .into()
        }
    }

    fn project_search(&self) -> Element<Message> {
        let spacing = self.core().system_theme().cosmic().spacing;

        let search_input = widget::text_input::search_input(
            fl!("project-search"),
            self.project_search_value.clone(),
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

                for (file_i, file_search_result) in project_search_result.files.iter().enumerate() {
                    let mut column = widget::column::with_capacity(file_search_result.lines.len());
                    let mut line_number_width = 1;
                    if let Some(line_search_result) = file_search_result.lines.last() {
                        let mut number = line_search_result.number;
                        while number >= 10 {
                            number /= 10;
                            line_number_width += 1;
                        }
                    }
                    for (line_i, line_search_result) in file_search_result.lines.iter().enumerate()
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
                                    widget::text(line_search_result.text.to_string())
                                        .font(Font::MONOSPACE)
                                        .into(),
                                ])
                                .spacing(spacing.space_xs),
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
            .spacing(spacing.space_s)
            .padding([spacing.space_xxs, spacing.space_none])
            .into()
    }

    fn prompt_save_changes(&self, entity: segmented_button::Entity) -> Element<Message> {
        let (spacing, warning_color) = {
            let theme = self.core().system_theme().cosmic();
            (theme.spacing, theme.warning_text_color())
        };

        // "Save" is displayed regardless if the file was already saved because the message handles
        // "Save As" if necessary
        let save = fl!("save");
        let save_button = widget::button::suggested(save)
            .on_press(Message::Save)
            .width(Length::Fill);

        // "Save As" is only shown if the file has been saved previously
        // Rationale: The user may want to save the modified buffer as a new file
        let save_as_button = match self.tab_model.data(entity) {
            Some(Tab::Editor(tab)) if tab.path_opt.is_some() => {
                let save_as = fl!("save-as");
                let save_as_button = widget::button::suggested(save_as)
                    .on_press(Message::SaveAsDialog)
                    .width(Length::Fill);
                Some(save_as_button)
            }
            _ => None,
        };

        // TODO: Maybe a button to show diffs in a split?
        // And more info, such as the path if any?
        // let diff = widget::text(fl!("diff"));
        // let diff_button = widget::button(diff.into());

        // Discards unsaved changes
        let discard = fl!("discard");
        let discard_button = widget::button::destructive(discard)
            .on_press(Message::TabCloseForce(entity))
            .width(Length::Fill);

        widget::column::with_capacity(3)
            .push(
                widget::text(fl!("prompt-unsaved-changes"))
                    .style(theme::Text::Color(warning_color.into())),
            )
            .push(save_button)
            .push_maybe(save_as_button)
            .push(discard_button)
            .spacing(spacing.space_s)
            .into()
    }

    fn settings(&self) -> Element<Message> {
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
            let mut font_system = font_system().write().unwrap();
            let current_font_name = font_system.raw().db().family_name(&Family::Monospace);
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
                .add(
                    widget::settings::item::builder(fl!("theme")).control(widget::dropdown(
                        &self.app_themes,
                        Some(app_theme_selected),
                        move |index| {
                            Message::AppTheme(match index {
                                1 => AppTheme::Dark,
                                2 => AppTheme::Light,
                                _ => AppTheme::System,
                            })
                        },
                    )),
                )
                .add(
                    widget::settings::item::builder(fl!("syntax-dark")).control(widget::dropdown(
                        &self.theme_names,
                        dark_selected,
                        move |index| Message::SyntaxTheme(index, true),
                    )),
                )
                .add(
                    widget::settings::item::builder(fl!("syntax-light")).control(widget::dropdown(
                        &self.theme_names,
                        light_selected,
                        move |index| Message::SyntaxTheme(index, false),
                    )),
                )
                .add(
                    widget::settings::item::builder(fl!("default-font")).control(widget::dropdown(
                        &self.font_names,
                        font_selected,
                        Message::DefaultFont,
                    )),
                )
                .add(
                    widget::settings::item::builder(fl!("default-font-size")).control(
                        widget::dropdown(&self.font_size_names, font_size_selected, |index| {
                            Message::DefaultFontSize(index)
                        }),
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
            let mut font_system = font_system().write().unwrap();
            font_system
                .raw()
                .db_mut()
                .set_monospace_family(&flags.config.font_name);
        }

        let app_themes = vec![fl!("match-desktop"), fl!("dark"), fl!("light")];

        let font_names = {
            let mut font_names = Vec::new();
            let mut font_system = font_system().write().unwrap();
            //TODO: do not repeat, used in Tab::new
            let attrs = cosmic_text::Attrs::new().family(Family::Monospace);
            for face in font_system.raw().db().faces() {
                if attrs.matches(face) && face.monospaced {
                    //TODO: get localized name if possible
                    let font_name = face
                        .families
                        .first()
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

        let mut theme_names =
            Vec::with_capacity(SYNTAX_SYSTEM.get().unwrap().theme_set.themes.len());
        for (theme_name, _theme) in SYNTAX_SYSTEM.get().unwrap().theme_set.themes.iter() {
            theme_names.push(theme_name.to_string());
        }

        let mut app = App {
            core,
            nav_model: nav_bar::Model::builder().build(),
            tab_model: segmented_button::Model::builder().build(),
            config_handler: flags.config_handler,
            config: flags.config,
            config_state_handler: flags.config_state_handler,
            config_state: flags.config_state,
            key_binds: key_binds(),
            app_themes,
            font_names,
            font_size_names,
            font_sizes,
            theme_names,
            context_page: ContextPage::Settings,
            text_box_id: widget::Id::unique(),
            dialog_opt: None,
            find_opt: None,
            find_replace_id: widget::Id::unique(),
            find_replace_value: String::new(),
            find_search_id: widget::Id::unique(),
            find_search_value: String::new(),
            git_project_status: None,
            projects: Vec::new(),
            project_search_id: widget::Id::unique(),
            project_search_value: String::new(),
            project_search_result: None,
            watcher_opt: None,
            modifiers: Modifiers::empty(),
        };

        // Do not show nav bar by default. Will be opened by open_project if needed
        app.core.nav_bar_set_toggled(false);
        for arg in env::args().skip(1) {
            let path = PathBuf::from(arg);
            if path.is_dir() {
                app.open_project(path);
            } else {
                app.open_tab(Some(path));
            }
        }

        // Add button to open a project if none provided
        if app.nav_model.iter().next().is_none() {
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
            .style(theme::SegmentedButton::TabBar)
            .apply(widget::container)
            .padding(space_s)
            .width(Length::Fill);

        if !self.core().is_condensed() {
            nav = nav.max_width(280);
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

    fn on_context_drawer(&mut self) -> Command<Message> {
        // Focus correct widget
        self.update_focus()
    }

    //TODO: currently the first escape unfocuses, and the second calls this function
    fn on_escape(&mut self) -> Command<Message> {
        if self.core.window.show_context {
            // Close context drawer if open
            self.core.window.show_context = false;
        } else if self.find_opt.is_some() {
            // Close find if open
            self.find_opt = None;
        }

        // Focus correct widget
        self.update_focus()
    }

    fn on_nav_select(&mut self, id: nav_bar::Id) -> Command<Message> {
        // Toggle open state and get clone of node data
        let node_opt = match self.nav_model.data_mut::<ProjectNode>(id) {
            Some(node) => {
                if let ProjectNode::Folder { open, .. } = node {
                    *open = !*open;
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
                            while let Some(child_id) = self.nav_model.entity_at(position + 1) {
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
            Message::ConfigState(config_state) => {
                if config_state != self.config_state {
                    log::info!("update config state");
                    self.config_state = config_state;
                }
            }
            Message::CloseFile => {
                return self.update(Message::TabClose(self.tab_model.active()));
            }
            Message::CloseProject(project_i) => {
                if project_i < self.projects.len() {
                    let (_project_name, project_path) = self.projects.remove(project_i);
                    let mut position = 0;
                    let mut closing = false;
                    while let Some(id) = self.nav_model.entity_at(position) {
                        match self.nav_model.data::<ProjectNode>(id) {
                            Some(node) => {
                                if let ProjectNode::Folder { path, root, .. } = node {
                                    if path == &project_path {
                                        // Found the project root node, closing
                                        closing = true;
                                    } else if *root && closing {
                                        // Found another project root node after closing, breaking
                                        break;
                                    }
                                }
                            }
                            None => break,
                        }
                        if closing {
                            self.nav_model.remove(id);
                        } else {
                            position += 1;
                        }
                    }
                }
            }
            Message::Copy => {
                if let Some(Tab::Editor(tab)) = self.active_tab() {
                    let editor = tab.editor.lock().unwrap();
                    let selection_opt = editor.copy_selection();
                    if let Some(selection) = selection_opt {
                        return clipboard::write(selection);
                    }
                }
            }
            Message::Cut => {
                if let Some(Tab::Editor(tab)) = self.active_tab() {
                    let selection_opt = {
                        let mut editor = tab.editor.lock().unwrap();
                        let selection_opt = editor.copy_selection();
                        editor.start_change();
                        editor.delete_selection();
                        editor.finish_change();
                        selection_opt
                    };
                    if let Some(selection) = selection_opt {
                        return Command::batch([
                            clipboard::write(selection),
                            self.update(Message::TabChanged(self.tab_model.active())),
                        ]);
                    }
                }
            }
            Message::DefaultFont(index) => {
                match self.font_names.get(index) {
                    Some(font_name) => {
                        if font_name != &self.config.font_name {
                            // Update font name from config
                            {
                                let mut font_system = font_system().write().unwrap();
                                font_system.raw().db_mut().set_monospace_family(font_name);
                            }

                            // Reset line number cache
                            {
                                let mut line_number_cache =
                                    LINE_NUMBER_CACHE.get().unwrap().lock().unwrap();
                                line_number_cache.clear();
                            }

                            // This does a complete reset of shaping data!
                            let entities: Vec<_> = self.tab_model.iter().collect();
                            for entity in entities {
                                if let Some(Tab::Editor(tab)) =
                                    self.tab_model.data_mut::<Tab>(entity)
                                {
                                    let mut editor = tab.editor.lock().unwrap();
                                    editor.with_buffer_mut(|buffer| {
                                        for line in buffer.lines.iter_mut() {
                                            line.reset();
                                        }
                                    });
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
            Message::DialogMessage(dialog_message) => {
                if let Some(dialog) = &mut self.dialog_opt {
                    return dialog.update(dialog_message);
                }
            }
            Message::Find(find_opt) => {
                self.find_opt = find_opt;

                // Focus correct input
                return self.update_focus();
            }
            Message::FindCaseSensitive(find_case_sensitive) => {
                self.config.find_case_sensitive = find_case_sensitive;
                return self.save_config();
            }
            Message::FindNext => {
                if !self.find_search_value.is_empty() {
                    if let Some(Tab::Editor(tab)) = self.active_tab() {
                        //TODO: do not compile find regex on every search?
                        match self.config.find_regex(&self.find_search_value) {
                            Ok(regex) => {
                                tab.search(&regex, true);
                            }
                            Err(err) => {
                                //TODO: put regex error in find box
                                log::warn!(
                                    "failed to compile regex {:?}: {}",
                                    self.find_search_value,
                                    err
                                );
                            }
                        }
                    }
                }

                // Focus correct input
                return self.update_focus();
            }
            Message::FindPrevious => {
                if !self.find_search_value.is_empty() {
                    if let Some(Tab::Editor(tab)) = self.active_tab() {
                        //TODO: do not compile find regex on every search?
                        match self.config.find_regex(&self.find_search_value) {
                            Ok(regex) => {
                                tab.search(&regex, false);
                            }
                            Err(err) => {
                                //TODO: put regex error in find box
                                log::warn!(
                                    "failed to compile regex {:?}: {}",
                                    self.find_search_value,
                                    err
                                );
                            }
                        }
                    }
                }

                // Focus correct input
                return self.update_focus();
            }
            Message::FindReplace => {
                if !self.find_search_value.is_empty() {
                    if let Some(Tab::Editor(tab)) = self.active_tab() {
                        //TODO: do not compile find regex on every search?
                        match self.config.find_regex(&self.find_search_value) {
                            Ok(regex) => {
                                //TODO: support captures
                                tab.replace(&regex, &self.find_replace_value);
                                return self.update(Message::TabChanged(self.tab_model.active()));
                            }
                            Err(err) => {
                                //TODO: put regex error in find box
                                log::warn!(
                                    "failed to compile regex {:?}: {}",
                                    self.find_search_value,
                                    err
                                );
                            }
                        }
                    }
                }

                // Focus correct input
                return self.update_focus();
            }
            Message::FindReplaceAll => {
                if !self.find_search_value.is_empty() {
                    if let Some(Tab::Editor(tab)) = self.active_tab() {
                        //TODO: do not compile find regex on every search?
                        match self.config.find_regex(&self.find_search_value) {
                            Ok(regex) => {
                                //TODO: support captures
                                {
                                    let mut editor = tab.editor.lock().unwrap();
                                    editor.set_cursor(cosmic_text::Cursor::new(0, 0));
                                }
                                while tab.replace(&regex, &self.find_replace_value) {}
                                return self.update(Message::TabChanged(self.tab_model.active()));
                            }
                            Err(err) => {
                                //TODO: put regex error in find box
                                log::warn!(
                                    "failed to compile regex {:?}: {}",
                                    self.find_search_value,
                                    err
                                );
                            }
                        }
                    }
                }

                // Focus correct input
                return self.update_focus();
            }
            Message::FindReplaceValueChanged(value) => {
                self.find_replace_value = value;
            }
            Message::FindSearchValueChanged(value) => {
                self.find_search_value = value;
            }
            Message::FindUseRegex(find_use_regex) => {
                self.config.find_use_regex = find_use_regex;
                return self.save_config();
            }
            Message::GitProjectStatus(project_status) => {
                self.git_project_status = Some(project_status);
            }
            Message::Key(modifiers, key) => {
                for (key_bind, action) in self.key_binds.iter() {
                    if key_bind.matches(modifiers, &key) {
                        return self.update(action.message(None));
                    }
                }
            }
            Message::LaunchUrl(url) => match open::that_detached(&url) {
                Ok(()) => {}
                Err(err) => {
                    log::warn!("failed to open {:?}: {}", url, err);
                }
            },
            Message::Modifiers(modifiers) => {
                self.modifiers = modifiers;
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
                    if let Some(Tab::Editor(tab)) = self.tab_model.data::<Tab>(entity) {
                        if let Some(path) = &tab.path_opt {
                            if event.paths.contains(path) {
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
                }

                for entity in needs_reload {
                    match self.tab_model.data_mut::<Tab>(entity) {
                        Some(Tab::Editor(tab)) => {
                            tab.reload();
                        }
                        _ => {
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
                        if let Some(Tab::Editor(tab)) = self.tab_model.data::<Tab>(entity) {
                            tab.watch(&mut self.watcher_opt);
                        }
                    }
                }
                None => {
                    log::warn!("message did not contain notify watcher");
                }
            },
            Message::OpenFile(path) => {
                self.open_tab(Some(path));
                return self.update_tab();
            }
            Message::OpenFileDialog => {
                if self.dialog_opt.is_none() {
                    let (dialog, command) = Dialog::new(
                        DialogKind::OpenMultipleFiles,
                        None,
                        Message::DialogMessage,
                        Message::OpenFileResult,
                    );
                    self.dialog_opt = Some(dialog);
                    return command;
                }
            }
            Message::OpenFileResult(result) => {
                self.dialog_opt = None;
                match result {
                    DialogResult::Cancel => {}
                    DialogResult::Open(paths) => {
                        for path in paths {
                            self.open_tab(Some(path));
                        }
                        return self.update_tab();
                    }
                }
            }
            Message::OpenGitDiff(project_path, diff) => {
                let relative_path = match diff.path.strip_prefix(project_path.clone()) {
                    Ok(ok) => ok,
                    Err(err) => {
                        log::warn!(
                            "failed to find relative path of {:?} in project {:?}: {}",
                            diff.path,
                            project_path,
                            err
                        );
                        &diff.path
                    }
                };
                let title = format!(
                    "{}: {}",
                    if diff.staged {
                        fl!("staged-changes")
                    } else {
                        fl!("unstaged-changes")
                    },
                    relative_path.display()
                );
                let icon = icon::icon(mime_icon(mime_for_path(&diff.path), 16)).size(16);
                let tab = Tab::GitDiff(GitDiffTab { title, diff });
                self.tab_model
                    .insert()
                    .text(tab.title())
                    .icon(icon)
                    .data::<Tab>(tab)
                    .closable()
                    .activate();
                return self.update_tab();
            }
            Message::OpenProjectDialog => {
                if self.dialog_opt.is_none() {
                    let (dialog, command) = Dialog::new(
                        DialogKind::OpenMultipleFolders,
                        None,
                        Message::DialogMessage,
                        Message::OpenProjectResult,
                    );
                    self.dialog_opt = Some(dialog);
                    return command;
                }
            }
            Message::OpenProjectResult(result) => {
                self.dialog_opt = None;
                match result {
                    DialogResult::Cancel => {}
                    DialogResult::Open(paths) => {
                        for path in paths {
                            self.open_project(path);
                        }
                    }
                }
            }
            Message::OpenRecentFile(index) => {
                if let Some(path) = self.config_state.recent_files.get(index).cloned() {
                    self.open_tab(Some(path));
                    return self.update_tab();
                }
            }
            Message::OpenRecentProject(index) => {
                if let Some(path) = self.config_state.recent_projects.get(index).cloned() {
                    self.open_project(path);
                }
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
            Message::PasteValue(value) => {
                if let Some(Tab::Editor(tab)) = self.active_tab() {
                    {
                        let mut editor = tab.editor.lock().unwrap();
                        editor.start_change();
                        editor.insert_string(&value, None);
                        editor.finish_change();
                    }
                    return self.update(Message::TabChanged(self.tab_model.active()));
                }
            }
            Message::PrepareGitDiff(project_path, path, staged) => {
                return Command::perform(
                    async move {
                        //TODO: send errors to UI
                        match GitRepository::new(&project_path) {
                            Ok(repo) => match repo.diff(&path, staged).await {
                                Ok(diff) => {
                                    return message::app(Message::OpenGitDiff(project_path, diff));
                                }
                                Err(err) => {
                                    log::error!(
                                        "failed to get diff of {:?} in {:?}: {}",
                                        path,
                                        project_path,
                                        err
                                    );
                                }
                            },
                            Err(err) => {
                                log::error!(
                                    "failed to open repository {:?}: {}",
                                    project_path,
                                    err
                                );
                            }
                        }
                        message::none()
                    },
                    |x| x,
                );
            }
            Message::ProjectSearchResult(project_search_result) => {
                self.project_search_result = Some(project_search_result);

                // Focus correct input
                return self.update_focus();
            }
            Message::ProjectSearchSubmit => {
                //TODO: Figure out length requirements?
                if !self.project_search_value.is_empty() {
                    let projects = self.projects.clone();
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
                                project_search_result.search_projects(projects);
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
                return window::close(window::Id::MAIN);
            }
            Message::Redo => {
                if let Some(Tab::Editor(tab)) = self.active_tab() {
                    {
                        let mut editor = tab.editor.lock().unwrap();
                        editor.redo();
                    }

                    return self.update(Message::TabChanged(self.tab_model.active()));
                }
            }
            Message::Save => {
                let mut title_opt = None;

                if let Some(Tab::Editor(tab)) = self.active_tab_mut() {
                    if tab.path_opt.is_none() {
                        return self.update(Message::SaveAsDialog);
                    }
                    title_opt = Some(tab.title());
                    tab.save();
                }

                if let Some(title) = title_opt {
                    self.tab_model.text_set(self.tab_model.active(), title);
                }
            }
            Message::SaveAsDialog => {
                if self.dialog_opt.is_none() {
                    let entity = self.tab_model.active();
                    if let Some(Tab::Editor(tab)) = self.tab_model.data::<Tab>(entity) {
                        let (filename, path_opt) = match &tab.path_opt {
                            Some(path) => (
                                path.file_name()
                                    .and_then(|x| x.to_str())
                                    .map(|x| x.to_string())
                                    .unwrap_or(String::new()),
                                path.parent().map(|x| x.to_path_buf()),
                            ),
                            None => (String::new(), None),
                        };
                        let (dialog, command) = Dialog::new(
                            DialogKind::SaveFile { filename },
                            path_opt,
                            Message::DialogMessage,
                            move |result| Message::SaveAsResult(entity, result),
                        );
                        self.dialog_opt = Some(dialog);
                        return command;
                    }
                }
            }
            Message::SaveAsResult(entity, result) => {
                self.dialog_opt = None;
                match result {
                    DialogResult::Cancel => {}
                    DialogResult::Open(mut paths) => {
                        if !paths.is_empty() {
                            let mut title_opt = None;
                            if let Some(Tab::Editor(tab)) = self.tab_model.data_mut::<Tab>(entity) {
                                tab.path_opt = Some(paths.remove(0));
                                title_opt = Some(tab.title());
                                tab.save();
                            }
                            if let Some(title) = title_opt {
                                self.tab_model.text_set(entity, title);
                            }
                            // Close save changes prompt only if the dialog succeeded
                            if let ContextPage::PromptSaveChanges(_) = self.context_page {
                                self.core_mut().window.show_context = false;
                            }
                        }
                    }
                }
            }
            Message::SelectAll => {
                if let Some(Tab::Editor(tab)) = self.active_tab_mut() {
                    let mut editor = tab.editor.lock().unwrap();

                    // Set cursor to lowest possible value
                    editor.set_cursor(Cursor::new(0, 0));

                    // Set selection end to highest possible value
                    let selection = editor.with_buffer(|buffer| {
                        let last_line = buffer.lines.len().saturating_sub(1);
                        let last_index = buffer.lines[last_line].text().len();
                        Selection::Normal(Cursor::new(last_line, last_index))
                    });
                    editor.set_selection(selection);
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
            Message::TabActivateJump(pos) => {
                // Length is always at least one, so there shouldn't be a division by zero
                let len = self.tab_model.iter().count();
                // Indices 1 to 8 jumps to tabs 1-8 while 9 jumps to the last
                let pos = if pos >= 8 || pos > len - 1 {
                    len - 1
                } else {
                    pos % len
                };

                let entity = self.tab_model.iter().nth(pos);
                if let Some(entity) = entity {
                    return self.update(Message::TabActivate(entity));
                }
            }
            Message::TabChanged(entity) => {
                if let Some(Tab::Editor(tab)) = self.tab_model.data::<Tab>(entity) {
                    let mut title = tab.title();
                    //TODO: better way of adding change indicator
                    if tab.changed() {
                        title.push_str(" \u{2022}");
                    }
                    self.tab_model.text_set(entity, title);
                }
            }
            Message::TabClose(entity) => {
                match self.tab_model.data_mut::<Tab>(entity) {
                    // Only match a changed editor tab...
                    Some(Tab::Editor(tab)) if tab.changed() => {
                        // * The save prompt shouldn't be closed if `TabClose` is emitted again
                        // * Prompt should be opened if no pages are open
                        // * Prompt should replace a different page (e.g. Settings)
                        //
                        // `PromptSaveChanges` for a different tab other than `entity` counts as
                        // a different page
                        // Ex. If tab 2 and 3 both have unsaved changes and `PromptSaveChanges` is
                        // open for tab 2, closing tab 3 should open the page for tab 3
                        if !self.core().window.show_context
                            || self.context_page != ContextPage::PromptSaveChanges(entity)
                        {
                            // Focus the tab in case the user is closing an unfocussed tab
                            // Otherwise, closing an unfocussed tab would be very confusing
                            return Command::batch([
                                self.update(Message::TabActivate(entity)),
                                self.update(Message::ToggleContextPage(
                                    ContextPage::PromptSaveChanges(entity),
                                )),
                            ]);
                        }
                    }
                    // ...or else just close it
                    _ => {
                        return self.update(Message::TabCloseForce(entity));
                    }
                }
            }
            Message::TabCloseForce(entity) => {
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

                // Close PromptSaveChanges page if open for this entity
                if self.core().window.show_context
                    && matches!(self.context_page, ContextPage::PromptSaveChanges(_))
                {
                    return Command::batch([
                        self.update(Message::ToggleContextPage(ContextPage::PromptSaveChanges(
                            entity,
                        ))),
                        self.update_tab(),
                    ]);
                }

                return self.update_tab();
            }
            Message::TabContextAction(entity, action) => {
                if let Some(Tab::Editor(tab)) = self.tab_model.data_mut::<Tab>(entity) {
                    // Close context menu
                    tab.context_menu = None;
                    // Run action's message
                    return self.update(action.message(None));
                }
            }
            Message::TabContextMenu(entity, position_opt) => {
                if let Some(Tab::Editor(tab)) = self.tab_model.data_mut::<Tab>(entity) {
                    // Update context menu
                    tab.context_menu = position_opt;
                }
            }
            Message::TabNext => {
                let len = self.tab_model.iter().count();
                // Next tab position. Wraps around to 0 (the first tab) if the last tab is active.
                let pos = self
                    .tab_model
                    .position(self.tab_model.active())
                    .map(|i| (i as usize + 1) % len)
                    .expect("at least one tab is always open");

                let entity = self.tab_model.iter().nth(pos);
                if let Some(entity) = entity {
                    return self.update(Message::TabActivate(entity));
                }
            }
            Message::TabPrev => {
                let pos = self
                    .tab_model
                    .position(self.tab_model.active())
                    .and_then(|i| (i as usize).checked_sub(1))
                    .unwrap_or_else(|| {
                        self.tab_model
                            .iter()
                            .count()
                            .checked_sub(1)
                            .unwrap_or_default()
                    });

                let entity = self.tab_model.iter().nth(pos);
                if let Some(entity) = entity {
                    return self.update(Message::TabActivate(entity));
                }
            }
            Message::TabSetCursor(entity, cursor) => {
                if let Some(Tab::Editor(tab)) = self.tab_model.data::<Tab>(entity) {
                    let mut editor = tab.editor.lock().unwrap();
                    editor.set_cursor(cursor);
                }
            }
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

                // Execute commands for specific pages
                if self.core.window.show_context && self.context_page == ContextPage::GitManagement
                {
                    self.git_project_status = None;
                    let projects = self.projects.clone();
                    return Command::perform(
                        async move {
                            let mut project_status = Vec::new();
                            for (project_name, project_path) in projects.iter() {
                                //TODO: send errors to UI
                                match GitRepository::new(project_path) {
                                    Ok(repo) => match repo.status().await {
                                        Ok(status) => {
                                            if !status.is_empty() {
                                                project_status.push((
                                                    project_name.clone(),
                                                    project_path.clone(),
                                                    status,
                                                ));
                                            }
                                        }
                                        Err(err) => {
                                            log::error!(
                                                "failed to get status of {:?}: {}",
                                                project_path,
                                                err
                                            );
                                        }
                                    },
                                    Err(err) => {
                                        log::error!(
                                            "failed to open repository {:?}: {}",
                                            project_path,
                                            err
                                        );
                                    }
                                }
                            }
                            message::app(Message::GitProjectStatus(project_status))
                        },
                        |x| x,
                    );
                }

                // Ensure focus of correct input
                return self.update_focus();
            }
            Message::ToggleHighlightCurrentLine => {
                self.config.highlight_current_line = !self.config.highlight_current_line;

                // This forces a redraw of all buffers
                let entities: Vec<_> = self.tab_model.iter().collect();
                for entity in entities {
                    if let Some(Tab::Editor(tab)) = self.tab_model.data_mut::<Tab>(entity) {
                        let mut editor = tab.editor.lock().unwrap();
                        editor.set_redraw(true);
                    }
                }

                return self.save_config();
            }
            Message::ToggleLineNumbers => {
                self.config.line_numbers = !self.config.line_numbers;

                // This forces a redraw of all buffers
                let entities: Vec<_> = self.tab_model.iter().collect();
                for entity in entities {
                    if let Some(Tab::Editor(tab)) = self.tab_model.data_mut::<Tab>(entity) {
                        let mut editor = tab.editor.lock().unwrap();
                        editor.set_redraw(true);
                    }
                }

                return self.save_config();
            }
            Message::ToggleWordWrap => {
                self.config.word_wrap = !self.config.word_wrap;
                return self.save_config();
            }
            Message::Undo => {
                if let Some(Tab::Editor(tab)) = self.active_tab() {
                    {
                        let mut editor = tab.editor.lock().unwrap();
                        editor.undo();
                    }

                    return self.update(Message::TabChanged(self.tab_model.active()));
                }
            }
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

        Some(match self.context_page {
            ContextPage::About => self.about(),
            ContextPage::DocumentStatistics => self.document_statistics(),
            ContextPage::GitManagement => self.git_management(),
            ContextPage::ProjectSearch => self.project_search(),
            ContextPage::PromptSaveChanges(entity) => self.prompt_save_changes(entity),
            ContextPage::Settings => self.settings(),
        })
    }

    fn header_start(&self) -> Vec<Element<Message>> {
        vec![menu_bar(
            &self.config,
            &self.config_state,
            &self.key_binds,
            &self.projects,
        )]
    }

    fn view(&self) -> Element<Message> {
        let cosmic_theme::Spacing {
            space_none,
            space_xxs,
            ..
        } = self.core().system_theme().cosmic().spacing;

        let mut tab_column = widget::column::with_capacity(3).padding([space_none, space_xxs]);

        tab_column = tab_column.push(
            widget::row::with_capacity(2)
                .align_items(Alignment::Center)
                .push(
                    widget::tab_bar::horizontal(&self.tab_model)
                        .button_height(32)
                        .button_spacing(space_xxs)
                        .close_icon(icon_cache_get("window-close-symbolic", 16))
                        //TODO: this causes issues with small window sizes .minimum_button_width(240)
                        .on_activate(Message::TabActivate)
                        .on_close(Message::TabClose)
                        .width(Length::Shrink),
                )
                .push(
                    button(icon_cache_get("list-add-symbolic", 16))
                        .on_press(Message::NewFile)
                        .padding(space_xxs)
                        .style(style::Button::Icon),
                ),
        );

        let tab_id = self.tab_model.active();
        match self.tab_model.data::<Tab>(tab_id) {
            Some(Tab::Editor(tab)) => {
                let status = {
                    let editor = tab.editor.lock().unwrap();
                    let parser = editor.parser();
                    match &parser.mode {
                        ViMode::Normal => {
                            format!("{}", parser.cmd)
                        }
                        ViMode::Insert => "-- INSERT --".to_string(),
                        ViMode::Extra(extra) => {
                            format!("{}{}", parser.cmd, extra)
                        }
                        ViMode::Replace => "-- REPLACE --".to_string(),
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
                    .id(self.text_box_id.clone())
                    .on_changed(Message::TabChanged(tab_id))
                    .has_context_menu(tab.context_menu.is_some())
                    .on_context_menu(move |position_opt| {
                        Message::TabContextMenu(tab_id, position_opt)
                    });
                if self.config.highlight_current_line {
                    text_box = text_box.highlight_current_line();
                }
                if self.config.line_numbers {
                    text_box = text_box.line_numbers();
                }
                let mut popover = widget::popover(text_box);
                if let Some(point) = tab.context_menu {
                    popover = popover
                        .popup(menu::context_menu(&self.key_binds, tab_id))
                        .position(widget::popover::Position::Point(point));
                }
                tab_column = tab_column.push(popover);
                if !status.is_empty() {
                    tab_column = tab_column.push(text(status).font(Font::MONOSPACE));
                }
            }
            Some(Tab::GitDiff(tab)) => {
                let mut diff_widget = widget::column::with_capacity(tab.diff.hunks.len());
                for hunk in tab.diff.hunks.iter() {
                    let mut hunk_widget = widget::column::with_capacity(hunk.lines.len());
                    for line in hunk.lines.iter() {
                        let line_widget = match line {
                            GitDiffLine::Context {
                                old_line,
                                new_line,
                                text,
                            } => widget::container(widget::text::monotext(format!(
                                "{:4} {:4}   {}",
                                old_line, new_line, text
                            ))),
                            GitDiffLine::Added { new_line, text } => widget::container(
                                widget::text::monotext(format!(
                                    "{:4} {:4} + {}",
                                    "", new_line, text
                                )),
                            )
                            .style(theme::Container::Custom(Box::new(|_theme| {
                                //TODO: theme this color
                                widget::container::Appearance {
                                    background: Some(Background::Color(Color::from_rgb8(
                                        0x00, 0x40, 0x00,
                                    ))),
                                    ..Default::default()
                                }
                            }))),
                            GitDiffLine::Deleted { old_line, text } => widget::container(
                                widget::text::monotext(format!(
                                    "{:4} {:4} - {}",
                                    old_line, "", text
                                )),
                            )
                            .style(theme::Container::Custom(Box::new(|_theme| {
                                //TODO: theme this color
                                widget::container::Appearance {
                                    background: Some(Background::Color(Color::from_rgb8(
                                        0x40, 0x00, 0x00,
                                    ))),
                                    ..Default::default()
                                }
                            }))),
                        };
                        hunk_widget = hunk_widget.push(line_widget.width(Length::Fill));
                    }
                    diff_widget = diff_widget.push(hunk_widget);
                }
                tab_column = tab_column.push(widget::scrollable(
                    widget::layer_container(diff_widget).layer(cosmic_theme::Layer::Primary),
                ));
            }
            None => {}
        }

        if let Some(replace) = &self.find_opt {
            let find_input =
                widget::text_input::text_input(fl!("find-placeholder"), &self.find_search_value)
                    .id(self.find_search_id.clone())
                    .on_input(Message::FindSearchValueChanged)
                    .on_submit(if self.modifiers.contains(Modifiers::SHIFT) {
                        Message::FindPrevious
                    } else {
                        Message::FindNext
                    })
                    .width(Length::Fixed(320.0))
                    .trailing_icon(
                        button(icon_cache_get("edit-clear-symbolic", 16))
                            .on_press(Message::FindSearchValueChanged(String::new()))
                            .style(style::Button::Icon)
                            .into(),
                    );
            let find_widget = widget::row::with_children(vec![
                find_input.into(),
                widget::tooltip(
                    button(icon_cache_get("go-up-symbolic", 16))
                        .on_press(Message::FindPrevious)
                        .padding(space_xxs)
                        .style(style::Button::Icon),
                    fl!("find-previous"),
                    widget::tooltip::Position::Top,
                )
                .into(),
                widget::tooltip(
                    button(icon_cache_get("go-down-symbolic", 16))
                        .on_press(Message::FindNext)
                        .padding(space_xxs)
                        .style(style::Button::Icon),
                    fl!("find-next"),
                    widget::tooltip::Position::Top,
                )
                .into(),
                widget::horizontal_space(Length::Fill).into(),
                button(icon_cache_get("window-close-symbolic", 16))
                    .on_press(Message::Find(None))
                    .padding(space_xxs)
                    .style(style::Button::Icon)
                    .into(),
            ])
            .align_items(Alignment::Center)
            .padding(space_xxs)
            .spacing(space_xxs);

            let mut column = widget::column::with_capacity(3).push(find_widget);
            if *replace {
                let replace_input = widget::text_input::text_input(
                    fl!("replace-placeholder"),
                    &self.find_replace_value,
                )
                .id(self.find_replace_id.clone())
                .on_input(Message::FindReplaceValueChanged)
                .on_submit(Message::FindReplace)
                .width(Length::Fixed(320.0))
                .trailing_icon(
                    button(icon_cache_get("edit-clear-symbolic", 16))
                        .on_press(Message::FindReplaceValueChanged(String::new()))
                        .style(style::Button::Icon)
                        .into(),
                );
                let replace_widget = widget::row::with_children(vec![
                    replace_input.into(),
                    widget::tooltip(
                        button(icon_cache_get("replace-symbolic", 16))
                            .on_press(Message::FindReplace)
                            .padding(space_xxs)
                            .style(style::Button::Icon),
                        fl!("replace"),
                        widget::tooltip::Position::Top,
                    )
                    .into(),
                    widget::tooltip(
                        button(icon_cache_get("replace-all-symbolic", 16))
                            .on_press(Message::FindReplaceAll)
                            .padding(space_xxs)
                            .style(style::Button::Icon),
                        fl!("replace-all"),
                        widget::tooltip::Position::Top,
                    )
                    .into(),
                ])
                .align_items(Alignment::Center)
                .padding(space_xxs)
                .spacing(space_xxs);

                column = column.push(replace_widget);
            }

            column = column.push(
                widget::row::with_children(vec![
                    widget::checkbox(
                        fl!("case-sensitive"),
                        self.config.find_case_sensitive,
                        Message::FindCaseSensitive,
                    )
                    .into(),
                    widget::checkbox(
                        fl!("use-regex"),
                        self.config.find_use_regex,
                        Message::FindUseRegex,
                    )
                    .into(),
                ])
                .align_items(Alignment::Center)
                .padding(space_xxs)
                .spacing(space_xxs),
            );

            tab_column = tab_column
                .push(widget::layer_container(column).layer(cosmic_theme::Layer::Primary));
        }

        let content: Element<_> = tab_column.into();

        // Uncomment to debug layout:
        //content.explain(cosmic::iced::Color::WHITE)
        content
    }

    fn view_window(&self, window_id: window::Id) -> Element<Message> {
        match &self.dialog_opt {
            Some(dialog) => dialog.view(window_id),
            None => widget::text("Unknown window ID").into(),
        }
    }

    fn subscription(&self) -> subscription::Subscription<Message> {
        struct WatcherSubscription;
        struct ConfigSubscription;
        struct ConfigStateSubscription;
        struct ThemeSubscription;

        subscription::Subscription::batch([
            event::listen_with(|event, status| match event {
                event::Event::Keyboard(keyboard::Event::KeyPressed { modifiers, key, .. }) => {
                    match status {
                        event::Status::Ignored => Some(Message::Key(modifiers, key)),
                        event::Status::Captured => None,
                    }
                }
                event::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                    Some(Message::Modifiers(modifiers))
                }
                _ => None,
            }),
            subscription::channel(
                TypeId::of::<WatcherSubscription>(),
                100,
                |mut output| async move {
                    let watcher_res = {
                        let mut output = output.clone();
                        //TODO: debounce
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
            .map(|update| {
                for error in update.errors {
                    log::error!("error loading config: {error:?}");
                }

                Message::Config(update.config)
            }),
            cosmic_config::config_state_subscription(
                TypeId::of::<ConfigStateSubscription>(),
                Self::APP_ID.into(),
                CONFIG_VERSION,
            )
            .map(|update| {
                for error in update.errors {
                    log::error!("error loading config: {error:?}");
                }

                Message::ConfigState(update.config)
            }),
            cosmic_config::config_subscription::<_, cosmic_theme::ThemeMode>(
                TypeId::of::<ThemeSubscription>(),
                cosmic_theme::THEME_MODE_ID.into(),
                cosmic_theme::ThemeMode::version(),
            )
            .map(|update| {
                for error in update.errors {
                    log::error!("error loading theme mode: {error:?}");
                }

                Message::SystemThemeModeChange(update.config)
            }),
            match &self.dialog_opt {
                Some(dialog) => dialog.subscription(),
                None => subscription::Subscription::none(),
            },
        ])
    }
}
