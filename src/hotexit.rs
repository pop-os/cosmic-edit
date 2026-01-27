// SPDX-License-Identifier: GPL-3.0-only

//! Hot exit functionality for preserving unsaved work across crashes and unexpected closures.
//!
//! This module provides:
//! - Session management: Tracking open tabs and projects for restoration
//! - Orphan detection: Finding and restoring sessions from crashed instances
//!
//! For document backup operations, see the `backup` module.

use crate::backup;
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::{
    fmt, fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

// Re-export backup types and functions for backward compatibility
pub use backup::{
    CachedDocument, CachedDocumentMeta, backup_dir, cleanup_temp_files, compute_content_hash,
    generate_doc_id, make_backup_id, read_backup_by_id, remove_backup_file, remove_session_backups,
    write_backup_file,
};

// ============================================================================
// Restore options
// ============================================================================

/// Options for restoring orphaned sessions on startup.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RestoreOption {
    /// Discard all orphaned sessions and their unsaved changes.
    DiscardAll,
    /// Restore first N sessions (up to hot_exit_max_auto_restore), discard the rest.
    #[default]
    RestoreFirstN,
    /// Restore all orphaned sessions in separate windows.
    RestoreAll,
}

// ============================================================================
// Error types
// ============================================================================

/// Errors that can occur during hot exit operations
#[derive(Debug)]
pub enum HotExitError {
    /// Cache directory is not available
    NoCacheDir,
    /// Session is locked by another process
    SessionLocked,
    /// I/O error during file operations
    Io(std::io::Error),
    /// JSON serialization/deserialization error
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

// ============================================================================
// Session types
// ============================================================================

/// A tab entry in the session state
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionTab {
    /// File path (None for untitled documents)
    pub path: Option<PathBuf>,
    /// Whether this tab has unsaved changes (will be restored from backup cache)
    pub has_unsaved_changes: bool,
    /// Backup ID for tabs with unsaved changes
    pub backup_id: Option<String>,
}

/// A project entry in the session state
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionProject {
    /// Root path of the project
    pub path: PathBuf,
    /// Paths of expanded folders within the project (relative to project root)
    pub expanded_folders: Vec<PathBuf>,
}

/// Session state for remembering open tabs and projects
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionState {
    /// List of open tabs (in order)
    pub tabs: Vec<SessionTab>,
    /// List of open projects with expanded folder state
    #[serde(default)]
    pub projects: Vec<SessionProject>,
    /// Index of the active tab
    pub active_tab: usize,
    /// Active item in the project nav (file or folder path)
    #[serde(default)]
    pub active_project_path: Option<PathBuf>,
}

// ============================================================================
// ID generation
// ============================================================================

/// Generate a unique session ID based on current time, process ID, and randomness.
///
/// Uses time in nanoseconds combined with PID and a random component to create a
/// unique identifier. This prevents collisions even in the unlikely case of two
/// processes starting at the exact same nanosecond with the same PID (due to PID reuse).
///
/// The mixing constants are derived from the golden ratio (phi) and are commonly
/// used in hash functions to provide good bit distribution:
/// - `0x9e3779b97f4a7c15`: 2^64 / phi, used in many hash functions
/// - `0x517cc1b727220a95`: Another prime-derived constant for mixing
pub fn generate_session_id() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    // Mixing constants for good bit distribution (derived from golden ratio)
    const PHI_MIX: u64 = 0x9e3779b97f4a7c15;
    const SECONDARY_MIX: u64 = 0x517cc1b727220a95;

    let pid = std::process::id() as u64;
    let time_component = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        // Fallback: use PID mixed with constant to avoid collision with other fallbacks
        .unwrap_or_else(|_| pid.wrapping_mul(PHI_MIX));

    // Add random component to prevent collisions from PID reuse at same nanosecond
    let random_component = RandomState::new().build_hasher().finish();

    time_component
        .wrapping_add(pid.wrapping_mul(SECONDARY_MIX))
        .wrapping_add(random_component)
}

// ============================================================================
// Directory paths
// ============================================================================

/// Get the sessions directory path
fn sessions_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("cosmic-edit").join("sessions"))
}

