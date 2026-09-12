// SPDX-License-Identifier: GPL-3.0-only

//! Session restoration and hot exit recovery.

use crate::backup::{
    self, CachedDocument, CachedDocumentMeta, atomic_write, backup_file_path, compute_content_hash,
    read_backup_by_id, remove_backup_file, remove_session_backups, write_backup_file,
};
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fmt, fs,
    io::{Seek, Write},
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RestoreOption {
    DiscardAll,
    #[default]
    RestoreFirstN,
    RestoreAll,
}

#[derive(Debug)]
pub enum HotExitError {
    NoCacheDir,
    SessionLocked,
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for HotExitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCacheDir => write!(f, "cache directory unavailable"),
            Self::SessionLocked => write!(f, "session is locked by another process"),
            Self::Io(e) => write!(f, "I/O error: {}", e),
            Self::Json(e) => write!(f, "JSON error: {}", e),
        }
    }
}

impl std::error::Error for HotExitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NoCacheDir | Self::SessionLocked => None,
            Self::Io(e) => Some(e),
            Self::Json(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for HotExitError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<serde_json::Error> for HotExitError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct SessionTabView {
    pub cursor_line: usize,
    pub cursor_index: usize,
    pub scroll_line: usize,
    pub scroll_vertical: f32,
    pub scroll_horizontal: f32,
    pub zoom_adj: i8,
    pub content_hash: Option<u64>,
    pub scroll_anchor: Option<u64>,
    pub cursor_anchor: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SessionTab {
    pub path: Option<PathBuf>,
    pub has_unsaved_changes: bool,
    pub backup_id: Option<String>,
    #[serde(default)]
    pub view: Option<SessionTabView>,
}

impl SessionTab {
    /// Build a restorable session entry from the current editor state.
    ///
    /// When unsaved changes are being discarded, named files are retained so
    /// they can be reopened from disk, while untitled documents are omitted.
    pub fn from_editor_state(
        path: Option<PathBuf>,
        changed: bool,
        backup_id: Option<String>,
        preserve_unsaved_changes: bool,
        view: SessionTabView,
    ) -> Option<Self> {
        let has_unsaved_changes = changed && preserve_unsaved_changes && backup_id.is_some();
        if path.is_none() && !has_unsaved_changes {
            return None;
        }

        Some(Self {
            path,
            has_unsaved_changes,
            backup_id: backup_id.filter(|_| has_unsaved_changes),
            view: Some(view),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionProject {
    pub path: PathBuf,
    pub expanded_folders: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct SessionState {
    pub tabs: Vec<SessionTab>,
    #[serde(default)]
    pub projects: Vec<SessionProject>,
    pub active_tab: usize,
    #[serde(default)]
    pub active_project_path: Option<PathBuf>,
}

pub fn should_persist_snapshot(previous: Option<&SessionState>, current: &SessionState) -> bool {
    previous != Some(current)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GracefulClosePolicy {
    Prompt,
    HotExit,
    CleanExit,
}

pub fn graceful_close_policy(
    recovery_enabled: bool,
    has_other_live_windows: bool,
    has_unsaved_changes: bool,
) -> GracefulClosePolicy {
    if has_unsaved_changes && (!recovery_enabled || has_other_live_windows) {
        GracefulClosePolicy::Prompt
    } else if recovery_enabled && !has_other_live_windows {
        GracefulClosePolicy::HotExit
    } else {
        GracefulClosePolicy::CleanExit
    }
}

pub fn generate_session_id() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    let pid = std::process::id() as u64;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or_default();
    let random = RandomState::new().build_hasher().finish();

    timestamp ^ pid.rotate_left(32) ^ random
}

fn sessions_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("cosmic-edit").join("sessions"))
}

pub fn session_file(session_id: u64) -> Option<PathBuf> {
    sessions_dir().map(|d| d.join(format!("{:016x}.json", session_id)))
}

fn lock_file(session_id: u64) -> Option<PathBuf> {
    sessions_dir().map(|d| d.join(format!("{:016x}.lock", session_id)))
}

fn open_lifecycle_lock() -> Result<fs::File, HotExitError> {
    let sessions_path = sessions_dir().ok_or(HotExitError::NoCacheDir)?;
    fs::create_dir_all(&sessions_path)?;
    Ok(fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(sessions_path.join("lifecycle.lock"))?)
}

fn try_lock_exclusive(file: &fs::File) -> std::io::Result<bool> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(fs::TryLockError::WouldBlock) => Ok(false),
        Err(fs::TryLockError::Error(error)) => Err(error),
    }
}

pub fn acquire_lifecycle_lock() -> Result<fs::File, HotExitError> {
    let file = open_lifecycle_lock()?;
    file.lock()?;
    Ok(file)
}

pub fn save_session(session_id: u64, state: &SessionState) -> Result<(), HotExitError> {
    let path = session_file(session_id).ok_or(HotExitError::NoCacheDir)?;

    let json = serde_json::to_vec_pretty(state)?;
    atomic_write(&path, &json)?;
    debug!(
        "hotexit: saved session {:016x} with {} tabs, {} projects",
        session_id,
        state.tabs.len(),
        state.projects.len()
    );
    Ok(())
}

pub fn load_session(session_id: u64) -> Result<SessionState, HotExitError> {
    let path = session_file(session_id).ok_or(HotExitError::NoCacheDir)?;
    let json = fs::read_to_string(&path)?;
    let mut state: SessionState = serde_json::from_str(&json)?;

    // Enforce session limits to prevent DoS from malicious session files
    if state.tabs.len() > MAX_TABS_PER_SESSION {
        warn!(
            "hotexit: truncating tabs from {} to {} (limit)",
            state.tabs.len(),
            MAX_TABS_PER_SESSION
        );
        state.tabs.truncate(MAX_TABS_PER_SESSION);
    }
    if state.projects.len() > MAX_PROJECTS_PER_SESSION {
        warn!(
            "hotexit: truncating projects from {} to {} (limit)",
            state.projects.len(),
            MAX_PROJECTS_PER_SESSION
        );
        state.projects.truncate(MAX_PROJECTS_PER_SESSION);
    }

    if state.active_tab >= state.tabs.len() {
        warn!(
            "hotexit: active_tab {} out of bounds (tabs: {}), resetting to 0",
            state.active_tab,
            state.tabs.len()
        );
        state.active_tab = 0;
    }

    Ok(state)
}

const MAX_TABS_PER_SESSION: usize = 1000;

const MAX_PROJECTS_PER_SESSION: usize = 100;

fn remove_file_if_exists(path: &std::path::Path, file_type: &str) {
    match fs::remove_file(path) {
        Ok(()) => {
            debug!("hotexit: removed {} {:?}", file_type, path);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            warn!("hotexit: failed to remove {} {:?}: {}", file_type, path, e);
        }
    }
}

fn session_locks() -> &'static Mutex<HashMap<u64, fs::File>> {
    static LOCKS: OnceLock<Mutex<HashMap<u64, fs::File>>> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn create_session_lock(session_id: u64) -> Result<(), HotExitError> {
    try_acquire_session_lock(session_id)
}

pub fn release_session_lock(session_id: u64) {
    let file = session_locks().lock().unwrap().remove(&session_id);
    if let Some(file) = file {
        let _ = file.unlock();
    }
}

pub fn release_and_cleanup_session(session_id: u64) {
    remove_session_state(session_id);
    remove_session_backups(session_id);
    release_session_lock(session_id);
}

pub fn remove_session_state(session_id: u64) {
    if let Some(session_path) = session_file(session_id) {
        remove_file_if_exists(&session_path, "session file");
    }
}

pub fn is_session_active(session_id: u64) -> bool {
    if session_locks().lock().unwrap().contains_key(&session_id) {
        return true;
    }

    let Some(lock_path) = lock_file(session_id) else {
        return false;
    };
    let file = match fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock_path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
        Err(error) => {
            warn!("hotexit: failed to open session lock: {error}");
            return true;
        }
    };

    match try_lock_exclusive(&file) {
        Ok(true) => {
            let _ = file.unlock();
            false
        }
        Ok(false) => true,
        Err(error) => {
            warn!("hotexit: failed to inspect session lock: {error}");
            true
        }
    }
}

pub fn has_other_live_sessions(session_id: u64) -> Result<bool, HotExitError> {
    let sessions_path = sessions_dir().ok_or(HotExitError::NoCacheDir)?;
    let entries = match fs::read_dir(sessions_path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };

    let mut peer_session_ids = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_none_or(|extension| extension != "lock") {
            continue;
        }
        let Some(peer_session_id) = extract_session_id_from_path(&path) else {
            continue;
        };
        peer_session_ids.push(peer_session_id);
    }

    Ok(has_other_live_sessions_for(session_id, peer_session_ids))
}

fn has_other_live_sessions_for(
    session_id: u64,
    session_ids: impl IntoIterator<Item = u64>,
) -> bool {
    session_ids
        .into_iter()
        .any(|peer_session_id| peer_session_id != session_id && is_session_active(peer_session_id))
}

pub fn try_acquire_session_lock(session_id: u64) -> Result<(), HotExitError> {
    if session_locks().lock().unwrap().contains_key(&session_id) {
        return Ok(());
    }

    let lock_path = lock_file(session_id).ok_or(HotExitError::NoCacheDir)?;
    fs::create_dir_all(sessions_dir().ok_or(HotExitError::NoCacheDir)?)?;
    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    if !try_lock_exclusive(&file)? {
        return Err(HotExitError::SessionLocked);
    }
    file.set_len(0)?;
    file.rewind()?;
    writeln!(file, "{}", std::process::id())?;
    file.sync_all()?;
    session_locks().lock().unwrap().insert(session_id, file);
    debug!("hotexit: acquired lock for session {:016x}", session_id);
    Ok(())
}

fn try_lock_session_for_cleanup(session_id: u64) -> Result<Option<fs::File>, HotExitError> {
    if session_locks().lock().unwrap().contains_key(&session_id) {
        return Ok(None);
    }

    let lock_path = lock_file(session_id).ok_or(HotExitError::NoCacheDir)?;
    fs::create_dir_all(sessions_dir().ok_or(HotExitError::NoCacheDir)?)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    match try_lock_exclusive(&file)? {
        true => Ok(Some(file)),
        false => Ok(None),
    }
}

pub fn adopt_session(session_id: u64) -> bool {
    let session_path = match session_file(session_id) {
        Some(p) if p.exists() => p,
        _ => {
            warn!(
                "hotexit: cannot adopt session {:016x} - session file not found",
                session_id
            );
            return false;
        }
    };

    match try_acquire_session_lock(session_id) {
        Ok(()) => {
            info!(
                "hotexit: adopted session {:016x} (session file: {:?})",
                session_id, session_path
            );
            true
        }
        Err(HotExitError::SessionLocked) => {
            warn!(
                "hotexit: cannot adopt session {:016x} - locked by another process",
                session_id
            );
            false
        }
        Err(e) => {
            warn!(
                "hotexit: failed to adopt session {:016x}: {}",
                session_id, e
            );
            false
        }
    }
}

pub fn find_all_orphaned_sessions() -> Vec<(u64, SessionState)> {
    let sessions_path = match sessions_dir() {
        Some(p) => p,
        None => return Vec::new(),
    };

    let _ = fs::create_dir_all(&sessions_path);

    let entries = match fs::read_dir(&sessions_path) {
        Ok(e) => e,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                warn!(
                    "hotexit: failed to read sessions directory {:?}: {}",
                    sessions_path, e
                );
            }
            return Vec::new();
        }
    };

    let mut orphaned = Vec::new();

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();

        let is_json = path.extension().map(|e| e == "json").unwrap_or(false);
        if !is_json {
            continue;
        }

        let session_id = match extract_session_id_from_path(&path) {
            Some(id) => id,
            None => continue,
        };

        if is_session_active(session_id) {
            debug!("hotexit: session {:016x} is active, skipping", session_id);
            continue;
        }

        match load_session(session_id) {
            Ok(state) => {
                info!("hotexit: found orphaned session {:016x}", session_id);
                let modified = entry
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .unwrap_or(UNIX_EPOCH);
                orphaned.push((modified, session_id, state));
            }
            Err(e) => {
                warn!("hotexit: failed to load session {:016x}: {}", session_id, e);
            }
        }
    }

    orphaned.sort_by_key(|(modified, session_id, _)| (*modified, *session_id));
    orphaned
        .into_iter()
        .map(|(_, session_id, state)| (session_id, state))
        .collect()
}

