// SPDX-License-Identifier: GPL-3.0-only

//! Backup file management for hot exit functionality.
//!
//! This module handles:
//! - Document backup file I/O (read, write, remove)
//! - Backup directory management
//! - Stale backup cleanup
//! - Content hashing for change detection

use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

/// File extension for backup files
const BACKUP_FILE_EXT: &str = "backup";

/// Maximum backup file size (10 MB). Documents larger than this will skip backup with a warning.
const MAX_BACKUP_SIZE: usize = 10 * 1024 * 1024;

// ============================================================================
// Document types
// ============================================================================

/// Metadata for a cached document (serialized as first line of backup files).
/// Kept separate from content to enable efficient serialization without cloning.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CachedDocumentMeta {
    /// Unique identifier for this cached document (hash of path or random for untitled)
    pub id: String,
    /// Session ID to identify which app instance created this cache
    pub session_id: u64,
    /// The file path if the document was associated with a file, None for new unsaved documents
    pub path: Option<PathBuf>,
    /// Cursor line position (0-indexed)
    pub cursor_line: usize,
    /// Cursor column/index position
    pub cursor_index: usize,
    /// Zoom adjustment level
    pub zoom_adj: i8,
}

/// Represents a cached unsaved document that can be restored after a crash or unexpected close.
/// Uses composition with CachedDocumentMeta to avoid field duplication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachedDocument {
    /// Document metadata (id, path, cursor position, etc.)
    pub meta: CachedDocumentMeta,
    /// The text content of the document
    pub content: String,
}

impl CachedDocument {
    /// Create a new CachedDocument from metadata and content
    pub fn new(meta: CachedDocumentMeta, content: String) -> Self {
        Self { meta, content }
    }

    // Convenience accessors to maintain API compatibility
    pub fn id(&self) -> &str {
        &self.meta.id
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.meta.path.as_ref()
    }

    pub fn cursor_line(&self) -> usize {
        self.meta.cursor_line
    }

    pub fn cursor_index(&self) -> usize {
        self.meta.cursor_index
    }

    pub fn zoom_adj(&self) -> i8 {
        self.meta.zoom_adj
    }
}

// ============================================================================
// ID generation
// ============================================================================

/// Compute a deterministic hash of the given content for change detection.
/// Uses FNV-1a which is fast and deterministic across processes (unlike DefaultHasher).
pub fn compute_content_hash(content: &str) -> u64 {
    // FNV-1a constants for 64-bit
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in content.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Generate a document ID from the path or a random ID for untitled documents.
/// Uses FNV-1a for deterministic path hashing across processes.
pub fn generate_doc_id(path: &Option<PathBuf>, session_id: u64, index: usize) -> String {
    match path {
        Some(p) => {
            // Hash the path bytes using FNV-1a for deterministic results
            let path_bytes = p.to_string_lossy();
            format!("{:016x}", compute_content_hash(&path_bytes))
        }
        None => {
            // For untitled documents, use session_id and index
            format!("untitled_{:016x}_{}", session_id, index)
        }
    }
}

/// Create a backup ID from session ID and document ID
/// Format: {session_id:016x}_{doc_id}
pub fn make_backup_id(session_id: u64, doc_id: &str) -> String {
    format!("{:016x}_{}", session_id, doc_id)
}

// ============================================================================
// Directory paths
// ============================================================================

/// Get the backup directory path for cosmic-edit (in cache directory)
pub fn backup_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("cosmic-edit").join("backups"))
}

// ============================================================================
// Backup file operations
// ============================================================================

/// Clean up orphaned temp files from interrupted backup writes.
/// Call this at startup to remove any .backup.tmp files left from crashes.
pub fn cleanup_temp_files() {
    let backup_path = match backup_dir() {
        Some(p) => p,
        None => return,
    };

    let entries = match fs::read_dir(&backup_path) {
        Ok(e) => e,
        Err(e) => {
            // Directory might not exist yet (first run), that's OK
            if e.kind() != std::io::ErrorKind::NotFound {
                warn!(
                    "backup: failed to read backup directory {:?}: {}",
                    backup_path, e
                );
            }
            return;
        }
    };

    let mut removed = 0;
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        // Check for .tmp extension (from interrupted atomic writes)
        let is_tmp = path.extension().map(|e| e == "tmp").unwrap_or(false);
        if is_tmp {
            debug!("backup: removing orphaned temp file {:?}", path);
            if fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
    }

    if removed > 0 {
        info!("backup: cleaned up {} orphaned temp files", removed);
    }
}

