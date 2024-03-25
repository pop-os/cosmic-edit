// SPDX-License-Identifier: GPL-3.0-only

use cosmic::{
    cosmic_config::{self, cosmic_config_derive::CosmicConfigEntry, CosmicConfigEntry},
    theme,
};
use cosmic_text::Metrics;
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, path::PathBuf};

pub const CONFIG_VERSION: u64 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum AppTheme {
    Dark,
    Light,
    System,
}

impl AppTheme {
    pub fn theme(&self) -> theme::Theme {
        match self {
            Self::Dark => theme::Theme::dark(),
            Self::Light => theme::Theme::light(),
            Self::System => theme::system_preference(),
        }
    }
}

#[derive(Clone, CosmicConfigEntry, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Config {
    pub app_theme: AppTheme,
    pub auto_indent: bool,
    pub find_case_sensitive: bool,
    pub find_use_regex: bool,
    pub font_name: String,
    pub font_size: u16,
    pub highlight_current_line: bool,
    pub line_numbers: bool,
    pub syntax_theme_dark: String,
    pub syntax_theme_light: String,
    pub tab_width: u16,
    pub vim_bindings: bool,
    pub word_wrap: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            app_theme: AppTheme::System,
            auto_indent: true,
            find_case_sensitive: false,
            find_use_regex: false,
            font_name: "Fira Mono".to_string(),
            font_size: 14,
            highlight_current_line: true,
            line_numbers: true,
            syntax_theme_dark: "COSMIC Dark".to_string(),
            syntax_theme_light: "COSMIC Light".to_string(),
            tab_width: 4,
            vim_bindings: false,
            word_wrap: true,
        }
    }
}

impl Config {
    pub fn find_regex(&self, pattern: &str) -> Result<regex::Regex, regex::Error> {
        let mut builder = if self.find_use_regex {
            regex::RegexBuilder::new(pattern)
        } else {
            regex::RegexBuilder::new(&regex::escape(pattern))
        };
        builder.case_insensitive(!self.find_case_sensitive);
        builder.build()
    }

    // Calculate metrics from font size
    pub fn metrics(&self) -> Metrics {
        let font_size = self.font_size.max(1) as f32;
        let line_height = (font_size * 1.4).ceil();
        Metrics::new(font_size, line_height)
    }

    // Get current syntax theme based on dark mode
    pub fn syntax_theme(&self) -> &str {
        let dark = self.app_theme.theme().theme_type.is_dark();
        if dark {
            &self.syntax_theme_dark
        } else {
            &self.syntax_theme_light
        }
    }
}

#[derive(Clone, CosmicConfigEntry, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConfigState {
    pub recent_files: VecDeque<PathBuf>,
    pub recent_projects: VecDeque<PathBuf>,
}

impl Default for ConfigState {
    fn default() -> Self {
        Self {
            recent_files: VecDeque::new(),
            recent_projects: VecDeque::new(),
        }
    }
}
