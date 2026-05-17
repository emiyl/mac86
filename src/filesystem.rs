use crate::errors::{EmulationError, EmulationResult};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Virtual filesystem for the emulation environment
pub struct VirtualFileSystem {
    /// Mapping of emulated paths to host paths
    mounts: HashMap<PathBuf, PathBuf>,
    
    /// Open file descriptors
    file_descriptors: HashMap<u32, FileDescriptor>,
    
    /// Next available file descriptor
    next_fd: u32,
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
            next_fd: 3, // stdin, stdout, stderr are 0, 1, 2
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
        // Check if path matches any mount point
        for (emulated, host) in &self.mounts {
            if path.starts_with(emulated) {
                let relative = path.strip_prefix(emulated).unwrap_or(Path::new(""));
                return Ok(host.join(relative));
            }
        }

        // Default: treat as relative to current directory
        Ok(PathBuf::from(path))
    }

    /// Open a file
    pub fn open(&mut self, path: &Path, writable: bool) -> EmulationResult<u32> {
        let host_path = self.resolve_path(path)?;

        if !host_path.exists() && writable {
            // Create file if it doesn't exist
            std::fs::write(&host_path, &[])?;
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

    /// Close a file descriptor
    pub fn close(&mut self, fd: u32) -> EmulationResult<()> {
        self.file_descriptors
            .remove(&fd)
            .ok_or_else(|| EmulationError::FileSystemError(format!("Invalid FD: {}", fd)))?;
        Ok(())
    }

    /// Read from a file descriptor
    pub fn read(&mut self, fd: u32, _buf_addr: u32, size: usize) -> EmulationResult<usize> {
        let file_desc = self
            .file_descriptors
            .get_mut(&fd)
            .ok_or_else(|| EmulationError::FileSystemError(format!("Invalid FD: {}", fd)))?;

        let content = std::fs::read(&file_desc.host_path)?;
        let to_read = std::cmp::min(size, content.len() - file_desc.offset as usize);

        // In a real implementation, write to emulated memory at buf_addr
        file_desc.offset += to_read as u64;

        Ok(to_read)
    }

    /// Write to a file descriptor
    pub fn write(&mut self, fd: u32, _buf_addr: u32, size: usize) -> EmulationResult<usize> {
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

        // In a real implementation, read from emulated memory at buf_addr
        file_desc.offset += size as u64;

        Ok(size)
    }
}

impl Default for VirtualFileSystem {
    fn default() -> Self {
        Self::new()
    }
}
