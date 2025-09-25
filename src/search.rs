// SPDX-License-Identifier: GPL-3.0-only

use grep::matcher::{Match, Matcher};
use grep::regex::RegexMatcher;
use grep::searcher::{Searcher, sinks::UTF8};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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

// Buffer to accumulate results per worker and merge once at the end.
struct WorkerBuffer {
    shared: Arc<Mutex<Vec<FileSearchResult>>>,
    local: Vec<FileSearchResult>,
}

impl WorkerBuffer {
    fn new(shared: Arc<Mutex<Vec<FileSearchResult>>>) -> Self {
        Self {
            shared,
            local: Vec::with_capacity(256),
        }
    }
}

impl Drop for WorkerBuffer {
    fn drop(&mut self) {
        if self.local.is_empty() {
            return;
        }
        if let Ok(mut shared_vec) = self.shared.lock() {
            shared_vec.extend(self.local.drain(..));
        }
    }
}

impl ProjectSearchResult {
    pub fn search_projects(&mut self, projects: Vec<(String, PathBuf)>) {
        // Build a single matcher up front. Clone or Arc it into workers.
        match RegexMatcher::new(&self.value) {
            Ok(matcher) => {
                // Collect walk roots (deduplicated)
                let mut walk_builder_opt: Option<ignore::WalkBuilder> = None;
                {
                    use std::collections::HashSet;
                    let mut uniq: HashSet<PathBuf> = HashSet::new();
                    for (_, project_path) in projects.iter() {
                        uniq.insert(project_path.clone());
                    }
                    for project_path in uniq.into_iter() {
                        walk_builder_opt = match walk_builder_opt.take() {
                            Some(mut walk_builder) => {
                                walk_builder.add(&project_path);
                                Some(walk_builder)
                            }
                            None => Some(ignore::WalkBuilder::new(&project_path)),
                        };
                    }
                }

                // Share matcher between workers
                let matcher = Arc::new(matcher);

                if let Some(mut walk_builder) = walk_builder_opt {
                    // Align walker flags with estimator/search
                    walk_builder
                        .git_ignore(true)
                        .git_global(true)
                        .git_exclude(true)
                        .follow_links(false);
                    // Tune threads to available parallelism
                    let threads = std::thread::available_parallelism()
                        .map(|n| n.get())
                        .unwrap_or(1);
                    walk_builder.threads(threads);

                    // Shared results collected via per-worker buffering
                    let shared_results: Arc<Mutex<Vec<FileSearchResult>>> =
                        Arc::new(Mutex::new(Vec::new()));
                    let walker = walk_builder.build_parallel();
                    let matcher_outer = matcher.clone();
                    walker.run(|| {
                        // One Searcher and buffer per worker
                        let mut searcher = Searcher::new();
                        let matcher = matcher_outer.clone();
                        let mut buffer = WorkerBuffer::new(shared_results.clone());
                        Box::new(move |entry_res| {
                            match entry_res {
                                Ok(entry) => {
                                    if let Some(file_type) = entry.file_type() {
                                        if file_type.is_dir() {
                                            return ignore::WalkState::Continue;
                                        }
                                    }

                                    let entry_path = entry.path().to_path_buf();
                                    let mut lines: Vec<LineSearchResult> = Vec::new();

                                    match searcher.search_path(
                                        &*matcher,
                                        &entry_path,
                                        UTF8(|number_u64, text| {
                                            match usize::try_from(number_u64) {
                                                Ok(number) => match matcher.find(text.as_bytes()) {
                                                    Ok(Some(first)) => {
                                                        lines.push(LineSearchResult {
                                                            number,
                                                            text: text.to_string(),
                                                            first,
                                                        });
                                                    }
                                                    Ok(None) => {
                                                        log::error!(
                                                            "first match in file {:?} line {} not found",
                                                            entry_path, number
                                                        );
                                                    }
                                                    Err(err) => {
                                                        log::error!(
                                                            "failed to find first match in file {:?} line {}: {}",
                                                            entry_path, number, err
                                                        );
                                                    }
                                                },
                                                Err(err) => {
                                                    log::error!(
                                                        "failed to convert file {:?} line {} to usize: {}",
                                                        entry_path, number_u64, err
                                                    );
                                                }
                                            }
                                            Ok(true)
                                        }),
                                    ) {
                                        Ok(()) => {
                                            if !lines.is_empty() {
                                                // Buffer result locally; merged once at worker end
                                                buffer.local.push(FileSearchResult { path: entry_path, lines });
                                            }
                                        }
                                        Err(err) => {
                                            log::error!("failed to search file {:?}: {}", entry_path, err);
                                        }
                                    }
                                }
                                Err(err) => {
                                    log::error!("failed to walk project entry: {}", err);
                                }
                            }
                            ignore::WalkState::Continue
                        })
                    });

                    // Replace existing results with merged contents
                    self.files.clear();
                    let merged: Vec<FileSearchResult> = match shared_results.lock() {
                        Ok(guard) => guard.clone(),
                        Err(poisoned) => poisoned.into_inner().clone(),
                    };
                    self.files.extend(merged.into_iter());
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