/// Get the session file path for a specific session ID
pub fn session_file(session_id: u64) -> Option<PathBuf> {
    sessions_dir().map(|d| d.join(format!("{:016x}.json", session_id)))
}

/// Get the lock file path for a specific session ID
fn lock_file(session_id: u64) -> Option<PathBuf> {
    sessions_dir().map(|d| d.join(format!("{:016x}.lock", session_id)))
}

// ============================================================================
// Session file operations
// ============================================================================

/// Save session state to file for a specific session ID
pub fn save_session(session_id: u64, state: &SessionState) -> Result<(), HotExitError> {
    let path = session_file(session_id).ok_or(HotExitError::NoCacheDir)?;

    // Create directory if needed
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(state)?;
    fs::write(&path, json)?;
    debug!(
        "hotexit: saved session {:016x} with {} tabs, {} projects",
        session_id,
        state.tabs.len(),
        state.projects.len()
    );
    Ok(())
}

/// Load a specific session by ID
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

    // Validate active_tab is within bounds to prevent panics on restore.
    // This also handles the case where tabs is empty and active_tab > 0.
    if state.active_tab >= state.tabs.len() && !state.tabs.is_empty() {
        warn!(
            "hotexit: active_tab {} out of bounds (tabs: {}), resetting to 0",
            state.active_tab,
            state.tabs.len()
        );
        state.active_tab = 0;
    } else if state.tabs.is_empty() {
        state.active_tab = 0;
    }

    Ok(state)
}

// ============================================================================
// Session limits
// ============================================================================

/// Maximum number of tabs per session to prevent DoS from malicious session files
const MAX_TABS_PER_SESSION: usize = 1000;

/// Maximum number of projects per session
const MAX_PROJECTS_PER_SESSION: usize = 100;

// ============================================================================
// File removal helpers
// ============================================================================

/// Remove a file, treating NotFound as success. Logs at appropriate level.
fn remove_file_if_exists(path: &std::path::Path, file_type: &str) {
    match fs::remove_file(path) {
        Ok(()) => {
            debug!("hotexit: removed {} {:?}", file_type, path);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // File already gone, that's fine
        }
        Err(e) => {
            warn!("hotexit: failed to remove {} {:?}: {}", file_type, path, e);
        }
    }
}

// ============================================================================
// Session lock operations
// ============================================================================

/// Check if a process is running.
///
/// # Unix Implementation
/// Uses `kill(pid, 0)` which checks if the process exists without sending a signal.
/// Returns true if the process exists (even if we lack permission to signal it).
///
/// # Windows Implementation
/// Uses `OpenProcess()` and `GetExitCodeProcess()` from the Windows API to check
/// if the process is still running (exit code is STILL_ACTIVE).
#[cfg(unix)]
fn is_process_running(pid: u32) -> bool {
    // kill(pid, 0) returns 0 if process exists and we have permission to signal it
    // Returns -1 with ESRCH if process doesn't exist
    // Returns -1 with EPERM if process exists but we lack permission (still means it exists!)
    //
    // SAFETY: kill(pid, 0) with signal 0 only checks process existence without
    // sending a signal. This is safe for any PID value - invalid PIDs simply
    // return an error which we handle below.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if result == 0 {
        true
    } else {
        // Check errno - EPERM means process exists but we can't signal it
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

#[cfg(windows)]
fn is_process_running(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // SAFETY: OpenProcess with PROCESS_QUERY_LIMITED_INFORMATION is safe for any PID.
    // If the PID is invalid or we lack permission, it returns a null handle.
    // CloseHandle is safe when passed a valid handle from OpenProcess.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }

        let mut exit_code: u32 = 0;
        let result = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);

        // Process is running if we got the exit code and it's STILL_ACTIVE
        result != 0 && exit_code == STILL_ACTIVE
    }
}

#[cfg(not(any(unix, windows)))]
fn is_process_running(_pid: u32) -> bool {
    // Conservative approach for other platforms: assume process is not running.
    // This may cause extra "restore session" prompts but no data loss.
    false
}

/// Read PID from a lock file, returning None if file doesn't exist or is invalid.
fn read_lock_pid(lock_path: &std::path::Path) -> Option<u32> {
    fs::read_to_string(lock_path)
        .ok()
        .and_then(|content| content.trim().parse().ok())
}