pub fn discard_session(session_id: u64) {
    if try_acquire_session_lock(session_id).is_err() {
        return;
    }
    if let Some(session_path) = session_file(session_id) {
        remove_file_if_exists(&session_path, "orphaned session file");
    }
    remove_session_backups(session_id);
    release_session_lock(session_id);
}

fn extract_session_id_from_path(path: &std::path::Path) -> Option<u64> {
    path.file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| u64::from_str_radix(s, 16).ok())
}

pub fn cleanup_stale_backups() {
    cleanup_stale_backups_for(backup::backup_session_ids());
}

fn path_exists_or_unknown(path: &std::path::Path) -> bool {
    match fs::metadata(path) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            warn!("hotexit: preserving state after metadata error for {path:?}: {error}");
            true
        }
    }
}

fn cleanup_stale_backups_for(session_ids: impl IntoIterator<Item = u64>) {
    for session_id in session_ids {
        let cleanup_lock = match try_lock_session_for_cleanup(session_id) {
            Ok(Some(file)) => file,
            Ok(None) => continue,
            Err(error) => {
                warn!("hotexit: preserving backups after lock error: {error}");
                continue;
            }
        };

        if session_file(session_id).is_some_and(|path| path_exists_or_unknown(&path)) {
            if let Ok(state) = load_session(session_id) {
                let referenced_ids = state
                    .tabs
                    .into_iter()
                    .filter_map(|tab| tab.backup_id)
                    .collect();
                backup::remove_unreferenced_session_backups(session_id, &referenced_ids);
            }
        } else {
            backup::remove_session_backups(session_id);
        }
        let _ = cleanup_lock.unlock();
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RestoreDialogState {
    pub total_sessions: usize,
    pub max_auto_restore: usize,
    pub sessions_to_restore: usize,
    pub sessions_to_discard: usize,
}

impl RestoreDialogState {
    pub fn new(
        total_sessions: usize,
        max_auto_restore: usize,
        selected_option: RestoreOption,
    ) -> Self {
        let sessions_to_restore = match selected_option {
            RestoreOption::DiscardAll => 0,
            RestoreOption::RestoreAll => total_sessions,
            RestoreOption::RestoreFirstN => max_auto_restore.min(total_sessions),
        };
        Self {
            total_sessions,
            max_auto_restore,
            sessions_to_restore,
            sessions_to_discard: total_sessions - sessions_to_restore,
        }
    }

    pub fn restore_n_count(&self) -> usize {
        self.max_auto_restore.min(self.total_sessions)
    }
}

pub fn split_restore_sessions(
    mut session_ids: Vec<u64>,
    limit: Option<usize>,
) -> (Vec<u64>, Vec<u64>) {
    let restore_count = limit.unwrap_or(session_ids.len()).min(session_ids.len());
    let discard = session_ids.split_off(restore_count);
    (session_ids, discard)
}

pub enum StartupAction {
    Normal,
    RestoreSingle(u64, SessionState),
    RestoreMultiple {
        first_session: (u64, SessionState),
        spawn_sessions: Vec<u64>,
    },
    PromptUser(Vec<u64>),
}

pub fn determine_startup_action(reopen_on_start: bool, max_auto_restore: usize) -> StartupAction {
    if !reopen_on_start {
        debug!("hotexit: reopen_on_start disabled, skipping session restore");
        return StartupAction::Normal;
    }

    let orphaned = find_all_orphaned_sessions();

    if orphaned.is_empty() {
        debug!("hotexit: no orphaned sessions found");
        return StartupAction::Normal;
    }

    if orphaned.len() > max_auto_restore {
        info!(
            "hotexit: found {} orphaned sessions, prompting user",
            orphaned.len()
        );
        return StartupAction::PromptUser(orphaned.into_iter().map(|(id, _)| id).collect());
    }

    let mut iter = orphaned.into_iter();
    let first_session = iter.next().expect("orphaned sessions are not empty");
    let spawn_sessions: Vec<u64> = iter.map(|(id, _)| id).collect();
    if spawn_sessions.is_empty() {
        StartupAction::RestoreSingle(first_session.0, first_session.1)
    } else {
        StartupAction::RestoreMultiple {
            first_session,
            spawn_sessions,
        }
    }
}

pub enum RestoredTab {
    FromBackup {
        path_opt: Option<PathBuf>,
        content: String,
        view: SessionTabView,
        backup_id: String,
        content_hash: u64,
    },
    FromFile {
        path: PathBuf,
        view: Option<SessionTabView>,
    },
}

pub fn load_session_tabs(session_id: u64, session: &SessionState) -> Vec<RestoredTab> {
    let mut tabs = Vec::new();

    for session_tab in &session.tabs {
        if session_tab.has_unsaved_changes
            && let Some(backup_id) = &session_tab.backup_id
            && let Some(cached_doc) = read_backup_by_id(backup_id)
        {
            let CachedDocument { meta, content } = cached_doc;
            if meta.id != *backup_id || meta.session_id != session_id {
                warn!("hotexit: ignoring mismatched backup {backup_id}");
            } else {
                info!(
                    "hotexit: restoring tab from backup {} (path={:?})",
                    meta.id, session_tab.path
                );
                let content_hash = compute_content_hash(&content);
                let view = session_tab.view.clone().unwrap_or(SessionTabView {
                    cursor_line: meta.cursor_line,
                    cursor_index: meta.cursor_index,
                    zoom_adj: meta.zoom_adj,
                    content_hash: Some(content_hash),
                    ..SessionTabView::default()
                });
                tabs.push(RestoredTab::FromBackup {
                    path_opt: session_tab.path.clone(),
                    content,
                    view,
                    backup_id: meta.id,
                    content_hash,
                });
                continue;
            }
        }
        if session_tab.has_unsaved_changes {
            warn!(
                "hotexit: backup not found for tab (id={:?}, path={:?}), falling back to file",
                session_tab.backup_id, session_tab.path
            );
        }

        if let Some(path) = &session_tab.path
            && path.exists()
        {
            tabs.push(RestoredTab::FromFile {
                path: path.clone(),
                view: session_tab.view.clone(),
            });
        }
    }

    tabs
}

pub struct BackupRequest {
    pub path_opt: Option<PathBuf>,
    pub content: String,
    pub cursor_line: usize,
    pub cursor_index: usize,
    pub zoom_adj: i8,
    pub existing_backup_id: Option<String>,
}

pub fn backup_content_hash(session_id: u64, backup_id: &str) -> Option<u64> {
    let cached_doc = read_backup_by_id(backup_id)?;
    if cached_doc.meta.id != backup_id || cached_doc.meta.session_id != session_id {
        return None;
    }
    Some(compute_content_hash(&cached_doc.content))
}

fn write_tab_backup_inner(
    session_id: u64,
    request: BackupRequest,
    new_generation: bool,
) -> Result<String, HotExitError> {
    let generated_id = || {
        let doc_id = request.path_opt.as_ref().map_or_else(
            || format!("untitled_{:016x}", generate_session_id()),
            |path| format!("{:016x}", compute_content_hash(&path.to_string_lossy())),
        );
        if new_generation {
            format!("{session_id:016x}_{doc_id}_{:016x}", generate_session_id())
        } else {
            format!("{session_id:016x}_{doc_id}")
        }
    };
    let backup_id = if new_generation {
        generated_id()
    } else {
        request
            .existing_backup_id
            .clone()
            .unwrap_or_else(generated_id)
    };

    let meta = CachedDocumentMeta {
        id: backup_id.clone(),
        session_id,
        cursor_line: request.cursor_line,
        cursor_index: request.cursor_index,
        zoom_adj: request.zoom_adj,
    };
    let doc = CachedDocument {
        meta,
        content: request.content,
    };
    let backup_path = backup_file_path(&backup_id)?;

    write_backup_file(&backup_path, &doc)?;
    Ok(backup_id)
}

pub fn write_tab_backup(session_id: u64, request: BackupRequest) -> Result<String, HotExitError> {
    write_tab_backup_inner(session_id, request, false)
}

pub fn write_tab_backup_generation(
    session_id: u64,
    request: BackupRequest,
) -> Result<String, HotExitError> {
    write_tab_backup_inner(session_id, request, true)
}

pub fn cleanup_backup_after_save(backup_id: Option<String>) {
    if let Some(id) = backup_id
        && let Err(e) = remove_backup_file(&id)
    {
        warn!("hotexit: failed to remove backup {}: {}", id, e);
    }
}

pub fn spawn_restore_instance(session_id: u64) {
    match std::env::current_exe() {
        Ok(exe) => {
            let arg = format!("--restore-session={:016x}", session_id);
            match std::process::Command::new(&exe).arg(&arg).spawn() {
                Ok(_child) => {
                    info!(
                        "hotexit: spawned instance to restore session {:016x}",
                        session_id
                    );
                }
                Err(err) => {
                    log::error!("hotexit: failed to spawn restore instance: {}", err);
                }
            }
        }
        Err(err) => {
            log::error!("hotexit: failed to get current executable path: {}", err);
        }
    }
}
