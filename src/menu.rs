// SPDX-License-Identifier: GPL-3.0-only

use cosmic::{
    //TODO: export in cosmic::widget
    iced::{widget::horizontal_rule, Alignment, Length},
    theme,
    widget::{
        self, horizontal_space,
        menu::{ItemHeight, ItemWidth, MenuBar, MenuTree},
    },
    Element,
};

use crate::{fl, Config, ContextPage, Message};

pub fn menu_bar<'a>(config: &Config) -> Element<'a, Message> {
    //TODO: port to libcosmic
    let menu_root = |label| {
        widget::button(widget::text(label))
            .padding([4, 12])
            .style(theme::Button::MenuRoot)
    };

    macro_rules! menu_button {
        ($($x:expr),+ $(,)?) => (
            widget::button(
                widget::Row::with_children(
                    vec![$(Element::from($x)),+]
                )
                .align_items(Alignment::Center)
            )
            .height(Length::Fixed(32.0))
            .padding([4, 16])
            .width(Length::Fill)
            .style(theme::Button::MenuItem)
        );
    }

    let menu_folder =
        |label| menu_button!(widget::text(label), horizontal_space(Length::Fill), ">");

    let find_key = |message: &Message| -> String {
        let mut key = String::new();
        for (key_bind, action) in config.keybinds.iter() {
            if &action.message() == message {
                key = key_bind.to_string();
                break;
            }
        }
        key
    };

    let menu_item = |label, message| {
        let key = find_key(&message);
        MenuTree::new(
            menu_button!(
                widget::text(label),
                horizontal_space(Length::Fill),
                widget::text(key)
            )
            .on_press(message),
        )
    };

    //TODO: support key lookup?
    let menu_checkbox = |label, value, message| {
        let check: Element<_> = if value {
            widget::icon::from_name("object-select-symbolic")
                .size(16)
                .icon()
                .into()
        } else {
            widget::Space::with_width(Length::Fixed(16.0)).into()
        };
        let key = find_key(&message);
        MenuTree::new(
            menu_button!(
                check,
                widget::Space::with_width(Length::Fixed(8.0)),
                widget::text(label),
                horizontal_space(Length::Fill),
                widget::text(key)
            )
            .on_press(message),
        )
    };

    let menu_key = |label, key, message| {
        MenuTree::new(
            menu_button!(widget::text(label), horizontal_space(Length::Fill), key)
                .on_press(message),
        )
    };

    MenuBar::new(vec![
        MenuTree::with_children(
            menu_root(fl!("file")),
            vec![
                menu_item(fl!("new-file"), Message::NewFile),
                menu_item(fl!("new-window"), Message::NewWindow),
                MenuTree::new(horizontal_rule(1)),
                menu_item(fl!("open-file"), Message::OpenFileDialog),
                MenuTree::with_children(
                    menu_folder(fl!("open-recent-file")),
                    vec![menu_item(fl!("todo"), Message::Todo)],
                ),
                menu_item(fl!("close-file"), Message::CloseFile),
                MenuTree::new(horizontal_rule(1)),
                menu_item(fl!("menu-open-project"), Message::OpenProjectDialog),
                MenuTree::with_children(
                    menu_folder(fl!("open-recent-project")),
                    vec![menu_item(fl!("todo"), Message::Todo)],
                ),
                menu_item(fl!("close-project"), Message::CloseProject),
                MenuTree::new(horizontal_rule(1)),
                menu_item(fl!("save"), Message::Save),
                menu_key(fl!("save-as"), "Ctrl + Shift + S", Message::Todo),
                MenuTree::new(horizontal_rule(1)),
                menu_item(fl!("revert-all-changes"), Message::Todo),
                MenuTree::new(horizontal_rule(1)),
                menu_item(
                    fl!("menu-document-statistics"),
                    Message::ToggleContextPage(ContextPage::DocumentStatistics),
                ),
                menu_item(fl!("document-type"), Message::Todo),
                menu_item(fl!("encoding"), Message::Todo),
                menu_item(fl!("print"), Message::Todo),
                MenuTree::new(horizontal_rule(1)),
                menu_item(fl!("quit"), Message::Quit),
            ],
        ),
        MenuTree::with_children(
            menu_root(fl!("edit")),
            vec![
                menu_key(fl!("undo"), "Ctrl + Z", Message::Todo),
                menu_key(fl!("redo"), "Ctrl + Shift + Z", Message::Todo),
                MenuTree::new(horizontal_rule(1)),
                menu_item(fl!("cut"), Message::Cut),
                menu_item(fl!("copy"), Message::Copy),
                menu_item(fl!("paste"), Message::Paste),
                menu_item(fl!("select-all"), Message::SelectAll),
                MenuTree::new(horizontal_rule(1)),
                menu_key(fl!("find"), "Ctrl + F", Message::Todo),
                menu_key(fl!("replace"), "Ctrl + H", Message::Todo),
                MenuTree::new(horizontal_rule(1)),
                menu_item(fl!("spell-check"), Message::Todo),
            ],
        ),
        MenuTree::with_children(
            menu_root(fl!("view")),
            vec![
                MenuTree::with_children(
                    menu_folder(fl!("indentation")),
                    vec![
                        menu_item(fl!("automatic-indentation"), Message::Todo),
                        MenuTree::new(horizontal_rule(1)),
                        menu_item(fl!("tab-width", tab_width = 1), Message::Todo),
                        menu_item(fl!("tab-width", tab_width = 2), Message::Todo),
                        menu_item(fl!("tab-width", tab_width = 4), Message::Todo),
                        menu_item(fl!("tab-width", tab_width = 8), Message::Todo),
                        MenuTree::new(horizontal_rule(1)),
                        menu_item(fl!("convert-indentation-to-spaces"), Message::Todo),
                        menu_item(fl!("convert-indentation-to-tabs"), Message::Todo),
                    ],
                ),
                MenuTree::new(horizontal_rule(1)),
                menu_checkbox(fl!("word-wrap"), config.word_wrap, Message::ToggleWordWrap),
                menu_checkbox(fl!("show-line-numbers"), false, Message::Todo),
                menu_checkbox(fl!("highlight-current-line"), false, Message::Todo),
                menu_item(fl!("syntax-highlighting"), Message::Todo),
                MenuTree::new(horizontal_rule(1)),
                menu_item(
                    fl!("menu-settings"),
                    Message::ToggleContextPage(ContextPage::Settings),
                ),
                MenuTree::new(horizontal_rule(1)),
                menu_item(fl!("menu-keyboard-shortcuts"), Message::Todo),
                MenuTree::new(horizontal_rule(1)),
                menu_item(fl!("about-cosmic-text-editor"), Message::Todo),
            ],
        ),
    ])
    .item_height(ItemHeight::Dynamic(40))
    .item_width(ItemWidth::Uniform(240))
    .spacing(4.0)
    .into()
}
