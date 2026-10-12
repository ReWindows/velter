//! `dp` / `dp-`: attach as a debugger and detach without killing the game.
//!
//! Win32 ties a debuggee to the *thread* that attached, so a dedicated thread
//! owns the whole session: it attaches, pumps debug events, and detaches.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use windows_sys::Win32::Foundation::{
    CloseHandle, DBG_CONTINUE, DBG_EXCEPTION_NOT_HANDLED, EXCEPTION_BREAKPOINT,
    EXCEPTION_SINGLE_STEP, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    CONTEXT, CONTEXT_DEBUG_REGISTERS_AMD64, CREATE_PROCESS_DEBUG_EVENT, CREATE_THREAD_DEBUG_EVENT,
    ContinueDebugEvent, DEBUG_EVENT, DebugActiveProcess as d_a_p,
    DebugActiveProcessStop as d_a_p_s, DebugSetProcessKillOnExit as d_s_p_k_o_e,
    EXCEPTION_DEBUG_EVENT, EXIT_PROCESS_DEBUG_EVENT, LOAD_DLL_DEBUG_EVENT, SetThreadContext,
    WaitForDebugEvent,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows_sys::Win32::System::Threading::{
    OpenThread, ResumeThread, SuspendThread, THREAD_GET_CONTEXT, THREAD_SET_CONTEXT,
    THREAD_SUSPEND_RESUME,
};

use crate::{Error, Result, last_error};

pub struct Debugger {
    pub pid: u32,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<()>>>,
    /// Human-readable event log, newest last.
    pub log: Arc<Mutex<Vec<String>>>,
    commands: mpsc::Sender<Command>,
}

#[derive(Clone, Copy)]
enum Command {
    Watch { addr: u64, size: usize },
    ClearWatch,
}

