use crate::errors::{EmulationError, EmulationResult};
use crate::threads::ThreadTable;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub struct FileStat {
    pub size: u64,
    pub is_file: bool,
    pub is_dir: bool,
    pub ino: u64,
}

/// Encode a FileStat into the 120-byte i386 INODE64 stat struct layout.
///
/// macOS 32-bit binaries using `stat$INODE64` / `fstat$INODE64` expect:
///   offset  0: st_dev      (u32)
///   offset  4: st_mode     (u16)  ← NOT at offset 8 like the old non-INODE64 layout
///   offset  6: st_nlink    (u16)
///   offset  8: st_ino      (u64)  ← 64-bit inode
///   offset 16: st_uid      (u32)
///   offset 20: st_gid      (u32)
///   offset 24: st_rdev     (u32)
///   offset 28: (4-byte pad before timespec)
///   offset 32: st_atimespec (8 bytes)
///   offset 40: st_mtimespec (8 bytes)
///   offset 48: st_ctimespec (8 bytes)
///   offset 56: st_birthtimespec (8 bytes)
///   offset 64: st_size     (u64)  ← NOT at offset 32
///   offset 72: st_blocks   (u64)
///   offset 80: st_blksize  (u32)
///   offset 84: st_flags    (u32)
///   offset 88: st_gen      (u32)
///   Total: ~112 bytes (buf is 120 for headroom)
pub fn encode_stat_i386(st: &FileStat, buf: &mut [u8]) {
    assert!(buf.len() >= 112);

    buf.fill(0);

    let mode: u16 = if st.is_dir {
        0o040755 // S_IFDIR | rwxr-xr-x
    } else {
        0o100644 // S_IFREG | rw-r--r--
    };

    let w16 = |buf: &mut [u8], off: usize, v: u16| {
        buf[off..off + 2].copy_from_slice(&v.to_le_bytes());
    };
    let w32 = |buf: &mut [u8], off: usize, v: u32| {
        buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
    };
    let w64 = |buf: &mut [u8], off: usize, v: u64| {
        buf[off..off + 8].copy_from_slice(&v.to_le_bytes());
    };

    w32(buf, 0, 1);          // st_dev
    w16(buf, 4, mode);       // st_mode
    w16(buf, 6, 1);          // st_nlink
    w64(buf, 8, st.ino);     // st_ino (64-bit)
    w32(buf, 16, 501);       // st_uid
    w32(buf, 20, 20);        // st_gid
    w32(buf, 24, 0);         // st_rdev
    // offsets 28-63: timestamps (left as zero)
    w64(buf, 64, st.size);   // st_size
    w64(buf, 72, st.size / 512 + if st.size % 512 != 0 { 1 } else { 0 }); // st_blocks
    w32(buf, 80, 4096);      // st_blksize
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
            is_file: meta.is_file(),
            is_dir: meta.is_dir(),
            ino: meta.ino(),
        })
    }

    /// Return stat metadata for a path.
    pub fn stat_path(&self, path: &Path) -> EmulationResult<FileStat> {
        let host_path = self.resolve_path(path)?;
        let meta = std::fs::metadata(&host_path)?;
        Ok(FileStat {
            size: meta.len(),
            is_file: meta.is_file(),
            is_dir: meta.is_dir(),
            ino: meta.ino(),
        })
    }

    /// Return stat metadata for a path, without following symlinks.
    pub fn lstat_path(&self, path: &Path) -> EmulationResult<FileStat> {
        let host_path = self.resolve_path(path)?;
        let meta = std::fs::symlink_metadata(&host_path)?;
        Ok(FileStat {
            size: meta.len(),
            is_file: meta.is_file(),
            is_dir: meta.is_dir(),
            ino: meta.ino(),
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

    /// Move a file from src to dst
    pub fn rename(&self, src: &Path, dst: &Path) -> EmulationResult<()> {
        let host_src = self.resolve_path(src)?;
        let host_dst = self.resolve_path(dst)?;
        std::fs::rename(&host_src, &host_dst)
            .map_err(|e| EmulationError::FileSystemError(format!("rename failed: {}", e)))?;
        Ok(())
    }

    /// Copy a file from src to dst
    pub fn copyfile(&self, src: &Path, dst: &Path) -> EmulationResult<()> {
        let host_src = self.resolve_path(src)?;
        let host_dst = self.resolve_path(dst)?;
        std::fs::copy(&host_src, &host_dst)
            .map_err(|e| EmulationError::FileSystemError(format!("copyfile failed: {}", e)))?;
        Ok(())
    }

    /// Copy a file from src_fd to dst_fd (fcopyfile semantics). Both FDs must be open and dst_fd must be writable.
    pub fn fcopyfile(&self, src_fd: u32, dst_fd: u32) -> EmulationResult<()> {
        let src_desc = self
            .file_descriptors
            .get(&src_fd)
            .ok_or_else(|| EmulationError::FileSystemError(format!("Invalid FD: {}", src_fd)))?;
        let dst_desc = self
            .file_descriptors
            .get(&dst_fd)
            .ok_or_else(|| EmulationError::FileSystemError(format!("Invalid FD: {}", dst_fd)))?;

        if !dst_desc.writable {
            return Err(EmulationError::FileSystemError(format!(
                "FD {} is not writable",
                dst_fd
            )));
        }

        let mut src_file = File::open(&src_desc.host_path)?;
        let mut dst_file = OpenOptions::new().write(true).open(&dst_desc.host_path)?;

        std::io::copy(&mut src_file, &mut dst_file)?;
        Ok(())
    }

    /// Change permissions of a file
    pub fn chmod(&self, path: &Path, mode: u32) -> EmulationResult<()> {
        let host_path = self.resolve_path(path)?;
        let mut perms = std::fs::metadata(&host_path)?.permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(mode);
            std::fs::set_permissions(&host_path, perms)
                .map_err(|e| EmulationError::FileSystemError(format!("chmod failed: {}", e)))?;
        }
        #[cfg(not(unix))]
        {
            return Err(EmulationError::FileSystemError(
                "chmod not supported on this platform".into(),
            ));
        }
        Ok(())
    }

    pub fn fchmod(&self, fd: u32, mode: u32) -> EmulationResult<()> {
        let file_desc = self
            .file_descriptors
            .get(&fd)
            .ok_or_else(|| EmulationError::FileSystemError(format!("Invalid FD: {}", fd)))?;

        let host_path = &file_desc.host_path;
        let mut perms = std::fs::metadata(host_path)?.permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(mode);
            std::fs::set_permissions(host_path, perms)
                .map_err(|e| EmulationError::FileSystemError(format!("fchmod failed: {}", e)))?;
        }
        #[cfg(not(unix))]
        {
            return Err(EmulationError::FileSystemError(
                "fchmod not supported on this platform".into(),
            ));
        }
        Ok(())
    }

    /// Change ownership of a file (chown semantics). Not fully supported in emulation - will only succeed if the requested UID/GID matches the current owner.
    pub fn chown(&self, path: &Path, uid: u32, gid: u32) -> EmulationResult<()> {
        let host_path = self.resolve_path(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let meta = std::fs::metadata(&host_path)?;
            let current_uid = meta.uid();
            let current_gid = meta.gid();
            if uid != current_uid || gid != current_gid {
                return Err(EmulationError::FileSystemError(
                    "chown failed: changing ownership not supported in this emulation".into(),
                ));
            }
        }
        #[cfg(not(unix))]
        {
            return Err(EmulationError::FileSystemError(
                "chown not supported on this platform".into(),
            ));
        }
        Ok(())
    }

    pub fn fchown(&self, fd: u32, uid: u32, gid: u32) -> EmulationResult<()> {
        let file_desc = self
            .file_descriptors
            .get(&fd)
            .ok_or_else(|| EmulationError::FileSystemError(format!("Invalid FD: {}", fd)))?;

        let host_path = &file_desc.host_path;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let meta = std::fs::metadata(host_path)?;
            let current_uid = meta.uid();
            let current_gid = meta.gid();
            if uid != current_uid || gid != current_gid {
                return Err(EmulationError::FileSystemError(
                    "fchown failed: changing ownership not supported in this emulation".into(),
                ));
            }
        }
        #[cfg(not(unix))]
        {
            return Err(EmulationError::FileSystemError(
                "fchown not supported on this platform".into(),
            ));
        }
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