/// Create a lock for our session.
/// Returns Ok(()) on success, Err on failure.
pub fn create_session_lock(session_id: u64) -> Result<(), HotExitError> {
    let sessions_path = sessions_dir().ok_or(HotExitError::NoCacheDir)?;
    fs::create_dir_all(&sessions_path)?;

    let lock_path = lock_file(session_id).ok_or(HotExitError::NoCacheDir)?;
    fs::write(&lock_path, std::process::id().to_string())?;
    debug!("hotexit: created lock for session {:016x}", session_id);
    Ok(())
}

/// Release our session lock only, keeping the session file for restoration after kill.
/// Use this when the app is being killed (SIGTERM/SIGKILL) and we want to preserve
/// the session for recovery on next launch.
/// See also: `release_and_cleanup_session` for clean shutdown.
pub fn release_session_lock(session_id: u64) {
    if let Some(lock_path) = lock_file(session_id) {
        remove_file_if_exists(&lock_path, "lock file");
    }
}

/// Release session lock AND remove session file for a clean shutdown.
/// Use this when all tabs have been saved/closed normally and there's nothing to restore.
/// See also: `release_session_lock` for keeping session recoverable.
pub fn release_and_cleanup_session(session_id: u64) {
    release_session_lock(session_id);

    // Remove session file (clean shutdown means no need to restore)
    if let Some(session_path) = session_file(session_id) {
        remove_file_if_exists(&session_path, "session file");
    }
}

/// Check if a session is active (has a lock file with a running process)
pub fn is_session_active(session_id: u64) -> bool {
    lock_file(session_id)
        .filter(|p| p.exists())
        .and_then(|lock_path| read_lock_pid(&lock_path))
        .map(is_process_running)
        .unwrap_or(false)
}

/// Atomically try to acquire a session lock.
///
/// This function is race-condition safe: it first checks if an existing lock is stale
/// (process not running), removes it if so, then atomically creates a new lock file
/// using `create_new(true)` which fails if the file already exists.
///
/// Returns `Ok(())` if lock was acquired, `Err(SessionLocked)` if another process holds it.
pub fn try_acquire_session_lock(session_id: u64) -> Result<(), HotExitError> {
    let lock_path = lock_file(session_id).ok_or(HotExitError::NoCacheDir)?;

    // Ensure sessions directory exists
    if let Some(sessions_path) = sessions_dir() {
        fs::create_dir_all(&sessions_path)?;
    }

    // Check if existing lock is stale (process not running)
    if lock_path.exists() {
        if let Some(pid) = read_lock_pid(&lock_path) {
            if is_process_running(pid) {
                // Lock is held by a running process
                return Err(HotExitError::SessionLocked);
            }
        }
        // Stale lock file - remove it before atomic creation
        debug!(
            "hotexit: removing stale lock for session {:016x}",
            session_id
        );
        if let Err(e) = fs::remove_file(&lock_path) {
            warn!("hotexit: failed to remove stale lock: {}", e);
        }
    }

    // Atomic creation - fails if file was created between our check and now
    let file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // Another process created the lock between our check and creation
            debug!("hotexit: lock race lost for session {:016x}", session_id);
            return Err(HotExitError::SessionLocked);
        }
        Err(e) => return Err(HotExitError::Io(e)),
    };

    // Write our PID to the lock file and ensure it's flushed to disk
    use std::io::Write as IoWrite;
    writeln!(&file, "{}", std::process::id())?;
    file.sync_all()?;

    debug!("hotexit: acquired lock for session {:016x}", session_id);
    Ok(())
}