impl Debugger {
    /// `dp <pid>`
    pub fn attach(pid: u32) -> Result<Debugger> {
        let stop = Arc::new(AtomicBool::new(false));
        let log = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel();
        let (command_tx, command_rx) = mpsc::channel();
        let thread = {
            let (stop, log) = (stop.clone(), log.clone());
            std::thread::Builder::new()
                .name(format!("dbg-{pid}"))
                .spawn(move || session(pid, stop, log, tx, command_rx))
                .map_err(|e| Error::Other(e.to_string()))?
        };
        match rx.recv() {
            Ok(Ok(())) => Ok(Debugger {
                pid,
                stop,
                thread: Some(thread),
                log,
                commands: command_tx,
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err(Error::Other("debug thread died during attach".into())),
        }
    }

    pub fn is_alive(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Set DR0 on every thread to break after writes to this address.
    pub fn watch_writes(&self, addr: u64, size: usize) -> Result<()> {
        if !matches!(size, 1 | 2 | 4 | 8) {
            return Err(Error::Other("watch size must be 1, 2, 4, or 8".into()));
        }
        self.commands
            .send(Command::Watch { addr, size })
            .map_err(|_| Error::Other("debugger command channel closed".into()))
    }

    pub fn clear_watch(&self) -> Result<()> {
        self.commands
            .send(Command::ClearWatch)
            .map_err(|_| Error::Other("debugger command channel closed".into()))
    }

    /// `dp-`: detach and leave the game running.
    pub fn detach(mut self) -> Result<()> {
        self.stop_thread()
    }

    fn stop_thread(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::SeqCst);
        match self.thread.take() {
            Some(t) => t
                .join()
                .unwrap_or_else(|_| Err(Error::Other("debug thread panicked".into()))),
            None => Ok(()),
        }
    }
}

impl Drop for Debugger {
    fn drop(&mut self) {
        let _ = self.stop_thread();
    }
}

fn session(
    pid: u32,
    stop: Arc<AtomicBool>,
    log: Arc<Mutex<Vec<String>>>,
    ready: mpsc::Sender<Result<()>>,
    commands: mpsc::Receiver<Command>,
) -> Result<()> {
    unsafe {
        if d_a_p(pid) == 0 {
            let _ = ready.send(Err(last_error("DebugActiveProcess")));
            return Ok(());
        }
        // Never take the game down with us if this thread exits.
        d_s_p_k_o_e(0);
        let _ = ready.send(Ok(()));

        let push = |s: String| {
            let mut l = log.lock().unwrap();
            if l.len() > 500 {
                l.drain(..250);
            }
            l.push(s);
        };
        push(format!("attached to {pid}"));

        let mut ev: DEBUG_EVENT = std::mem::zeroed();
        let mut watch = None;
        while !stop.load(Ordering::SeqCst) {
            while let Ok(cmd) = commands.try_recv() {
                watch = match cmd {
                    Command::Watch { addr, size } => Some((addr, size)),
                    Command::ClearWatch => None,
                };
                match set_watch_all(pid, watch) {
                    Ok(n) => push(match watch {
                        Some((a, s)) => format!("watching {s} bytes @ 0x{a:x} on {n} threads"),
                        None => format!("watch cleared on {n} threads"),
                    }),
                    Err(e) => push(format!("watchpoint error: {e}")),
                }
            }
            if WaitForDebugEvent(&mut ev, 50) == 0 {
                continue;
            }
            let mut status = DBG_CONTINUE;
            match ev.dwDebugEventCode {
                CREATE_THREAD_DEBUG_EVENT => {
                    if let Some(w) = watch {
                        let _ = set_watch_thread(ev.u.CreateThread.hThread, Some(w));
                    }
                }
                CREATE_PROCESS_DEBUG_EVENT => {
                    let info = ev.u.CreateProcessInfo;
                    if !info.hFile.is_null() {
                        CloseHandle(info.hFile);
                    }
                }
                LOAD_DLL_DEBUG_EVENT => {
                    let info = ev.u.LoadDll;
                    if !info.hFile.is_null() {
                        CloseHandle(info.hFile);
                    }
                }
                EXCEPTION_DEBUG_EVENT => {
                    let rec = ev.u.Exception.ExceptionRecord;
                    if rec.ExceptionCode == EXCEPTION_SINGLE_STEP && watch.is_some() {
                        push(format!(
                            "WRITE HIT @ 0x{:x} · thread {}",
                            rec.ExceptionAddress as u64, ev.dwThreadId
                        ));
                    } else if rec.ExceptionCode != EXCEPTION_BREAKPOINT {
                        // Mono/Unity raise first-chance exceptions all the time (null refs
                        // via access violations etc.); hand them back to the game.
                        status = DBG_EXCEPTION_NOT_HANDLED;
                    } else {
                        push(format!("breakpoint @ 0x{:x}", rec.ExceptionAddress as u64));
                    }
                }
                EXIT_PROCESS_DEBUG_EVENT => {
                    push("process exited".into());
                    ContinueDebugEvent(ev.dwProcessId, ev.dwThreadId, DBG_CONTINUE);
                    return Ok(());
                }
                _ => {}
            }
            ContinueDebugEvent(ev.dwProcessId, ev.dwThreadId, status);
        }

        if d_a_p_s(pid) == 0 {
            return Err(last_error("DebugActiveProcessStop"));
        }
        push(format!("detached from {pid}"));
        Ok(())
    }
}

fn set_watch_thread(
    thread: windows_sys::Win32::Foundation::HANDLE,
    watch: Option<(u64, usize)>,
) -> Result<()> {
    unsafe {
        let mut context = CONTEXT::default();
        context.ContextFlags = CONTEXT_DEBUG_REGISTERS_AMD64;
        if let Some((addr, size)) = watch {
            context.Dr0 = addr;
            let len = match size {
                1 => 0,
                2 => 1,
                4 => 3,
                8 => 2,
                _ => unreachable!(),
            };
            context.Dr7 = 1 | (1 << 16) | (len << 18); // L0 + write + length
        }
        if SetThreadContext(thread, &context) == 0 {
            Err(last_error("SetThreadContext"))
        } else {
            Ok(())
        }
    }
}

fn set_watch_all(pid: u32, watch: Option<(u64, usize)>) -> Result<usize> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snap == INVALID_HANDLE_VALUE {
            return Err(last_error("CreateToolhelp32Snapshot"));
        }
        let mut entry = THREADENTRY32::default();
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut ok = Thread32First(snap, &mut entry);
        let mut count = 0;
        while ok != 0 {
            if entry.th32OwnerProcessID == pid {
                let rights = THREAD_GET_CONTEXT | THREAD_SET_CONTEXT | THREAD_SUSPEND_RESUME;
                let thread = OpenThread(rights, 0, entry.th32ThreadID);
                if !thread.is_null() {
                    SuspendThread(thread);
                    if set_watch_thread(thread, watch).is_ok() {
                        count += 1;
                    }
                    ResumeThread(thread);
                    CloseHandle(thread);
                }
            }
            ok = Thread32Next(snap, &mut entry);
        }
        CloseHandle(snap);
        Ok(count)
    }
}
