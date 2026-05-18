use crate::errors::{EmulationError, EmulationResult};
use crate::threads::ThreadTable;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub struct FileStat {
    pub size: u64,
    pub is_regular: bool,
    pub is_dir: bool,
}

/// Virtual filesystem for the emulation environment
pub struct VirtualFileSystem {
    /// Mapping of emulated paths to host paths
    mounts: HashMap<PathBuf, PathBuf>,

    /// Open file descriptors
    file_descriptors: HashMap<u32, FileDescriptor>,

    /// Next available file descriptor
    next_fd: u32,

    /// Program break for brk(2). Starts well above typical binary load address.
    heap_break: u32,

    /// Bump pointer for anonymous mmap allocations.
    mmap_next: u32,

    /// Thread table (TIDs, TLS, once, signal handlers).
    pub threads: ThreadTable,

    /// Symbol name → trampoline address for dlsym lookups.
    /// Populated after the libsystem trampoline is built.
    pub trampoline_map: HashMap<String, u32>,
}

#[derive(Debug, Clone)]
struct FileDescriptor {
    host_path: PathBuf,
    offset: u64,
    writable: bool,
}

impl VirtualFileSystem {
    pub fn new() -> Self {
        let mut vfs = VirtualFileSystem {
            mounts: HashMap::new(),
            file_descriptors: HashMap::new(),
            next_fd: 3,
            heap_break: 0x1000_0000,
            mmap_next: 0x2000_0000,
            threads: ThreadTable::new(),
            trampoline_map: HashMap::new(),
        };

        // Mount standard file descriptors
        vfs.file_descriptors.insert(
            0,
            FileDescriptor {
                host_path: PathBuf::from("/dev/stdin"),
                offset: 0,
                writable: false,
            },
        );
        vfs.file_descriptors.insert(
            1,
            FileDescriptor {
                host_path: PathBuf::from("/dev/stdout"),
                offset: 0,
                writable: true,
            },
        );
        vfs.file_descriptors.insert(
            2,
            FileDescriptor {
                host_path: PathBuf::from("/dev/stderr"),
                offset: 0,
                writable: true,
            },
        );

        vfs
    }

    /// Mount an emulated path to a host path
    pub fn mount(&mut self, emulated: PathBuf, host: PathBuf) -> EmulationResult<()> {
        if !host.exists() {
            return Err(EmulationError::FileSystemError(format!(
                "Host path does not exist: {}",
                host.display()
            )));
        }

        self.mounts.insert(emulated, host);
        Ok(())
    }

    /// Resolve an emulated path to a host path
    pub fn resolve_path(&self, path: &Path) -> EmulationResult<PathBuf> {
        for (emulated, host) in &self.mounts {
            if path.starts_with(emulated) {
                let relative = path.strip_prefix(emulated).unwrap_or(Path::new(""));
                return Ok(host.join(relative));
            }
        }
        Ok(PathBuf::from(path))
    }

    /// Open a file
    pub fn open(&mut self, path: &Path, writable: bool) -> EmulationResult<u32> {
        let host_path = self.resolve_path(path)?;

        if !host_path.exists() && writable {
            std::fs::write(&host_path, [])?;
        } else if !host_path.exists() {
            return Err(EmulationError::FileSystemError(format!(
                "File not found: {}",
                path.display()
            )));
        }

        let fd = self.next_fd;
        self.next_fd += 1;

        self.file_descriptors.insert(
            fd,
            FileDescriptor {
                host_path,
                offset: 0,
                writable,
            },
        );

        Ok(fd)
    }

    /// Read raw bytes from a file descriptor
    pub fn read_bytes(&mut self, fd: u32, size: usize) -> EmulationResult<Vec<u8>> {
        if fd == 0 {
            let mut input = vec![0u8; size];
            let n = std::io::stdin().read(&mut input)?;
            input.truncate(n);
            return Ok(input);
        }

        let file_desc = self
            .file_descriptors
            .get_mut(&fd)
            .ok_or_else(|| EmulationError::FileSystemError(format!("Invalid FD: {}", fd)))?;

        let mut file = File::open(&file_desc.host_path)?;
        file.seek(SeekFrom::Start(file_desc.offset))?;

        let mut buf = vec![0u8; size];
        let n = file.read(&mut buf)?;
        file_desc.offset += n as u64;
        buf.truncate(n);
        Ok(buf)
    }

    /// Write raw bytes to a file descriptor
    pub fn write_bytes(&mut self, fd: u32, data: &[u8]) -> EmulationResult<usize> {
        if fd == 1 {
            std::io::stdout().write_all(data)?;
            std::io::stdout().flush()?;
            return Ok(data.len());
        }

        if fd == 2 {
            std::io::stderr().write_all(data)?;
            std::io::stderr().flush()?;
            return Ok(data.len());
        }

        let file_desc = self
            .file_descriptors
            .get_mut(&fd)
            .ok_or_else(|| EmulationError::FileSystemError(format!("Invalid FD: {}", fd)))?;

        if !file_desc.writable {
            return Err(EmulationError::FileSystemError(format!(
                "FD {} is not writable",
                fd
            )));
        }

        let mut file = OpenOptions::new().write(true).open(&file_desc.host_path)?;
        file.seek(SeekFrom::Start(file_desc.offset))?;
        let written = file.write(data)?;
        file_desc.offset += written as u64;
        Ok(written)
    }