/// Adopt an orphaned session by taking over its session ID.
///
/// Uses atomic lock acquisition to prevent race conditions when multiple
/// instances try to adopt the same session simultaneously.
///
/// Returns true if the session file exists and adoption succeeded.
pub fn adopt_session(session_id: u64) -> bool {
    // Check that the session file exists
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

    // Atomically acquire the lock (handles stale lock cleanup internally)
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

// ============================================================================
// Orphaned session management
// ============================================================================

/// Find all orphaned sessions (session files without active locks)
/// Returns a vector of (session_id, SessionState) for all orphaned sessions, sorted by session_id
pub fn find_all_orphaned_sessions() -> Vec<(u64, SessionState)> {
    let sessions_path = match sessions_dir() {
        Some(p) => p,
        None => return Vec::new(),
    };

    // Create directory if needed
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

        // Only look at .json files (session state files)
        let is_json = path.extension().map(|e| e == "json").unwrap_or(false);
        if !is_json {
            continue;
        }

        // Extract session ID from filename
        let session_id = match extract_session_id_from_path(&path) {
            Some(id) => id,
            None => continue,
        };

        // Check if this session has an active lock
        if is_session_active(session_id) {
            debug!("hotexit: session {:016x} is active, skipping", session_id);
            continue;
        }

        // Clean up stale lock file if present (we know it's stale since is_session_active returned false)
        if let Some(lock_path) = lock_file(session_id) {
            remove_file_if_exists(&lock_path, "stale lock");
        }

        // Load the session state
        match fs::read_to_string(&path) {
            Ok(json) => match serde_json::from_str::<SessionState>(&json) {
                Ok(state) => {
                    info!("hotexit: found orphaned session {:016x}", session_id);
                    orphaned.push((session_id, state));
                }
                Err(e) => {
                    warn!(
                        "hotexit: failed to parse session {:016x}: {}",
                        session_id, e
                    );
                }
            },
            Err(e) => {
                warn!("hotexit: failed to read session {:016x}: {}", session_id, e);
            }
        }
    }

    // Sort by session_id (which is time-based) so oldest sessions are first
    orphaned.sort_by_key(|(id, _)| *id);
    orphaned
}

/// Claim an orphaned session by removing its session file
/// Call this after successfully restoring a session
pub fn claim_orphaned_session(session_id: u64) {
    if let Some(session_path) = session_file(session_id) {
        match fs::remove_file(&session_path) {
            Ok(()) => {
                info!("hotexit: claimed orphaned session {:016x}", session_id);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // File already gone, that's fine
            }
            Err(e) => {
                debug!("hotexit: failed to remove session file: {}", e);
            }
        }
    }
}

/// Discard a session completely (claim it and remove all its backups)
/// Use when user chooses to discard a session or when session is beyond restore limit
pub fn discard_session(session_id: u64) {
    claim_orphaned_session(session_id);
    remove_session_backups(session_id);
}

// ============================================================================
// Stale backup cleanup
// ============================================================================

/// Get all known session IDs (both active and orphaned)
/// This includes sessions with lock files (active) and session files (orphaned or active)
fn get_known_session_ids() -> std::collections::HashSet<u64> {
    use std::collections::HashSet;

    let mut known_sessions = HashSet::new();

    let sessions_path = match sessions_dir() {
        Some(p) => p,
        None => return known_sessions,
    };

    let entries = match fs::read_dir(&sessions_path) {
        Ok(e) => e,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                warn!(
                    "hotexit: failed to read sessions directory {:?}: {}",
                    sessions_path, e
                );
            }
            return known_sessions;
        }
    };

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();

        // Look at both .json and .lock files
        let ext = path.extension().and_then(|e| e.to_str());
        if ext != Some("json") && ext != Some("lock") {
            continue;
        }

        // Extract session ID from filename
        if let Some(session_id) = extract_session_id_from_path(&path) {
            known_sessions.insert(session_id);
        }
    }

    known_sessions
}

/// Extract session ID from a session or lock file path.
/// Session/lock files are named {session_id:016x}.json or {session_id:016x}.lock
fn extract_session_id_from_path(path: &std::path::Path) -> Option<u64> {
    path.file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| u64::from_str_radix(s, 16).ok())
}

/// Clean up stale backup files that belong to sessions that no longer exist.
/// Call this at startup after session restoration is complete.
pub fn cleanup_stale_backups() {
    let known_sessions = get_known_session_ids();
    backup::cleanup_stale_backups(&known_sessions);
}

// ============================================================================
// Dialog state helpers
// ============================================================================

/// Computed state for the restore sessions dialog.
///
/// This helper computes the values needed for the dialog based on the current
/// selection, avoiding redundant calculations in the UI code.
#[derive(Debug, Clone, Copy)]
pub struct RestoreDialogState {
    /// Total number of orphaned sessions
    pub total_sessions: usize,
    /// Maximum sessions that can be auto-restored
    pub max_auto_restore: usize,
    /// Number of sessions that will be restored based on current selection
    pub sessions_to_restore: usize,
    /// Number of sessions that will be discarded based on current selection
    pub sessions_to_discard: usize,
}

