/// FTS (file tree traversal) handle management for directory operations.

use lazy_static::lazy_static;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use walkdir::WalkDir;

pub const FTS_D: u32 = 1;   // preorder directory
pub const FTS_SL: u32 = 3;  // symbolic link
pub const FTS_F: u32 = 8;   // regular file
pub const FTS_DP: u32 = 6;  // postorder directory
pub const FTS_SKIP: u32 = 4; // fts_set option: skip this directory

#[derive(Clone)]
pub struct FtsHandle {
    pub root_path: PathBuf,
    /// Pre-generated entries: (host_path, fts_info, level)
    pub entries: Vec<(PathBuf, u32, u16)>,
    pub index: usize,
    /// Level of the most recently returned FTS_D entry (for fts_set).
    pub current_level: u16,
    /// Index of the most recently returned FTS_D entry + 1 (for fts_children).
    /// direct_children starts here so extra fts_read calls between fts_read(FTS_D)
    /// and fts_children don't cause earlier siblings to be skipped.
    pub current_dir_child_start: usize,
    /// When set, next_entry skips entries whose level > skip_level, stopping at
    /// the matching FTS_DP so the traversal can continue normally from there.
    pub skip_level: Option<u16>,
}

lazy_static! {
    static ref FTS_HANDLES: Mutex<HashMap<u32, FtsHandle>> = Mutex::new(HashMap::new());
    static ref FTS_COUNTER: Mutex<u32> = Mutex::new(1);
}

/// Build a preorder FTS entry list from a host path.
///
/// Order:
///   FTS_D  dir      (entering)
///   FTS_F  file
///   FTS_D  subdir
///   FTS_F  file-in-subdir
///   FTS_DP subdir   (leaving)
///   FTS_DP dir      (leaving)
pub fn allocate_fts_handle(path_str: &str) -> Option<u32> {
    let path = std::path::Path::new(path_str);
    let mut entries: Vec<(PathBuf, u32, u16)> = Vec::new();
    // Stack tracks open directories: (path, depth)
    let mut dir_stack: Vec<(PathBuf, u16)> = Vec::new();

    for entry in WalkDir::new(path)
        .sort_by_file_name()
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let entry_path = entry.path().to_path_buf();
        let depth = entry.depth() as u16;
        let is_dir = entry.file_type().is_dir();
        let is_symlink = entry.path_is_symlink();

        // Close any directories whose depth >= current depth (we've left them).
        while dir_stack
            .last()
            .map_or(false, |(_, d)| *d >= depth)
        {
            let (closed, closed_depth) = dir_stack.pop().unwrap();
            entries.push((closed, FTS_DP, closed_depth));
        }

        if is_symlink {
            entries.push((entry_path, FTS_SL, depth));
        } else if is_dir {
            entries.push((entry_path.clone(), FTS_D, depth));
            dir_stack.push((entry_path, depth));
        } else {
            entries.push((entry_path, FTS_F, depth));
        }
    }

    // Close any directories still open.
    while let Some((closed, closed_depth)) = dir_stack.pop() {
        entries.push((closed, FTS_DP, closed_depth));
    }

    let root_path = path.to_path_buf();
    let handle = FtsHandle {
        root_path,
        entries,
        index: 0,
        current_level: 0,
        current_dir_child_start: 0,
        skip_level: None,
    };

    let mut counter = FTS_COUNTER.lock().unwrap();
    let handle_id = *counter;
    *counter = counter.wrapping_add(1);
    FTS_HANDLES.lock().unwrap().insert(handle_id, handle);
    Some(handle_id)
}

impl FtsHandle {
    /// Advance to the next entry, respecting any active FTS_SKIP.
    pub fn next_entry(&mut self) -> Option<(PathBuf, u32, u16, usize)> {
        loop {
            if self.index >= self.entries.len() {
                return None;
            }
            let (path, info, level) = self.entries[self.index].clone();
            let idx = self.index;
            self.index += 1;

            if let Some(skip_level) = self.skip_level {
                if level > skip_level {
                    continue; // skip children inside the skipped directory
                }
                // Reached the FTS_DP for the skipped directory — clear skip.
                if info == FTS_DP && level == skip_level {
                    self.skip_level = None;
                }
            }

            if info == FTS_D {
                self.current_level = level;
                self.current_dir_child_start = self.index; // one past FTS_D
            }

            return Some((path, info, level, idx));
        }
    }

    /// Mark the current directory as skipped (fts_set FTS_SKIP).
    pub fn set_skip(&mut self) {
        self.skip_level = Some(self.current_level);
    }

    /// Return (path, fts_info, level, entry_index) for every DIRECT child of
    /// the directory at `parent_level` starting from the current index.
    ///
    /// "Direct child" means level == parent_level + 1 and is not a FTS_DP.
    ///
    /// If the entry at the current index is the parent's own FTS_D (which
    /// happens when fts_children is called before the first fts_read), it is
    /// skipped so we still return the children correctly.
    /// Return direct children at `parent_level + 1`, always scanning from just
    /// after the last FTS_D returned by fts_read (so that extra fts_read calls
    /// between fts_read(FTS_D) and fts_children don't cause siblings to be skipped).
    pub fn direct_children(&self, parent_level: u16) -> Vec<(PathBuf, u32, u16, usize)> {
        let child_level = parent_level + 1;
        let mut result = Vec::new();

        // Starting point: just after the FTS_D that was last returned.
        // current_dir_child_start is set to self.index immediately after fts_read
        // returns FTS_D, so it always points to the first child slot.
        // If fts_children is called before any fts_read (current_dir_child_start==0),
        // check whether index 0 is the parent's FTS_D and skip it.
        let start = if self.current_dir_child_start > 0 {
            self.current_dir_child_start
        } else {
            match self.entries.get(0) {
                Some((_, FTS_D, lvl)) if *lvl == parent_level => 1,
                _ => 0,
            }
        };

        let mut i = start;
        while i < self.entries.len() {
            let (ref p, info, lvl) = self.entries[i];
            if lvl <= parent_level {
                break;
            }
            if lvl == child_level && info != FTS_DP {
                result.push((p.clone(), info, lvl, i));
            }
            i += 1;
        }
        result
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
