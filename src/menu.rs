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

use crate::{Config, Message};

pub fn menu_bar<'a>(config: &Config) -> Element<'a, Message> {
    //TODO: port to libcosmic
    let menu_root = |label| {
        widget::button(label)
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

    let menu_folder = |label| menu_button!(label, horizontal_space(Length::Fill), ">");

    let menu_item = |label, message| MenuTree::new(menu_button!(label).on_press(message));

    let menu_key = |label, key, message| {
        MenuTree::new(menu_button!(label, horizontal_space(Length::Fill), key).on_press(message))
    };

    MenuBar::new(vec![
        MenuTree::with_children(
            menu_root("File"),
            vec![
                menu_key("New file", "Ctrl + N", Message::New),
                menu_key("New window", "Ctrl + Shift + N", Message::Todo),
                MenuTree::new(horizontal_rule(1)),
                menu_key("Open file...", "Ctrl + O", Message::OpenFileDialog),
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
    .item_height(ItemHeight::Dynamic(40))
    .item_width(ItemWidth::Uniform(240))
    .spacing(4.0)
    .into()
}
