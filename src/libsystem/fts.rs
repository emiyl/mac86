/// FTS (file tree traversal) handle management for recursive directory operations.

use lazy_static::lazy_static;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use walkdir::WalkDir;

#[derive(Clone)]
pub struct FtsHandle {
    entries: Vec<(PathBuf, u32, u16)>, // (path, fts_info, level)
    pub index: usize,
}

lazy_static! {
    static ref FTS_HANDLES: Mutex<HashMap<u32, FtsHandle>> = Mutex::new(HashMap::new());
    static ref FTS_COUNTER: Mutex<u32> = Mutex::new(1);
}

pub fn allocate_fts_handle(path_str: &str) -> Option<u32> {
    let path = std::path::Path::new(path_str);
    let mut entries = Vec::new();

    // Walk directory and collect entries in post-order (for proper deletion)
    // Post-order: children first, then parent
    let mut all_entries = Vec::new();
    for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
        let entry_path = entry.path().to_path_buf();
        let level = entry.depth() as u16;
        let is_dir = entry.file_type().is_dir();

        all_entries.push((entry_path, is_dir, level));
    }

    // Sort to get post-order: deeper items first, then parents
    // This ensures we delete files before directories
    all_entries.sort_by(|a, b| {
        // Sort by depth descending (deeper items first)
        // Then by path length descending (longer paths first, which are typically files)
        match b.2.cmp(&a.2) {
            std::cmp::Ordering::Equal => b.0.as_os_str().len().cmp(&a.0.as_os_str().len()),
            other => other,
        }
    });

    // Convert to (path, fts_info, level) where:
    // FTS_D = 1 (preorder directory), FTS_F = 8 (regular file), FTS_DP = 6 (postorder directory)
    // rm -r performs rmdir on FTS_DP entries, so the root must also be FTS_DP.
    for (entry_path, is_dir, level) in all_entries {
        // Use FTS_F=8 for files and FTS_DP=6 for directories.
        let fts_info = if is_dir {
            6 // FTS_DP for post-order directories (including root)
        } else {
            8 // FTS_F for files
        };

        entries.push((entry_path, fts_info, level));
    }

    let handle = FtsHandle { entries, index: 0 };

    let mut counter = FTS_COUNTER.lock().unwrap();
    let handle_id = *counter;
    *counter = counter.wrapping_add(1);

    FTS_HANDLES.lock().unwrap().insert(handle_id, handle);
    Some(handle_id)
}

impl FtsHandle {
    /// Returns (path, fts_info, level, old_index) for the current entry and advances.
    pub fn next_entry(&mut self) -> Option<(PathBuf, u32, u16, usize)> {
        if self.index >= self.entries.len() {
            return None;
        }
        let (path, info, level) = self.entries[self.index].clone();
        let idx = self.index;
        self.index += 1;
        Some((path, info, level, idx))
    }
}

pub fn with_fts_handle<F, R>(handle_id: u32, f: F) -> Option<R>
where
    F: FnOnce(&mut FtsHandle) -> R,
{
    let mut handles = FTS_HANDLES.lock().unwrap();
    handles.get_mut(&handle_id).map(f)
}

pub fn close_fts_handle(handle_id: u32) -> bool {
    FTS_HANDLES.lock().unwrap().remove(&handle_id).is_some()
}
