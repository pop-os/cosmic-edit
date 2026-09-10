// SPDX-License-Identifier: GPL-3.0-only

//! Backup file management for hot exit recovery.

use atomicwrites::{AllowOverwrite, AtomicFile};
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const BACKUP_FILE_EXT: &str = "backup";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CachedDocumentMeta {
    pub id: String,
    pub session_id: u64,
    pub cursor_line: usize,
    pub cursor_index: usize,
    pub zoom_adj: i8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachedDocument {
    pub meta: CachedDocumentMeta,
    pub content: String,
}

/// Compute a deterministic hash of the given content for change detection.
/// Uses FNV-1a for stable hashes across processes and Rust releases;
/// `DefaultHasher` does not guarantee a stable algorithm across releases.
pub fn compute_content_hash(content: &str) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0100_0000_01b3;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in content.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

pub fn backup_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|dir| dir.join("cosmic-edit").join("backups"))
}

pub fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let permissions = match fs::metadata(path) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    #[cfg(unix)]
    let identity = match fs::metadata(path) {
        Ok(metadata) => {
            use std::os::unix::fs::MetadataExt;
            if metadata.mode() & 0o6000 != 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "atomic replacement would change set-user-ID or set-group-ID permissions",
                ));
            }
            if metadata.nlink() > 1 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "atomic replacement would detach an existing hard link",
                ));
            }
            Some((metadata.uid(), metadata.gid()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    AtomicFile::new(path, AllowOverwrite)
        .write(|file| {
            #[cfg(unix)]
            if let Some((uid, gid)) = identity {
                use std::os::unix::fs::MetadataExt;
                let metadata = file.metadata()?;
                if metadata.uid() != uid || metadata.gid() != gid {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "atomic replacement would change file ownership",
                    ));
                }
            }
            if let Some(permissions) = permissions {
                file.set_permissions(permissions)?;
            }
            file.write_all(data)
        })
        .map_err(Into::into)
}

#[must_use]
pub fn read_backup_file(path: &Path) -> Option<CachedDocument> {
    debug!("backup: reading file {:?}", path);

    let file_content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            warn!("backup: failed to read file {:?}: {}", path, e);
            return None;
        }
    };

    let (metadata_line, content) = file_content
        .split_once('\n')
        .unwrap_or((file_content.as_str(), ""));

    let meta: CachedDocumentMeta = match serde_json::from_str(metadata_line) {
        Ok(m) => m,
        Err(e) => {
            warn!("backup: failed to parse metadata: {}", e);
            return None;
        }
    };

    let doc = CachedDocument {
        meta,
        content: content.to_string(),
    };

    debug!(
        "backup: loaded id={}, content_len={}",
        doc.meta.id,
        doc.content.len()
    );
    Some(doc)
}

pub fn write_backup_file(backup_path: &Path, doc: &CachedDocument) -> std::io::Result<()> {
    debug!(
        "backup: writing id={}, content_len={} to {:?}",
        doc.meta.id,
        doc.content.len(),
        backup_path
    );

    let mut data = serde_json::to_string(&doc.meta)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    data.push('\n');
    data.push_str(&doc.content);
    atomic_write(backup_path, data.as_bytes())?;

    debug!("backup: written successfully");
    Ok(())
}

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

pub fn backup_file_path(id: &str) -> std::io::Result<PathBuf> {
    if !is_safe_id(id) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid backup ID",
        ));
    }
    backup_dir()
        .map(|directory| directory.join(format!("{id}.{BACKUP_FILE_EXT}")))
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "backup directory unavailable")
        })
}

#[must_use]
pub fn read_backup_by_id(backup_id: &str) -> Option<CachedDocument> {
    let backup_path = backup_file_path(backup_id)
        .inspect_err(|error| warn!("backup: invalid backup ID {:?}: {}", backup_id, error))
        .ok()?;
    read_backup_file(&backup_path)
}

pub fn remove_backup_file(doc_id: &str) -> std::io::Result<()> {
    let file_path = backup_file_path(doc_id)?;
    match fs::remove_file(&file_path) {
        Ok(()) => {
            debug!("backup: removed file {:?}", file_path);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

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
    let suffix = format!(".{}", BACKUP_FILE_EXT);
    let mut removed = 0;

    if let Ok(entries) = fs::read_dir(&backup_path) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(&suffix))
            {
                match fs::remove_file(&path) {
                    Ok(()) => removed += 1,
                    Err(e) => debug!("backup: failed to remove {:?}: {}", path, e),
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

pub fn remove_unreferenced_session_backups(session_id: u64, referenced_ids: &HashSet<String>) {
    let Some(backup_path) = backup_dir() else {
        return;
    };
    let prefix = format!("{session_id:016x}_");
    let suffix = format!(".{BACKUP_FILE_EXT}");

    let Ok(entries) = fs::read_dir(&backup_path) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(backup_id) = file_name.strip_suffix(&suffix) else {
            continue;
        };
        if backup_id.starts_with(&prefix)
            && !referenced_ids.contains(backup_id)
            && let Err(error) = fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            warn!("backup: failed to remove unreferenced backup {path:?}: {error}");
        }
    }
}

const SESSION_ID_HEX_LEN: usize = 16;
const MIN_BACKUP_FILENAME_LEN: usize = SESSION_ID_HEX_LEN + 1 + 1 + BACKUP_FILE_EXT.len();

pub fn extract_session_id_from_backup(filename: &str) -> Option<u64> {
    if !filename.ends_with(".backup") {
        return None;
    }
    if filename.len() < MIN_BACKUP_FILENAME_LEN {
        return None;
    }
    if filename.as_bytes().get(SESSION_ID_HEX_LEN) != Some(&b'_') {
        return None;
    }
    let prefix = filename.get(..SESSION_ID_HEX_LEN)?;
    u64::from_str_radix(prefix, 16).ok()
}

pub fn backup_session_ids() -> std::collections::HashSet<u64> {
    let mut session_ids = std::collections::HashSet::new();
    let backup_path = match backup_dir() {
        Some(p) => p,
        None => return session_ids,
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
            return session_ids;
        }
    };

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();

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

        if let Some(session_id) = extract_session_id_from_backup(filename) {
            session_ids.insert(session_id);
        } else {
            debug!("backup: skipping file with invalid name: {}", filename);
        }
    }
    session_ids
}
