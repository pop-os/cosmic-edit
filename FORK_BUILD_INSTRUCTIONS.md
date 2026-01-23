# Fork Build Instructions

This document explains how to build this forked version of cosmic-edit with rope buffer support for large files.

## Overview

This fork includes:
- **Large file memory fix** (Issue #457): Prevents memory explosion when opening 100K+ line files
- **Rope buffer integration**: Experimental windowed viewing of large files (100MB+)

## Prerequisites

- Rust 1.85 or later (Edition 2024)
- Standard COSMIC desktop development dependencies

## Build Steps

### 1. Clone the repository

```bash
git clone https://github.com/Lcstyle/cosmic-edit.git
cd cosmic-edit
git checkout feature/rope-buffer-integration
```

### 2. Remove stale lock file

The Cargo.lock may contain references to upstream packages that conflict with our patches. Remove it to force regeneration:

```bash
rm Cargo.lock
```

### 3. Build

```bash
cargo build --release
```

The build will automatically:
- Fetch the forked cosmic-text from `https://github.com/Lcstyle/cosmic-text.git` (branch: `feature/rope-buffer`)
- Apply the `[patch]` section in Cargo.toml to redirect all cosmic-text dependencies to the fork

## Key Patches

This build uses patched dependencies defined in `Cargo.toml`:

```toml
# Direct dependency on forked cosmic-text
[dependencies.cosmic-text]
git = "https://github.com/Lcstyle/cosmic-text.git"
branch = "feature/rope-buffer"
features = ["syntect", "vi", "rope-buffer"]

# Patch to redirect transitive dependencies (e.g., via libcosmic/iced_glyphon)
[patch.'https://github.com/pop-os/cosmic-text.git']
cosmic-text = { git = "https://github.com/Lcstyle/cosmic-text.git", branch = "feature/rope-buffer" }

# Patch for onig (syntect dependency)
[patch.crates-io]
onig = { git = "https://github.com/rust-onig/rust-onig.git", branch = "main" }
onig_sys = { git = "https://github.com/rust-onig/rust-onig.git", branch = "main" }
```

## Current Limitations

### Rope Buffer (Large Files)

The rope buffer integration is currently **read-only**:
- Files over 1MB are loaded into a rope data structure
- Only ~500 lines around the cursor are loaded into the editor at a time
- Scrolling updates the visible window
- **Editing and saving large files is NOT fully implemented** - edits are only made to the visible window

For editing large files, use the standard buffer (files under 1MB threshold).

### Memory Improvements

For the memory fix (without rope buffer), files are loaded normally but with a minimal buffer height set before loading to prevent shaping all lines at once. This reduces memory usage from 18+ GB to ~200MB for 60MB files.

## Branches

| Branch | Description |
|--------|-------------|
| `fix/large-file-memory` | Minimal 23-line fix for memory explosion |
| `feature/rope-buffer-integration` | Full rope buffer integration (experimental) |

## Related Repositories

- **cosmic-text fork**: https://github.com/Lcstyle/cosmic-text.git (branch: `feature/rope-buffer`)
  - Adds `RopeBuffer`, `RopeText`, `SparseMetadata`, `LineCache` types
  - Behind the `rope-buffer` feature flag

## Troubleshooting

### Version Mismatch Errors

If you see errors about conflicting cosmic-text versions:

```
error: failed to select a version for `cosmic-text`
```

Delete `Cargo.lock` and rebuild:

```bash
rm Cargo.lock
cargo build --release
```

### Missing Dependencies

Ensure you have the COSMIC desktop development dependencies installed. On Fedora:

```bash
sudo dnf install gtk3-devel libxkbcommon-devel wayland-devel
```

## Running

```bash
./target/release/cosmic-edit
```

Or for large file testing:

```bash
./target/release/cosmic-edit /path/to/large/file.txt
```