impl RestoreDialogState {
    /// Create dialog state from session count and selected option.
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

    /// Get the count to show for the "restore N" option.
    pub fn restore_n_count(&self) -> usize {
        self.max_auto_restore.min(self.total_sessions)
    }
}

// ============================================================================
// Startup action helpers
// ============================================================================

/// Action to take at startup based on orphaned sessions
pub enum StartupAction {
    /// No orphaned sessions found, normal startup
    Normal,
    /// Single orphaned session to restore in this instance
    RestoreSingle(u64, SessionState),
    /// Multiple sessions within auto-restore limit
    /// First session to restore in this instance, remaining session IDs to spawn
    RestoreMultiple {
        first_session: (u64, SessionState),
        spawn_sessions: Vec<u64>,
    },
    /// Too many orphaned sessions, prompt user for action
    /// Contains all orphaned session IDs
    PromptUser(Vec<u64>),
}

/// Determine startup action based on orphaned sessions and configuration.
///
/// This encapsulates the logic for deciding what to do at startup when there are
/// orphaned sessions from previous crashes.
pub fn determine_startup_action(reopen_on_start: bool, max_auto_restore: usize) -> StartupAction {
    if !reopen_on_start {
        debug!("hotexit: reopen_on_start disabled, skipping session restore");
        return StartupAction::Normal;
    }

    let orphaned = find_all_orphaned_sessions();

    match orphaned.len() {
        0 => {
            debug!("hotexit: no orphaned sessions found");
            StartupAction::Normal
        }
        1 => {
            // Safe destructure - match guard guarantees exactly one element
            let mut iter = orphaned.into_iter();
            match iter.next() {
                Some((session_id, state)) => StartupAction::RestoreSingle(session_id, state),
                None => StartupAction::Normal, // Defensive fallback
            }
        }
        n if n <= max_auto_restore => {
            let mut iter = orphaned.into_iter();
            // Safe destructure - match guard guarantees at least one element
            match iter.next() {
                Some(first_session) => {
                    let spawn_sessions: Vec<u64> = iter.map(|(id, _)| id).collect();
                    StartupAction::RestoreMultiple {
                        first_session,
                        spawn_sessions,
                    }
                }
                None => StartupAction::Normal, // Defensive fallback
            }
        }
        _ => {
            info!(
                "hotexit: found {} orphaned sessions, prompting user",
                orphaned.len()
            );
            let session_ids: Vec<u64> = orphaned.into_iter().map(|(id, _)| id).collect();
            StartupAction::PromptUser(session_ids)
        }
    }
}

// ============================================================================
// Tab restoration helpers
// ============================================================================

/// A tab to be restored, loaded from either backup or file
pub enum RestoredTab {
    /// Tab with unsaved changes restored from backup
    FromBackup {
        /// File path (None for untitled documents)
        path_opt: Option<PathBuf>,
        /// Document content from backup
        content: String,
        /// Cursor line position
        cursor_line: usize,
        /// Cursor character position within line
        cursor_index: usize,
        /// Zoom adjustment
        zoom_adj: i8,
        /// The backup ID
        backup_id: String,
        /// Hash of the content
        content_hash: u64,
    },
    /// Tab restored from file (no unsaved changes)
    FromFile {
        /// File path to open
        path: PathBuf,
    },
}

