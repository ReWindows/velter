use std::ffi::c_void;
use std::mem::size_of;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, STILL_ACTIVE};
use windows_sys::Win32::System::Diagnostics::Debug::{
    FlushInstructionCache, ReadProcessMemory, WriteProcessMemory,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW, PROCESSENTRY32W,
    Process32FirstW, Process32NextW, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Memory::{
    MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READWRITE, VirtualProtectEx, VirtualQueryEx,
};
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_ALL_ACCESS};

use crate::{Result, last_error};

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtSuspendProcess(process: HANDLE) -> i32;
    fn NtResumeProcess(process: HANDLE) -> i32;
}

#[derive(Clone, Debug)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
}

pub fn list_processes() -> Vec<ProcessInfo> {
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e);
        while ok != 0 {
            out.push(ProcessInfo {
                pid: e.th32ProcessID,
                name: wstr(&e.szExeFile),
            });
            ok = Process32NextW(snap, &mut e);
        }
        CloseHandle(snap);
    }
    out
}

#[derive(Clone, Debug)]
pub struct Module {
    pub name: String,
    pub base: u64,
    pub size: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Mono,
    Il2Cpp,
    Unknown,
}

/// An opened process. Owns the handle; closed on drop.
pub struct Process {
    pub pid: u32,
    pub name: String,
    handle: HANDLE,
}

// The handle is a kernel object reference; Win32 calls on it are thread-safe.
unsafe impl Send for Process {}
unsafe impl Sync for Process {}

impl Process {
    pub fn open(pid: u32, name: impl Into<String>) -> Result<Process> {
        let handle = unsafe { OpenProcess(PROCESS_ALL_ACCESS, 0, pid) };
        if handle.is_null() {
            return Err(last_error("OpenProcess"));
        }
        Ok(Process {
            pid,
            name: name.into(),
            handle,
        })
    }

    pub fn handle(&self) -> HANDLE {
        self.handle
    }

    /// True while the target has not exited. A process handle remains valid after exit,
    /// so callers must check this before treating reads as live data.
    pub fn is_alive(&self) -> bool {
        let mut code = 0u32;
        unsafe { GetExitCodeProcess(self.handle, &mut code) != 0 && code == STILL_ACTIVE as u32 }
    }

    /// Reads as many bytes as possible into `buf`; returns how many were read.
    pub fn read(&self, addr: u64, buf: &mut [u8]) -> usize {
        let mut n = 0usize;
        unsafe {
            ReadProcessMemory(
                self.handle,
                addr as *const c_void,
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut n,
            );
        }
        n
    }

    pub fn read_bits(&self, addr: u64, size: usize) -> Option<u64> {
        let mut b = [0u8; 8];
        (self.read(addr, &mut b[..size]) == size).then(|| u64::from_le_bytes(b))
    }

    /// Writes `data`, temporarily making the page writable if needed (code patches).
    pub fn write(&self, addr: u64, data: &[u8]) -> Result<()> {
        unsafe {
            let p = addr as *const c_void;
            let mut n = 0usize;
            if WriteProcessMemory(self.handle, p, data.as_ptr().cast(), data.len(), &mut n) != 0
                && n == data.len()
            {
                return Ok(());
            }
            let mut old = 0u32;
            if VirtualProtectEx(self.handle, p, data.len(), PAGE_EXECUTE_READWRITE, &mut old) == 0 {
                return Err(last_error("VirtualProtectEx"));
            }
            let ok = WriteProcessMemory(self.handle, p, data.as_ptr().cast(), data.len(), &mut n);
            let err = (ok == 0).then(|| last_error("WriteProcessMemory"));
            VirtualProtectEx(self.handle, p, data.len(), old, &mut old);
            FlushInstructionCache(self.handle, p, data.len());
            err.map_or(Ok(()), Err)
        }
    }

    pub fn suspend(&self) -> Result<()> {
        let st = unsafe { NtSuspendProcess(self.handle) };
        if st < 0 {
            Err(crate::Error::Other(format!("NtSuspendProcess: 0x{st:08x}")))
        } else {
            Ok(())
        }
    }

    pub fn resume(&self) -> Result<()> {
        let st = unsafe { NtResumeProcess(self.handle) };
        if st < 0 {
            Err(crate::Error::Other(format!("NtResumeProcess: 0x{st:08x}")))
        } else {
            Ok(())
        }
    }

    pub fn query(&self, addr: u64) -> Option<MEMORY_BASIC_INFORMATION> {
        unsafe {
            let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
            let n = VirtualQueryEx(
                self.handle,
                addr as *const c_void,
                &mut mbi,
                size_of::<MEMORY_BASIC_INFORMATION>(),
            );
            (n != 0).then_some(mbi)
        }
    }

    pub fn modules(&self) -> Vec<Module> {
        let mut out = Vec::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, self.pid);
            if snap == INVALID_HANDLE_VALUE {
                return out;
            }
            let mut e: MODULEENTRY32W = std::mem::zeroed();
            e.dwSize = size_of::<MODULEENTRY32W>() as u32;
            let mut ok = Module32FirstW(snap, &mut e);
            while ok != 0 {
                out.push(Module {
                    name: wstr(&e.szModule),
                    base: e.modBaseAddr as u64,
                    size: e.modBaseSize as u64,
                });
                ok = Module32NextW(snap, &mut e);
            }
            CloseHandle(snap);
        }
        out
    }

    pub fn backend(&self) -> Backend {
        let mods = self.modules();
        let has = |n: &str| mods.iter().any(|m| m.name.eq_ignore_ascii_case(n));
        if has("GameAssembly.dll") {
            Backend::Il2Cpp
        } else if mods
            .iter()
            .any(|m| m.name.to_ascii_lowercase().starts_with("mono"))
        {
            Backend::Mono
        } else {
            Backend::Unknown
        }
    }

    /// Module containing `addr`, as "module+0xoffset".
    pub fn symbolize(&self, mods: &[Module], addr: u64) -> String {
        mods.iter()
            .find(|m| addr >= m.base && addr < m.base + m.size)
            .map(|m| format!("{}+0x{:x}", m.name, addr - m.base))
            .unwrap_or_else(|| format!("0x{addr:x}"))
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}

fn wstr(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}