/// Read a backup file and return the CachedDocument
#[must_use]
pub fn read_backup_file(path: &Path) -> Option<CachedDocument> {
    debug!("backup: reading file {:?}", path);

    // Read entire file content
    let file_content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            warn!("backup: failed to read file {:?}: {}", path, e);
            return None;
        }
    };

    // First line contains JSON metadata, rest is content
    // Find the first newline to split metadata from content
    let (metadata_line, content) = match file_content.find('\n') {
        Some(pos) => {
            let metadata = &file_content[..pos];
            // Skip the newline character after metadata
            let content_start = pos + 1;
            let content = if content_start < file_content.len() {
                &file_content[content_start..]
            } else {
                ""
            };
            (metadata, content.to_string())
        }
        None => {
            // No newline found - file only contains metadata, empty content
            (file_content.as_str(), String::new())
        }
    };

    let meta: CachedDocumentMeta = match serde_json::from_str(metadata_line) {
        Ok(m) => m,
        Err(e) => {
            warn!("backup: failed to parse metadata: {}", e);
            return None;
        }
    };

    let doc = CachedDocument::new(meta, content);

    debug!(
        "backup: loaded id={}, path={:?}, content_len={}",
        doc.id(),
        doc.path(),
        doc.content.len()
    );
    Some(doc)
}

/// Write a backup file for a CachedDocument using atomic write (temp file + rename)
pub fn write_backup_file(backup_path: &Path, doc: &CachedDocument) -> std::io::Result<()> {
    // Skip backup for very large documents to avoid excessive disk usage
    if doc.content.len() > MAX_BACKUP_SIZE {
        warn!(
            "backup: skipping backup for {:?} - content too large ({} bytes > {} byte limit)",
            doc.path(),
            doc.content.len(),
            MAX_BACKUP_SIZE
        );
        return Ok(());
    }

    debug!(
        "backup: writing id={}, path={:?}, content_len={} to {:?}",
        doc.id(),
        doc.path(),
        doc.content.len(),
        backup_path
    );

    // Create backup directory if it doesn't exist
    if let Some(parent) = backup_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Write to a temp file first, then rename for atomic operation
    let temp_path = backup_path.with_extension("backup.tmp");
    let mut file = fs::File::create(&temp_path)?;

    // Write metadata on first line (without content to keep it small)
    //
    // INVARIANT: Metadata JSON must not contain newlines.
    //
    // The backup file format uses the first newline to separate metadata from content:
    //   Line 1: JSON metadata (single line)
    //   Lines 2+: Document content (may contain newlines)
    //
    // This invariant is guaranteed by serde_json::to_string(), which produces
    // compact JSON without newlines (unlike to_string_pretty).
    let metadata_json = serde_json::to_string(&doc.meta)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if metadata_json.contains('\n') {
        let _ = fs::remove_file(&temp_path);
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "metadata JSON contains newlines",
        ));
    }
    writeln!(file, "{}", metadata_json)?;

    // Write content on subsequent lines
    write!(file, "{}", doc.content)?;

    // Ensure data is flushed to disk before rename
    file.sync_all()?;
    drop(file);

    // Atomic rename - clean up temp file on failure
    if let Err(e) = fs::rename(&temp_path, backup_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(e);
    }

    debug!("backup: written successfully");
    Ok(())
}

/// Validate that a backup/document ID doesn't contain path traversal sequences.
/// Returns false if the ID is empty, contains path separators, parent directory
/// references, null bytes, or non-safe characters.
fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && !id.contains('\0')
        && !id.contains('/')
        && !id.contains('\\')
        && !id.contains("..")
        && id != "."
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Read a backup file by backup ID (direct lookup).
/// The backup_id should be the full ID including session prefix (e.g., "0123456789abcdef_docid")
#[must_use]
pub fn read_backup_by_id(backup_id: &str) -> Option<CachedDocument> {
    // Prevent path traversal attacks
    if !is_safe_id(backup_id) {
        warn!(
            "backup: rejecting backup_id with path traversal: {:?}",
            backup_id
        );
        return None;
    }
    let backup_dir = backup_dir()?;
    let backup_path = backup_dir.join(format!("{}.{}", backup_id, BACKUP_FILE_EXT));
    read_backup_file(&backup_path)
}

