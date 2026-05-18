use crate::errors::{EmulationError, EmulationResult};
use std::path::PathBuf;

/// Flags controlling diagnostic output for an emulation run.
#[derive(Debug, Clone, Copy, Default)]
pub struct TraceConfig {
    /// Print syscall number and arguments at each INT 0x80.
    pub syscalls: bool,
    /// Print every instruction address as it executes.
    pub instructions: bool,
}

/// Top-level emulation context — carries configuration and prepares the
/// host environment before handing off to `Process`.
pub struct EmulationContext {
    /// Optional path for emulation-environment files (SDK root, cached libs).
    /// The directory is created on startup; currently used only as a future
    /// hook for multi-binary environments.
    env_path: PathBuf,
    trace_config: TraceConfig,
    initialized: bool,
}

impl EmulationContext {
    pub fn new(env_path: Option<PathBuf>, trace_config: TraceConfig) -> EmulationResult<Self> {
        let env_path = env_path.unwrap_or_else(|| {
            let mut p = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
            p.push("mac86_env");
            p
        });

        std::fs::create_dir_all(&env_path).map_err(EmulationError::IoError)?;

        Ok(EmulationContext {
            env_path,
            trace_config,
            initialized: false,
        })
    }

    /// Prepare for process execution.  Safe to call multiple times.
    pub fn initialize(&mut self) -> EmulationResult<()> {
        self.initialized = true;
        Ok(())
    }

    pub fn trace(&self) -> TraceConfig {
        self.trace_config
    }
}

impl Drop for EmulationContext {
    fn drop(&mut self) {
        if self.initialized {
            log::debug!("EmulationContext dropped (env: {})", self.env_path.display());
        }
    }
}
