use std::io::{self, Write};

use velter_core::{
    Debugger, Module, Process, ScanCmp, Scanner, Value, ValueType, control_flow_graph, disassemble,
    list_processes,
};
use velter_runtime::load_auto;

fn main() {
    if let Err(e) = run() {
        eprintln!("velter: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "help" || args[0] == "-h" {
        usage();
        return Ok(());
    }
    if args[0] == "ps" {
        let filter = args.get(1).map_or("", String::as_str).to_ascii_lowercase();
        let mut ps = list_processes();
        ps.sort_by_key(|p| p.name.to_ascii_lowercase());
        for p in ps
            .into_iter()
            .filter(|p| p.name.to_ascii_lowercase().contains(&filter))
        {
            println!("{:<7} {}", p.pid, p.name);
        }
        return Ok(());
    }
    if args[0] == "meta" {
        let path = args
            .get(1)
            .ok_or("meta needs an assembly or global-metadata.dat path")?;
        let query = args.get(2).map_or("", String::as_str);
        let model = load_auto(path).map_err(|e| e.to_string())?;
        println!(
            "{:?} v{} · {} assemblies · {} types · {} fields · {} methods",
            model.kind,
            model.format_version,
            model.assemblies.len(),
            model.type_count(),
            model.field_count(),
            model.method_count()
        );
        if !query.is_empty() {
            for (assembly, ty) in model.search(query) {
                println!("{} :: {}.{}", assembly.name, ty.namespace, ty.name);
                for method in &ty.methods {
                    println!(
                        "  0x{:08x} {}({} params)",
                        method.token, method.name, method.parameter_count
                    );
                }
            }
        }
        return Ok(());
    }
    let info = if args[0] == "open" {
        let needle = args
            .get(1)
            .ok_or("open needs a process name")?
            .to_ascii_lowercase();
        list_processes()
            .into_iter()
            .find(|p| p.name.to_ascii_lowercase().contains(&needle))
            .ok_or_else(|| format!("no process matching {needle:?}"))?
    } else {
        let pid: u32 = args[0]
            .parse()
            .map_err(|_| "expected a PID, `ps`, or `open NAME`")?;
        list_processes()
            .into_iter()
            .find(|p| p.pid == pid)
            .ok_or("PID not found")?
    };
    let process = Process::open(info.pid, &info.name).map_err(|e| e.to_string())?;
    println!(
        "Velter CLI · {} ({}) · {:?}",
        info.name,
        info.pid,
        process.backend()
    );
    repl(process)
}

fn usage() {
    println!(
        "Velter CLI\n  velter-cli ps [filter]\n  velter-cli meta <Assembly-CSharp.dll|global-metadata.dat> [search]\n  velter-cli <pid>\n  velter-cli open <name>\n\nInside a target, type `help`."
    );
}

struct Session {
    process: Process,
    modules: Vec<Module>,
    seek: u64,
    scanner: Option<Scanner>,
    debugger: Option<Debugger>,
}

fn repl(process: Process) -> Result<(), String> {
    let modules = process.modules();
    let seek = modules.first().map_or(0, |m| m.base);
    let mut s = Session {
        process,
        modules,
        seek,
        scanner: None,
        debugger: None,
    };
    let stdin = io::stdin();
    loop {
        print!("[0x{:x}]> ", s.seek);
        io::stdout().flush().map_err(|e| e.to_string())?;
        let mut line = String::new();
        if stdin.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match command(&mut s, line) {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => eprintln!("error: {e}"),
        }
    }
    if let Some(d) = s.debugger.take() {
        let _ = d.detach();
    }
    Ok(())
}

fn command(s: &mut Session, line: &str) -> Result<bool, String> {
    let mut words = line.split_whitespace();
    let cmd = words.next().unwrap();
    match cmd {
        "q" | "quit" | "exit" => return Ok(false),
        "?" | "help" => println!(
            "info | modules | s ADDRESS | px [LEN] [@ ADDR] | pd [COUNT] [@ ADDR] | agf [BYTES] [@ ADDR]\nwx HEXBYTES [@ ADDR] | scan TYPE VALUE | next CMP [VALUE] | hits [N]\npause | resume | dp | dp- | watch ADDRESS [1|2|4|8] | dw- | log | q"
        ),
        "info" => println!(
            "{} pid={} backend={:?}",
            s.process.name,
            s.process.pid,
            s.process.backend()
        ),
        "modules" | "dm" => {
            for m in &s.modules {
                println!("0x{:016x}  {:9x}  {}", m.base, m.size, m.name);
            }
        }
        "s" => {
            s.seek = resolve(s, words.next().ok_or("s needs an address")?)?;
        }
        "px" | "read" => {
            let rest: Vec<_> = words.collect();
            let (count, addr) = count_addr(s, &rest, 128)?;
            let mut bytes = vec![0; count.min(65536)];
            let n = s.process.read(addr, &mut bytes);
            bytes.truncate(n);
            for (row, chunk) in bytes.chunks(16).enumerate() {
                print!("0x{:016x}  ", addr + (row * 16) as u64);
                for b in chunk {
                    print!("{b:02x} ");
                }
                println!();
            }
            s.seek = addr + n as u64;
        }
        "pd" | "disasm" => {
            let rest: Vec<_> = words.collect();
            let (count, addr) = count_addr(s, &rest, 16)?;
            let lines = disassemble(&s.process, addr, count.saturating_mul(15));
            for i in lines.into_iter().take(count) {
                let raw = i
                    .bytes
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join("");
                println!("0x{:016x}  {:<24} {}", i.address, raw, i.text);
                s.seek = i.address + i.bytes.len() as u64;
            }
        }
        "agf" | "cfg" => {
            let rest: Vec<_> = words.collect();
            let (bytes, addr) = count_addr(s, &rest, 512)?;
            for b in control_flow_graph(&s.process, addr, bytes) {
                let edges = b
                    .successors
                    .iter()
                    .map(|x| format!("0x{x:x}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                println!(
                    "0x{:x}..0x{:x}  {:>3} insn  -> [{}]",
                    b.start, b.end, b.instruction_count, edges
                );
            }
        }
        "wx" | "write" => {
            let rest: Vec<_> = words.collect();
            let at = rest.iter().position(|x| *x == "@");
            let end = at.unwrap_or(rest.len());
            let data = hex_bytes(&rest[..end].join(""))?;
            let addr = at
                .and_then(|i| rest.get(i + 1))
                .map(|x| resolve(s, x))
                .transpose()?
                .unwrap_or(s.seek);
            s.process.write(addr, &data).map_err(|e| e.to_string())?;
            println!("wrote {} bytes @ 0x{addr:x}", data.len());
            s.seek = addr + data.len() as u64;
        }
        "pause" => s.process.suspend().map_err(|e| e.to_string())?,
        "resume" => s.process.resume().map_err(|e| e.to_string())?,
        "dp" => {
            if s.debugger.is_none() {
                s.debugger = Some(Debugger::attach(s.process.pid).map_err(|e| e.to_string())?);
                println!("debugger attached");
            }
        }
        "dp-" => {
            if let Some(d) = s.debugger.take() {
                d.detach().map_err(|e| e.to_string())?;
                println!("debugger detached");
            }
        }
        "watch" | "dww" => {
            let addr = resolve(s, words.next().ok_or("watch needs an address")?)?;
            let size = words
                .next()
                .unwrap_or("4")
                .parse()
                .map_err(|_| "bad size")?;
            if s.debugger.is_none() {
                s.debugger = Some(Debugger::attach(s.process.pid).map_err(|e| e.to_string())?);
            }
            s.debugger
                .as_ref()
                .unwrap()
                .watch_writes(addr, size)
                .map_err(|e| e.to_string())?;
            println!("watching writes to 0x{addr:x}; use `log`");
        }
        "dw-" => {
            if let Some(d) = &s.debugger {
                d.clear_watch().map_err(|e| e.to_string())?;
            }
        }
        "log" => {
            if let Some(d) = &s.debugger {
                for x in d.log.lock().unwrap().iter() {
                    println!("{x}");
                }
            }
        }
        "scan" => {
            let ty = value_type(words.next().ok_or("scan needs TYPE VALUE")?)?;
            let value =
                Value::parse(ty, words.next().ok_or("scan needs a value")?).ok_or("bad value")?;
            println!("scanning…");
            let found = Scanner::first(&s.process, ty, value);
            println!("{} hits", found.hits.len());
            s.scanner = Some(found);
        }
        "next" => {
            let Some(mut scan) = s.scanner.take() else {
                return Err("run scan first".into());
            };
            let kind = words
                .next()
                .ok_or("next needs exact/changed/unchanged/increased/decreased")?;
            let cmp = match kind {
                "exact" => ScanCmp::Exact(
                    Value::parse(scan.ty, words.next().ok_or("exact needs value")?)
                        .ok_or("bad value")?,
                ),
                "changed" => ScanCmp::Changed,
                "unchanged" => ScanCmp::Unchanged,
                "increased" => ScanCmp::Increased,
                "decreased" => ScanCmp::Decreased,
                _ => return Err("unknown comparison".into()),
            };
            scan.next(&s.process, cmp);
            println!("{} hits", scan.hits.len());
            s.scanner = Some(scan);
        }
        "hits" => {
            let n = words
                .next()
                .unwrap_or("50")
                .parse()
                .map_err(|_| "bad count")?;
            if let Some(scan) = &s.scanner {
                for h in scan.hits.iter().take(n) {
                    println!("0x{:016x}  {}", h.addr, Value::from_bits(scan.ty, h.bits));
                }
            }
        }
        _ => return Err(format!("unknown command {cmd:?}; try help")),
    }
    Ok(true)
}

fn resolve(s: &Session, text: &str) -> Result<u64, String> {
    if let Some((name, off)) = text.rsplit_once('+') {
        let base = s
            .modules
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case(name))
            .ok_or("module not found")?
            .base;
        return Ok(base + number(off)?);
    }
    number(text)
}

fn number(s: &str) -> Result<u64, String> {
    let s = s.replace('_', "");
    if let Some(x) = s.strip_prefix("0x") {
        u64::from_str_radix(x, 16).map_err(|_| "bad number".into())
    } else {
        s.parse()
            .or_else(|_| u64::from_str_radix(&s, 16))
            .map_err(|_| "bad number".into())
    }
}

fn count_addr(s: &Session, words: &[&str], default: usize) -> Result<(usize, u64), String> {
    let at = words.iter().position(|x| *x == "@");
    let count = words
        .first()
        .filter(|x| **x != "@")
        .map(|x| number(x).map(|n| n as usize))
        .transpose()?
        .unwrap_or(default);
    let addr = at
        .and_then(|i| words.get(i + 1))
        .map(|x| resolve(s, x))
        .transpose()?
        .unwrap_or(s.seek);
    Ok((count, addr))
}

fn value_type(s: &str) -> Result<ValueType, String> {
    match s.to_ascii_lowercase().as_str() {
        "i32" | "int" | "int32" => Ok(ValueType::I32),
        "i64" | "int64" => Ok(ValueType::I64),
        "f32" | "float" => Ok(ValueType::F32),
        "f64" | "double" => Ok(ValueType::F64),
        _ => Err("type: i32/i64/f32/f64".into()),
    }
}

fn hex_bytes(s: &str) -> Result<Vec<u8>, String> {
    let s = s.replace([',', '_'], "");
    if s.is_empty() || s.len() % 2 != 0 {
        return Err("hex bytes need pairs".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| "bad hex bytes".into()))
        .collect()
}
