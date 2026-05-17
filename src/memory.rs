use crate::errors::{EmulationError, EmulationResult};
use std::collections::BTreeMap;

/// Virtual memory manager for the i386 emulation environment
pub struct MemoryManager {
    /// Memory regions indexed by virtual address
    regions: BTreeMap<u32, MemoryRegion>,
    
    /// Total memory available
    total_memory: usize,
}

#[derive(Debug, Clone)]
pub struct MemoryRegion {
    pub base: u32,
    pub size: u32,
    pub data: Vec<u8>,
    pub permissions: MemoryPermissions,
}

#[derive(Debug, Clone, Copy)]
pub struct MemoryPermissions {
    pub readable: bool,
    pub writable: bool,
    pub executable: bool,
}

impl MemoryManager {
    /// Create a new memory manager with specified total memory
    pub fn new(total_memory: usize) -> Self {
        MemoryManager {
            regions: BTreeMap::new(),
            total_memory,
        }
    }

    /// Allocate a memory region
    pub fn allocate(
        &mut self,
        size: u32,
        permissions: MemoryPermissions,
    ) -> EmulationResult<u32> {
        // Find a suitable base address
        let base = if self.regions.is_empty() {
            0x1000 // Start at 4KB
        } else {
            let last = self.regions.iter().last().unwrap();
            (last.0 + last.1.size + 0xFFF) & !0xFFF // Align to page boundary
        };

        if (base as usize + size as usize) > self.total_memory {
            return Err(EmulationError::MemoryError(
                "Insufficient memory for allocation".to_string(),
            ));
        }

        let region = MemoryRegion {
            base,
            size,
            data: vec![0; size as usize],
            permissions,
        };

        self.regions.insert(base, region);
        Ok(base)
    }

    /// Read from memory
    pub fn read(&self, addr: u32, size: usize) -> EmulationResult<Vec<u8>> {
        let region = self.find_region(addr)?;

        if !region.permissions.readable {
            return Err(EmulationError::MemoryError(format!(
                "Attempted to read from non-readable region at 0x{:x}",
                addr
            )));
        }

        let offset = (addr - region.base) as usize;
        if offset + size > region.data.len() {
            return Err(EmulationError::MemoryError(
                "Read out of bounds".to_string(),
            ));
        }

        Ok(region.data[offset..offset + size].to_vec())
    }

    /// Write to memory
    pub fn write(&mut self, addr: u32, data: &[u8]) -> EmulationResult<()> {
        let region = self.find_region_mut(addr)?;

        if !region.permissions.writable {
            return Err(EmulationError::MemoryError(format!(
                "Attempted to write to non-writable region at 0x{:x}",
                addr
            )));
        }

        let offset = (addr - region.base) as usize;
        if offset + data.len() > region.data.len() {
            return Err(EmulationError::MemoryError(
                "Write out of bounds".to_string(),
            ));
        }

        region.data[offset..offset + data.len()].copy_from_slice(data);
        Ok(())
    }

    /// Find a memory region for a given address
    fn find_region(&self, addr: u32) -> EmulationResult<&MemoryRegion> {
        self.regions
            .iter()
            .find(|(_, region)| addr >= region.base && addr < region.base + region.size)
            .map(|(_, region)| region)
            .ok_or_else(|| EmulationError::MemoryError(format!("Invalid address: 0x{:x}", addr)))
    }

    /// Find a mutable memory region for a given address
    fn find_region_mut(&mut self, addr: u32) -> EmulationResult<&mut MemoryRegion> {
        self.regions
            .iter_mut()
            .find(|(_, region)| addr >= region.base && addr < region.base + region.size)
            .map(|(_, region)| region)
            .ok_or_else(|| EmulationError::MemoryError(format!("Invalid address: 0x{:x}", addr)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_allocation() {
        let mut mm = MemoryManager::new(1024 * 1024);
        let perms = MemoryPermissions {
            readable: true,
            writable: true,
            executable: false,
        };
        
        let addr = mm.allocate(4096, perms).unwrap();
        assert_eq!(addr, 0x1000);
    }
}
