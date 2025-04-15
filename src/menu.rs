// SPDX-License-Identifier: GPL-3.0-only

use cosmic::widget::menu::key_bind::KeyBind;
use cosmic::widget::menu::{items as menu_items, root as menu_root, Item as MenuItem};
use cosmic::{
    iced::{widget::column, Background, Length},
    iced_core::Border,
    widget::{
        self, divider, horizontal_space,
        menu::{menu_button, ItemHeight, ItemWidth, MenuBar, Tree as MenuTree},
        segmented_button,
    },
    Element,
};
use std::{collections::HashMap, path::PathBuf};

use crate::{fl, Action, Config, ConfigState, Message};

pub fn context_menu<'a>(
    key_binds: &HashMap<KeyBind, Action>,
    entity: segmented_button::Entity,
) -> Element<'a, Message> {
    let menu_item = |menu_label, menu_action| {
        let mut key = String::new();
        for (key_bind, key_action) in key_binds.iter() {
            if key_action == &menu_action {
                key = key_bind.to_string();
                break;
            }
        }
        menu_button(vec![
            widget::text(menu_label).into(),
            horizontal_space().into(),
            widget::text(key).into(),
        ])
        .on_press(Message::TabContextAction(entity, menu_action))
    };

    widget::container(column!(
        menu_item(fl!("undo"), Action::Undo),
        menu_item(fl!("redo"), Action::Redo),
        divider::horizontal::light(),
        menu_item(fl!("cut"), Action::Cut),
        menu_item(fl!("copy"), Action::Copy),
        menu_item(fl!("paste"), Action::Paste),
        menu_item(fl!("select-all"), Action::SelectAll),
    ))
    .padding(1)
    //TODO: move style to libcosmic
    .style(|theme| {
        let cosmic = theme.cosmic();
        let component = &cosmic.background.component;
        widget::container::Style {
            icon_color: Some(component.on.into()),
            text_color: Some(component.on.into()),
            background: Some(Background::Color(component.base.into())),
            border: Border {
                radius: cosmic.radius_s().map(|x| x + 1.0).into(),
                width: 1.0,
                color: component.divider.into(),
            },
            ..Default::default()
        }
    })
    .width(Length::Fixed(240.0))
    .into()
}