/// Remove a backup file by document ID
pub fn remove_backup_file(doc_id: &str) -> std::io::Result<()> {
    // Prevent path traversal attacks
    if !is_safe_id(doc_id) {
        warn!("backup: rejecting doc_id with path traversal: {:?}", doc_id);
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid document ID",
        ));
    }
    let backup_path = backup_dir().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "backup directory unavailable")
    })?;
    let file_path = backup_path.join(format!("{}.{}", doc_id, BACKUP_FILE_EXT));
    // Remove the file, treating NotFound as success (file already gone)
    match fs::remove_file(&file_path) {
        Ok(()) => {
            debug!("backup: removed file {:?}", file_path);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Remove all backup files for a specific session using efficient prefix matching.
/// No file reads required - just matches on filename prefix.
pub fn remove_session_backups(session_id: u64) {
    debug!(
        "backup: removing all backups for session {:016x}",
        session_id
    );
    let backup_path = match backup_dir() {
        Some(p) => p,
        None => return,
    };

    let prefix = format!("{:016x}_", session_id);
    let mut removed = 0;

    if let Ok(entries) = fs::read_dir(&backup_path) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with(&prefix) && name.ends_with(&format!(".{}", BACKUP_FILE_EXT)) {
                    match fs::remove_file(&path) {
                        Ok(()) => removed += 1,
                        Err(e) => debug!("backup: failed to remove {:?}: {}", path, e),
                    }
                }
            }
        }
    }

    if removed > 0 {
        info!(
            "backup: removed {} backups for session {:016x}",
            removed, session_id
        );
    }
}

/// Length of session ID in hex format (16 hex chars = 64 bits)
const SESSION_ID_HEX_LEN: usize = 16;

/// Minimum length of a valid backup filename:
/// session_id (16) + underscore (1) + min doc_id (1) + ".backup" (7) = 25
const MIN_BACKUP_FILENAME_LEN: usize = SESSION_ID_HEX_LEN + 1 + 1 + BACKUP_FILE_EXT.len();

/// Extract session ID from a backup filename.
/// Backup filenames have format: {session_id:016x}_{doc_id}.backup
///
/// This is exported for use by hotexit module to avoid code duplication.
pub fn extract_session_id_from_backup(filename: &str) -> Option<u64> {
    // Validate expected format: 16 hex chars + underscore + doc_id + .backup
    if !filename.ends_with(".backup") {
        return None;
    }
    if filename.len() < MIN_BACKUP_FILENAME_LEN {
        return None;
    }
    // Check underscore separator at position 16
    if filename.as_bytes().get(SESSION_ID_HEX_LEN) != Some(&b'_') {
        return None;
    }
    // Extract and parse the 16-char session ID prefix
    let prefix = filename.get(..SESSION_ID_HEX_LEN)?;
    u64::from_str_radix(prefix, 16).ok()
}

/// Clean up stale backup files that belong to sessions that no longer exist.
/// Takes a set of known session IDs to preserve.
pub fn cleanup_stale_backups(known_sessions: &std::collections::HashSet<u64>) {
    let backup_path = match backup_dir() {
        Some(p) => p,
        None => return,
    };

    let entries = match fs::read_dir(&backup_path) {
        Ok(e) => e,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                warn!(
                    "backup: failed to read backup directory {:?}: {}",
                    backup_path, e
                );
            }
            return;
        }
    };

    let mut removed = 0;
    let mut kept = 0;

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();

        // Only look at .backup files
        let is_backup = path
            .extension()
            .map(|e| e == BACKUP_FILE_EXT)
            .unwrap_or(false);
        if !is_backup {
            continue;
        }

        let filename = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };

        // Extract session ID from backup filename
        let session_id = match extract_session_id_from_backup(filename) {
            Some(id) => id,
            None => {
                debug!("backup: skipping file with invalid name: {}", filename);
                continue;
            }
        };

        // Check if this backup belongs to a known session
        if known_sessions.contains(&session_id) {
            kept += 1;
            continue;
        }

        // Stale backup - session no longer exists
        debug!(
            "backup: removing stale {:?} (session {:016x} not found)",
            path, session_id
        );
        match fs::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(e) => debug!("backup: failed to remove stale {:?}: {}", path, e),
        }
    }

    if removed > 0 {
        info!("backup: cleaned up {} stale files ({} kept)", removed, kept);
    } else if kept > 0 {
        debug!("backup: no stale files found ({} valid)", kept);
    }
}
