//! velter-core: process access, debugger attach, memory scanning.

pub mod cfg;
pub mod debugger;
pub mod disasm;
pub mod process;
pub mod scan;
pub mod value;

pub use cfg::{BasicBlock, control_flow_graph};
pub use debugger::Debugger;
pub use disasm::{Instruction, disassemble};
pub use process::{Backend, Module, Process, ProcessInfo, list_processes};
pub use scan::{Region, ScanCmp, Scanner};
pub use value::{Value, ValueType};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{what} failed (win32 error {code})")]
    Win32 { what: &'static str, code: u32 },
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn last_error(what: &'static str) -> Error {
    let code = unsafe { windows_sys::Win32::Foundation::GetLastError() };
    Error::Win32 { what, code }
}
