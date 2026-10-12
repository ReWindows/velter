//! Capability-driven interface for local processes, emulators, remote agents,
//! and future physical transports.

use std::fmt;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Capabilities(u64);

impl Capabilities {
    pub const READ_MEMORY: Self = Self(1 << 0);
    pub const WRITE_MEMORY: Self = Self(1 << 1);
    pub const PAUSE: Self = Self(1 << 2);
    pub const RESUME: Self = Self(1 << 3);
    pub const STEP: Self = Self(1 << 4);
    pub const BREAKPOINTS: Self = Self(1 << 5);
    pub const WATCHPOINTS: Self = Self(1 << 6);
    pub const REGISTERS: Self = Self(1 << 7);
    pub const MODULES: Self = Self(1 << 8);
    pub const THREADS: Self = Self(1 << 9);
    pub const fn empty() -> Self {
        Self(0)
    }
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryRegion {
    pub base: u64,
    pub size: u64,
    pub readable: bool,
    pub writable: bool,
    pub executable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Module {
    pub name: String,
    pub base: u64,
    pub size: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Thread {
    pub id: u64,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisterValue {
    pub name: String,
    pub bytes_le: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StopReason {
    Requested,
    Breakpoint(u64),
    Watchpoint(u64),
    Exception(u32),
    Exited(u32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TargetError {
    Unsupported(&'static str),
    InvalidAddress(u64),
    InvalidState(&'static str),
    Transport(String),
    PartialRead { expected: usize, actual: usize },
}

impl fmt::Display for TargetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for TargetError {}

pub trait Target {
    fn capabilities(&self) -> Capabilities;
    fn read_memory(&mut self, address: u64, output: &mut [u8]) -> Result<(), TargetError>;
    fn write_memory(&mut self, address: u64, bytes: &[u8]) -> Result<(), TargetError>;
    fn pause(&mut self) -> Result<StopReason, TargetError>;
    fn resume(&mut self) -> Result<(), TargetError>;
    fn step(&mut self, _thread: u64) -> Result<StopReason, TargetError> {
        Err(TargetError::Unsupported("single step"))
    }
    fn memory_map(&mut self) -> Result<Vec<MemoryRegion>, TargetError> {
        Err(TargetError::Unsupported("memory map"))
    }
    fn modules(&mut self) -> Result<Vec<Module>, TargetError> {
        Err(TargetError::Unsupported("modules"))
    }
    fn threads(&mut self) -> Result<Vec<Thread>, TargetError> {
        Err(TargetError::Unsupported("threads"))
    }
    fn registers(&mut self, _thread: u64) -> Result<Vec<RegisterValue>, TargetError> {
        Err(TargetError::Unsupported("registers"))
    }
    fn set_breakpoint(&mut self, _address: u64) -> Result<(), TargetError> {
        Err(TargetError::Unsupported("breakpoints"))
    }
    fn set_watchpoint(&mut self, _address: u64, _size: u8) -> Result<(), TargetError> {
        Err(TargetError::Unsupported("watchpoints"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Mock {
        memory: Vec<u8>,
        paused: bool,
    }
    impl Target for Mock {
        fn capabilities(&self) -> Capabilities {
            Capabilities::READ_MEMORY
                .union(Capabilities::WRITE_MEMORY)
                .union(Capabilities::PAUSE)
                .union(Capabilities::RESUME)
        }
        fn read_memory(&mut self, address: u64, output: &mut [u8]) -> Result<(), TargetError> {
            let start = address as usize;
            let source = self
                .memory
                .get(start..start + output.len())
                .ok_or(TargetError::InvalidAddress(address))?;
            output.copy_from_slice(source);
            Ok(())
        }
        fn write_memory(&mut self, address: u64, bytes: &[u8]) -> Result<(), TargetError> {
            let start = address as usize;
            let target = self
                .memory
                .get_mut(start..start + bytes.len())
                .ok_or(TargetError::InvalidAddress(address))?;
            target.copy_from_slice(bytes);
            Ok(())
        }
        fn pause(&mut self) -> Result<StopReason, TargetError> {
            self.paused = true;
            Ok(StopReason::Requested)
        }
        fn resume(&mut self) -> Result<(), TargetError> {
            self.paused = false;
            Ok(())
        }
    }
    #[test]
    fn mock_target_round_trip() {
        let mut target = Mock {
            memory: vec![0; 8],
            paused: false,
        };
        target.write_memory(2, &[0xaa, 0xbb]).unwrap();
        let mut bytes = [0; 2];
        target.read_memory(2, &mut bytes).unwrap();
        assert_eq!(bytes, [0xaa, 0xbb]);
        assert_eq!(target.pause().unwrap(), StopReason::Requested);
        assert!(target.paused);
    }
}