/// Load tabs from a session state for restoration.
///
/// Reads backup files for tabs with unsaved changes, returns file paths for clean tabs.
/// Returns a Vec of RestoredTab in order, which the caller can use to create EditorTabs.
pub fn load_session_tabs(session: &SessionState) -> Vec<RestoredTab> {
    let mut tabs = Vec::new();

    for session_tab in &session.tabs {
        if session_tab.has_unsaved_changes {
            // Try to restore from backup cache
            if let Some(backup_id) = &session_tab.backup_id {
                if let Some(cached_doc) = read_backup_by_id(backup_id) {
                    info!(
                        "hotexit: loading tab from backup id={}, path={:?}",
                        cached_doc.id(),
                        cached_doc.path()
                    );
                    tabs.push(RestoredTab::FromBackup {
                        path_opt: cached_doc.path().cloned(),
                        content: cached_doc.content.clone(),
                        cursor_line: cached_doc.cursor_line(),
                        cursor_index: cached_doc.cursor_index(),
                        zoom_adj: cached_doc.zoom_adj(),
                        backup_id: cached_doc.id().to_string(),
                        content_hash: compute_content_hash(&cached_doc.content),
                    });
                    continue;
                }
            }
            // Backup not found, try to load from file if path exists
            warn!(
                "hotexit: backup not found for tab (id={:?}, path={:?}), falling back to file",
                session_tab.backup_id, session_tab.path
            );
        }

        // Restore from file (no unsaved changes or backup not found)
        if let Some(path) = &session_tab.path {
            if path.exists() {
                tabs.push(RestoredTab::FromFile { path: path.clone() });
            }
        }
    }

    tabs
}

// ============================================================================
// Backup write helpers
// ============================================================================

/// Request to write a backup for a single tab
pub struct BackupRequest {
    /// File path (None for untitled documents)
    pub path_opt: Option<PathBuf>,
    /// Document content
    pub content: String,
    /// Cursor line position
    pub cursor_line: usize,
    /// Cursor character position within line
    pub cursor_index: usize,
    /// Zoom adjustment
    pub zoom_adj: i8,
    /// Existing backup ID (if any)
    pub existing_backup_id: Option<String>,
    /// Previous content hash (for change detection)
    pub previous_content_hash: Option<u64>,
}

/// Result of a successful backup write
pub struct BackupResult {
    /// The backup ID used (may be new or existing)
    pub backup_id: String,
    /// Hash of the written content
    pub content_hash: u64,
}

/// Write a backup for a single tab.
///
/// Returns:
/// - `Ok(Some(BackupResult))` if backup was written successfully
/// - `Ok(None)` if backup was skipped (empty content or unchanged)
/// - `Err(e)` if backup failed due to I/O or other errors
///
/// The caller should update the tab's backup_id and backup_content_hash from the result.
pub fn write_tab_backup(
    session_id: u64,
    request: BackupRequest,
    untitled_counter: &mut usize,
) -> Result<Option<BackupResult>, HotExitError> {
    // Skip completely empty documents
    if request.content.is_empty() {
        debug!("hotexit: skipping empty document {:?}", request.path_opt);
        return Ok(None);
    }

    // Compute hash of content to check if backup is needed
    let content_hash = compute_content_hash(&request.content);

    // Skip if content hasn't changed since last backup
    if request.previous_content_hash == Some(content_hash) {
        debug!(
            "hotexit: skipping unchanged backup for {:?}",
            request.path_opt
        );
        return Ok(None);
    }

    // Get backup directory
    let backup_dir = backup_dir().ok_or(HotExitError::NoCacheDir)?;

    // Use existing backup_id if available, otherwise generate new one
    let backup_id = request.existing_backup_id.unwrap_or_else(|| {
        let doc_id = generate_doc_id(&request.path_opt, session_id, *untitled_counter);
        if request.path_opt.is_none() {
            *untitled_counter += 1;
        }
        make_backup_id(session_id, &doc_id)
    });

    let meta = CachedDocumentMeta {
        id: backup_id.clone(),
        session_id,
        path: request.path_opt.clone(),
        cursor_line: request.cursor_line,
        cursor_index: request.cursor_index,
        zoom_adj: request.zoom_adj,
    };
    let doc = CachedDocument::new(meta, request.content);
    let backup_path = backup_dir.join(format!("{}.backup", backup_id));

    write_backup_file(&backup_path, &doc)?;
    Ok(Some(BackupResult {
        backup_id,
        content_hash,
    }))
}

// ============================================================================
// App helper functions
// ============================================================================

/// Clean up backup file after a successful save operation.
/// Logs a warning if removal fails but doesn't propagate the error.
pub fn cleanup_backup_after_save(backup_id: Option<String>) {
    if let Some(id) = backup_id {
        if let Err(e) = remove_backup_file(&id) {
            warn!("hotexit: failed to remove backup {}: {}", id, e);
        }
    }
}

/// Spawn a new editor instance to restore a specific session.
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
