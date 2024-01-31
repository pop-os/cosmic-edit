// SPDX-License-Identifier: GPL-3.0-only

use grep::matcher::{Match, Matcher};
use grep::regex::RegexMatcher;
use grep::searcher::{sinks::UTF8, Searcher};
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineSearchResult {
    pub number: usize,
    pub text: String,
    pub first: Match,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileSearchResult {
    pub path: PathBuf,
    pub lines: Vec<LineSearchResult>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectSearchResult {
    //TODO: should this be included?
    pub value: String,
    pub in_progress: bool,
    pub files: Vec<FileSearchResult>,
}

impl ProjectSearchResult {
    pub fn search_projects(&mut self, projects: Vec<(String, PathBuf)>) {
        //TODO: support literal search
        //TODO: use ignore::WalkParallel?
        match RegexMatcher::new(&self.value) {
            Ok(matcher) => {
                let mut searcher = Searcher::new();
                let mut walk_builder_opt: Option<ignore::WalkBuilder> = None;
                for (_, project_path) in projects.iter() {
                    walk_builder_opt = match walk_builder_opt.take() {
                        Some(mut walk_builder) => {
                            walk_builder.add(project_path);
                            Some(walk_builder)
                        }
                        None => Some(ignore::WalkBuilder::new(project_path)),
                    };
                }

                if let Some(walk_builder) = walk_builder_opt {
                    for entry_res in walk_builder.build() {
                        let entry = match entry_res {
                            Ok(ok) => ok,
                            Err(err) => {
                                log::error!("failed to walk projects {:?}: {}", projects, err);
                                continue;
                            }
                        };

                        if let Some(file_type) = entry.file_type() {
                            if file_type.is_dir() {
                                continue;
                            }
                        }

                        let entry_path = entry.path();

                        let mut lines = Vec::new();
                        match searcher.search_path(
                            &matcher,
                            entry_path,
                            UTF8(|number_u64, text| {
                                match usize::try_from(number_u64) {
                                    Ok(number) => match matcher.find(text.as_bytes()) {
                                        Ok(Some(first)) => {
                                            lines.push(LineSearchResult {
                                                number,
                                                text: text.to_string(),
                                                first,
                                            });
                                        },
                                        Ok(None) => {
                                            log::error!("first match in file {:?} line {} not found", entry_path, number);
                                        }
                                        Err(err) => {
                                            log::error!("failed to find first match in file {:?} line {}: {}", entry_path, number, err);
                                        }
                                    },
                                    Err(err) => {
                                        log::error!("failed to convert file {:?} line {} to usize: {}", entry_path, number_u64, err);
                                    }
                                }
                                Ok(true)
                            }),
                        ) {
                            Ok(()) => {
                                if !lines.is_empty() {
                                    self.files.push(FileSearchResult {
                                        path: entry_path.to_path_buf(),
                                        lines,
                                    });
                                }
                            }
                            Err(err) => {
                                log::error!("failed to search file {:?}: {}", entry_path, err);
                            }
                        }
                    }
                }
            }
            Err(err) => {
                log::error!(
                    "failed to create regex matcher with value {:?}: {}",
                    self.value,
                    err
                );
            }
        }
        self.in_progress = false;
    }
}