    /// Seek a file descriptor. Returns the new offset.
    ///
    /// whence: 0 = SEEK_SET, 1 = SEEK_CUR, 2 = SEEK_END
    pub fn seek(&mut self, fd: u32, offset: i64, whence: i32) -> EmulationResult<u64> {
        let file_desc = self
            .file_descriptors
            .get_mut(&fd)
            .ok_or_else(|| EmulationError::FileSystemError(format!("Invalid FD: {}", fd)))?;

        let new_offset: u64 = match whence {
            0 => offset as u64,
            1 => (file_desc.offset as i64).wrapping_add(offset) as u64,
            2 => {
                let size = std::fs::metadata(&file_desc.host_path)?.len();
                (size as i64).wrapping_add(offset) as u64
            }
            _ => {
                return Err(EmulationError::FileSystemError(format!(
                    "Invalid lseek whence: {}",
                    whence
                )))
            }
        };

        file_desc.offset = new_offset;
        Ok(new_offset)
    }

    /// Return stat metadata for an open file descriptor.
    pub fn fstat_fd(&self, fd: u32) -> EmulationResult<FileStat> {
        let file_desc = self
            .file_descriptors
            .get(&fd)
            .ok_or_else(|| EmulationError::FileSystemError(format!("Invalid FD: {}", fd)))?;

        let meta = std::fs::metadata(&file_desc.host_path)?;
        Ok(FileStat {
            size: meta.len(),
            is_regular: meta.is_file(),
            is_dir: meta.is_dir(),
        })
    }

    /// Return stat metadata for a path.
    pub fn stat_path(&self, path: &Path) -> EmulationResult<FileStat> {
        let host_path = self.resolve_path(path)?;
        let meta = std::fs::metadata(&host_path)?;
        Ok(FileStat {
            size: meta.len(),
            is_regular: meta.is_file(),
            is_dir: meta.is_dir(),
        })
    }

    /// Duplicate a file descriptor (returns the new fd).
    pub fn dup(&mut self, fd: u32) -> EmulationResult<u32> {
        let desc = self
            .file_descriptors
            .get(&fd)
            .ok_or_else(|| EmulationError::FileSystemError(format!("Invalid FD: {}", fd)))?
            .clone();
        let new_fd = self.next_fd;
        self.next_fd += 1;
        self.file_descriptors.insert(new_fd, desc);
        Ok(new_fd)
    }

    /// Make `to` refer to the same file as `from` (dup2 semantics).
    pub fn dup2(&mut self, from: u32, to: u32) -> EmulationResult<u32> {
        let desc = self
            .file_descriptors
            .get(&from)
            .ok_or_else(|| EmulationError::FileSystemError(format!("Invalid FD: {}", from)))?
            .clone();
        self.file_descriptors.remove(&to);
        self.file_descriptors.insert(to, desc);
        Ok(to)
    }

    /// Close a file descriptor
    pub fn close(&mut self, fd: u32) -> EmulationResult<()> {
        self.file_descriptors
            .remove(&fd)
            .ok_or_else(|| EmulationError::FileSystemError(format!("Invalid FD: {}", fd)))?;
        Ok(())
    }

    /// Create a directory at the given path
    pub fn mkdir(&self, path: &Path) -> EmulationResult<()> {
        let host_path = self.resolve_path(path)?;
        std::fs::create_dir(&host_path)
            .map_err(|e| EmulationError::FileSystemError(format!("mkdir failed: {}", e)))?;
        Ok(())
    }

    /// Remove a file
    pub fn unlink(&self, path: &Path) -> EmulationResult<()> {
        let host_path = self.resolve_path(path)?;
        std::fs::remove_file(&host_path)
            .map_err(|e| EmulationError::FileSystemError(format!("unlink failed: {}", e)))?;
        Ok(())
    }

    /// Remove an empty directory
    pub fn rmdir(&self, path: &Path) -> EmulationResult<()> {
        let host_path = self.resolve_path(path)?;
        std::fs::remove_dir(&host_path)
            .map_err(|e| EmulationError::FileSystemError(format!("rmdir failed: {}", e)))?;
        Ok(())
    }

    // ── Heap / mmap ──────────────────────────────────────────────────────────

    /// Set or query the program break. Passing 0 queries the current break.
    /// Returns the (possibly new) program break.
    pub fn brk(&mut self, new_break: u32) -> u32 {
        if new_break > self.heap_break {
            self.heap_break = new_break;
        }
        self.heap_break
    }

    /// Allocate an anonymous region of `len` bytes and return its base address.
    /// Unicorn already has the full 2 GB mapped, so we just bump the pointer.
    pub fn mmap_anon(&mut self, len: u32) -> EmulationResult<u32> {
        if len == 0 {
            return Err(EmulationError::MemoryError(
                "mmap: zero-length mapping".into(),
            ));
        }
        let addr = self.mmap_next;
        // Round up to page boundary (4096 bytes)
        let aligned_len = (len + 0xFFF) & !0xFFF;
        self.mmap_next = self
            .mmap_next
            .checked_add(aligned_len)
            .ok_or_else(|| EmulationError::MemoryError("mmap: address space exhausted".into()))?;
        Ok(addr)
    }
}

impl Default for VirtualFileSystem {
    fn default() -> Self {
        Self::new()
    }
}