pub fn menu_bar<'a>(
    config: &Config,
    config_state: &ConfigState,
    key_binds: &HashMap<KeyBind, Action>,
    projects: &Vec<(String, PathBuf)>,
) -> Element<'a, Message> {
    //TODO: port to libcosmic
    let menu_tab_width = |tab_width: u16| {
        MenuItem::CheckBox(
            fl!("tab-width", tab_width = tab_width),
            None,
            config.tab_width == tab_width,
            Action::TabWidth(tab_width),
        )
    };

    let home_dir_opt = dirs::home_dir();
    let format_path = |path: &PathBuf| -> String {
        if let Some(home_dir) = &home_dir_opt {
            if let Ok(part) = path.strip_prefix(home_dir) {
                return format!("~/{}", part.display());
            }
        }
        path.display().to_string()
    };

    let mut recent_files = Vec::with_capacity(config_state.recent_files.len());
    for (i, path) in config_state.recent_files.iter().enumerate() {
        recent_files.push(MenuItem::Button(
            format_path(path),
            None,
            Action::OpenRecentFile(i),
        ));
    }

    let mut recent_projects = Vec::with_capacity(config_state.recent_projects.len());
    for (i, path) in config_state.recent_projects.iter().enumerate() {
        recent_projects.push(MenuItem::Button(
            format_path(path),
            None,
            Action::OpenRecentProject(i),
        ));
    }

    let mut close_projects = Vec::with_capacity(projects.len());
    for (project_i, (name, _path)) in projects.iter().enumerate() {
        close_projects.push(MenuItem::Button(
            name.clone(),
            None,
            Action::CloseProject(project_i),
        ));
    }

    MenuBar::new(vec![
        MenuTree::with_children(
            menu_root(fl!("file")),
            menu_items(
                key_binds,
                vec![
                    MenuItem::Button(fl!("new-file"), None, Action::NewFile),
                    MenuItem::Button(fl!("new-window"), None, Action::NewWindow),
                    MenuItem::Divider,
                    MenuItem::Button(fl!("open-file"), None, Action::OpenFileDialog),
                    MenuItem::Folder(fl!("open-recent-file"), recent_files),
                    MenuItem::Button(fl!("close-file"), None, Action::CloseFile),
                    MenuItem::Divider,
                    MenuItem::Button(fl!("menu-open-project"), None, Action::OpenProjectDialog),
                    MenuItem::Folder(fl!("open-recent-project"), recent_projects),
                    MenuItem::Folder(fl!("close-project"), close_projects),
                    MenuItem::Divider,
                    MenuItem::Button(fl!("save"), None, Action::Save),
                    MenuItem::Button(fl!("save-as"), None, Action::SaveAsDialog),
                    MenuItem::Divider,
                    MenuItem::Button(fl!("revert-all-changes"), None, Action::RevertAllChanges),
                    MenuItem::Divider,
                    MenuItem::Button(
                        fl!("menu-document-statistics"),
                        None,
                        Action::ToggleDocumentStatistics,
                    ),
                    //TODO MenuItem::Button(fl!("document-type"), Action::Todo),
                    //TODO MenuItem::Button(fl!("encoding"), Action::Todo),
                    MenuItem::Button(
                        fl!("menu-git-management"),
                        None,
                        Action::ToggleGitManagement,
                    ),
                    MenuItem::Divider,
                    MenuItem::Button(fl!("print"), None, Action::Print),
                    MenuItem::Divider,
                    MenuItem::Button(fl!("quit"), None, Action::Quit),
                ],
            ),
        ),
        MenuTree::with_children(
            menu_root(fl!("edit")),
            menu_items(
                key_binds,
                vec![
                    MenuItem::Button(fl!("undo"), None, Action::Undo),
                    MenuItem::Button(fl!("redo"), None, Action::Redo),
                    MenuItem::Divider,
                    MenuItem::Button(fl!("cut"), None, Action::Cut),
                    MenuItem::Button(fl!("copy"), None, Action::Copy),
                    MenuItem::Button(fl!("paste"), None, Action::Paste),
                    MenuItem::Button(fl!("select-all"), None, Action::SelectAll),
                    MenuItem::Divider,
                    MenuItem::Button(fl!("find"), None, Action::Find),
                    MenuItem::Button(fl!("replace"), None, Action::FindAndReplace),
                    MenuItem::Button(fl!("find-in-project"), None, Action::ToggleProjectSearch),
                    /*TODO: implement spell-check
                    MenuItem::Divider,
                    MenuItem::Button(fl!("spell-check"), None, Action::Todo),
                    */
                ],
            ),
        ),
        MenuTree::with_children(
            menu_root(fl!("view")),
            menu_items(
                key_binds,
                vec![
                    MenuItem::Folder(
                        fl!("indentation"),
                        vec![
                            MenuItem::CheckBox(
                                fl!("automatic-indentation"),
                                None,
                                config.auto_indent,
                                Action::ToggleAutoIndent,
                            ),
                            MenuItem::Divider,
                            menu_tab_width(1),
                            menu_tab_width(2),
                            menu_tab_width(3),
                            menu_tab_width(4),
                            menu_tab_width(5),
                            menu_tab_width(6),
                            menu_tab_width(7),
                            menu_tab_width(8),
                            //TODO MenuItem::Divider,
                            //TODO MenuItem::Button(fl!("convert-indentation-to-spaces"), Action::Todo),
                            //TODO MenuItem::Button(fl!("convert-indentation-to-tabs"), Action::Todo),
                        ],
                    ),
                    MenuItem::Divider,
                    MenuItem::Button(fl!("zoom-in"), None, Action::ZoomIn),
                    MenuItem::Button(fl!("default-size"), None, Action::ZoomReset),
                    MenuItem::Button(fl!("zoom-out"), None, Action::ZoomOut),
                    MenuItem::Divider,
                    MenuItem::CheckBox(
                        fl!("word-wrap"),
                        None,
                        config.word_wrap,
                        Action::ToggleWordWrap,
                    ),
                    MenuItem::CheckBox(
                        fl!("show-line-numbers"),
                        None,
                        config.line_numbers,
                        Action::ToggleLineNumbers,
                    ),
                    MenuItem::CheckBox(
                        fl!("highlight-current-line"),
                        None,
                        config.highlight_current_line,
                        Action::ToggleHighlightCurrentLine,
                    ),
                    //TODO: MenuItem::CheckBox(fl!("syntax-highlighting"), Action::Todo),
                    MenuItem::Divider,
                    MenuItem::Button(fl!("menu-settings"), None, Action::ToggleSettingsPage),
                    //TODO MenuItem::Divider,
                    //TODO MenuItem::Button(fl!("menu-keyboard-shortcuts"), Action::Todo),
                    MenuItem::Divider,
                    MenuItem::Button(fl!("menu-about"), None, Action::About),
                ],
            ),
        ),
    ])
    .item_height(ItemHeight::Dynamic(40))
    .item_width(ItemWidth::Uniform(320))
    .spacing(4.0)
    .into()
}
