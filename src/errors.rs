use thiserror::Error;

#[derive(Error, Debug)]
pub enum EmulationError {
    #[error("Binary loading failed: {0}")]
    BinaryLoadError(String),

    #[error("Invalid i386 architecture: {0}")]
    InvalidArchitecture(String),

    #[error("Emulation error: {0}")]
    EmulationError(String),

    #[error("Syscall error: {0}")]
    SyscallError(String),

    #[error("Memory error: {0}")]
    MemoryError(String),

    #[error("Process creation failed: {0}")]
    ProcessError(String),

    #[error("File system error: {0}")]
    FileSystemError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Unknown error: {0}")]
    Unknown(String),
}

pub type EmulationResult<T> = Result<T, EmulationError>;
