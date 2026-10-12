use std::sync::Arc;
use std::thread::JoinHandle;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, RichText, ScrollArea, TextEdit, ViewportCommand,
};
use velter_core::{
    Backend, Debugger, Module, Process, ProcessInfo, ScanCmp, Scanner, Value, ValueType,
    control_flow_graph, disassemble, list_processes,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_F9, VK_INSERT};

use crate::glass::{Hotkeys, Hwnd};
use crate::theme::{self, ACCENT, ACCENT2, DIM, GOOD, WARN};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Overlay takes input, game blurred behind it.
    Interact,
    /// Click-through HUD; you keep playing.
    Passthrough,
}

/// A row in the cheat table.
struct Entry {
    addr: u64,
    ty: ValueType,
    label: String,
    edit: String,
    frozen: Option<u64>,
    pinned: bool,
}

struct Patch {
    addr: u64,
    before: Vec<u8>,
    after: Vec<u8>,
    active: bool,
}

struct Attached {
    proc: Arc<Process>,
    backend: Backend,
    modules: Vec<Module>,
    dbg: Option<Debugger>,
    paused: bool,
}

pub struct App {
    hwnd: Option<Hwnd>,
    hit_rects: Vec<egui::Rect>,
    mode: Mode,
    keys: Hotkeys,

    procs: Vec<ProcessInfo>,
    proc_filter: String,
    target: Option<Attached>,

    scanner: Option<Scanner>,
    scan_job: Option<JoinHandle<Scanner>>,
    scan_ty: ValueType,
    scan_text: String,
    next_cmp: usize,

    table: Vec<Entry>,
    selected_data: Option<(u64, ValueType)>,
    inspect_addr: String,
    patch_text: String,
    patches: Vec<Patch>,
    last_writer: Option<u64>,
    status: String,
}

const NEXT_CMPS: [&str; 5] = ["exact", "changed", "unchanged", "increased", "decreased"];

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        theme::apply(&cc.egui_ctx);
        let hwnd = Hwnd::from_handle(cc);
        let mut procs = list_processes();
        procs.sort_by_key(|p| p.name.to_ascii_lowercase());
        App {
            hwnd,
            hit_rects: Vec::new(),
            mode: Mode::Interact,
            keys: Hotkeys::default(),
            procs,
            proc_filter: String::new(),
            target: None,
            scanner: None,
            scan_job: None,
            scan_ty: ValueType::I32,
            scan_text: String::new(),
            next_cmp: 0,
            table: Vec::new(),
            selected_data: None,
            inspect_addr: String::new(),
            patch_text: String::new(),
            patches: Vec::new(),
            last_writer: None,
            status: "Insert: show/hide overlay   F9: pause/resume game".into(),
        }
    }

    fn set_mode(&mut self, ctx: &egui::Context, mode: Mode) {
        self.mode = mode;
        let interact = mode == Mode::Interact;
        if let Some(hwnd) = self.hwnd {
            if !interact {
                hwnd.clear_panel_region();
            }
        }
        ctx.send_viewport_cmd(ViewportCommand::MousePassthrough(!interact));
        if interact {
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
    }

    fn attach(&mut self, info: &ProcessInfo) {
        self.detach();
        match Process::open(info.pid, &info.name) {
            Ok(p) => {
                let backend = p.backend();
                let modules = p.modules();
                self.status = format!("opened {} ({}) · {:?}", info.name, info.pid, backend);
                self.target = Some(Attached {
                    proc: Arc::new(p),
                    backend,
                    modules,
                    dbg: None,
                    paused: false,
                });
            }
            Err(e) => self.status = format!("open failed: {e}"),
        }
    }

    fn detach(&mut self) {
        if let Some(mut t) = self.target.take() {
            if t.paused {
                let _ = t.proc.resume();
            }
            if let Some(d) = t.dbg.take() {
                let _ = d.detach();
            }
        }
        self.scanner = None;
        self.table.clear();
        self.selected_data = None;
        self.inspect_addr.clear();
        self.patches.clear();
        self.last_writer = None;
    }

    fn toggle_pause(&mut self) {
        let Some(t) = &mut self.target else { return };
        let r = if t.paused {
            t.proc.resume()
        } else {
            t.proc.suspend()
        };
        match r {
            Ok(()) => t.paused = !t.paused,
            Err(e) => self.status = format!("pause failed: {e}"),
        }
    }

    fn toggle_debugger(&mut self) {
        let Some(t) = &mut self.target else { return };
        self.status = match t.dbg.take() {
            Some(d) => match d.detach() {
                Ok(()) => "dp- · detached, game still running".into(),
                Err(e) => format!("dp- failed: {e}"),
            },
            None => match Debugger::attach(t.proc.pid) {
                Ok(d) => {
                    t.dbg = Some(d);
                    format!("dp {} · debugger attached", t.proc.pid)
                }
                Err(e) => format!("dp failed: {e}"),
            },
        };
    }

    fn start_scan(&mut self, first: bool) {
        let Some(t) = &self.target else { return };
        if self.scan_job.is_some() {
            return;
        }
        let proc = t.proc.clone();
        let parsed = Value::parse(self.scan_ty, &self.scan_text);
        if first {
            let Some(v) = parsed else {
                self.status = format!(
                    "can't parse {:?} as {}",
                    self.scan_text,
                    self.scan_ty.name()
                );
                return;
            };
            let ty = self.scan_ty;
            self.scan_job = Some(std::thread::spawn(move || Scanner::first(&proc, ty, v)));
        } else {
            let Some(mut s) = self.scanner.take() else {
                return;
            };
            let cmp = match self.next_cmp {
                0 => match Value::parse(s.ty, &self.scan_text) {
                    Some(v) => ScanCmp::Exact(v),
                    None => {
                        self.status = "next scan 'exact' needs a value".into();
                        self.scanner = Some(s);
                        return;
                    }
                },
                1 => ScanCmp::Changed,
                2 => ScanCmp::Unchanged,
                3 => ScanCmp::Increased,
                _ => ScanCmp::Decreased,
            };
            self.scan_job = Some(std::thread::spawn(move || {
                s.next(&proc, cmp);
                s
            }));
        }
    }

    fn poll_scan(&mut self) {
        if self.scan_job.as_ref().is_some_and(|j| j.is_finished()) {
            match self.scan_job.take().unwrap().join() {
                Ok(s) => {
                    if self.target.is_some() {
                        self.status = format!(
                            "scan #{}: {} hits{}",
                            s.scans,
                            s.hits.len(),
                            if s.truncated { " (truncated)" } else { "" }
                        );
                        self.scanner = Some(s);
                    }
                }
                Err(_) => self.status = "scan thread panicked".into(),
            }
        }
    }

    fn add_entry(&mut self, addr: u64, ty: ValueType) {
        if self.table.iter().any(|e| e.addr == addr) {
            return;
        }
        let label = self
            .target
            .as_ref()
            .map(|t| t.proc.symbolize(&t.modules, addr))
            .unwrap_or_default();
        self.table.push(Entry {
            addr,
            ty,
            label,
            edit: String::new(),
            frozen: None,
            pinned: false,
        });
    }

    fn apply_freezes(&self) {
        let Some(t) = &self.target else { return };
        for e in &self.table {
            if let Some(bits) = e.frozen {
                let _ = t.proc.write(e.addr, &bits.to_le_bytes()[..e.ty.size()]);
            }
        }
    }

    fn resolve_address(&self, text: &str) -> Option<u64> {
        let t = self.target.as_ref()?;
        let text = text.trim();
        if let Some((module, offset)) = text.rsplit_once('+') {
            let base = t
                .modules
                .iter()
                .find(|m| m.name.eq_ignore_ascii_case(module.trim()))?
                .base;
            return parse_u64(offset).map(|o| base + o);
        }
        parse_u64(text)
    }

    fn apply_byte_patch(&mut self) {
        let (Some(t), Some(addr)) = (&self.target, self.resolve_address(&self.inspect_addr)) else {
            self.status = "inspector needs a valid address".into();
            return;
        };
        let Some(after) = parse_hex_bytes(&self.patch_text) else {
            self.status = "bytes: use hex like 90 90 90".into();
            return;
        };
        let mut before = vec![0; after.len()];
        if t.proc.read(addr, &mut before) != before.len() {
            self.status = "could not read original bytes".into();
            return;
        }
        match t.proc.write(addr, &after) {
            Ok(()) => {
                self.patches.push(Patch {
                    addr,
                    before,
                    after: after.clone(),
                    active: true,
                });
                self.status = format!("patched {} bytes @ 0x{addr:x}", after.len());
            }
            Err(e) => self.status = format!("patch failed: {e}"),
        }
    }
}

fn parse_u64(text: &str) -> Option<u64> {
    let s = text.trim().replace('_', "");
    u64::from_str_radix(
        s.strip_prefix("0x")
            .or_else(|| s.strip_prefix("0X"))
            .unwrap_or(&s),
        16,
    )
    .ok()
}

fn parse_hex_bytes(text: &str) -> Option<Vec<u8>> {
    let compact: String = text
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != ',')
        .collect();
    if compact.is_empty() || compact.len() % 2 != 0 {
        return None;
    }
    (0..compact.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&compact[i..i + 2], 16).ok())
        .collect()
}

fn parse_write_hit(line: &str) -> Option<u64> {
    let rest = line.strip_prefix("WRITE HIT @ 0x")?;
    let hex = rest.split_whitespace().next()?;
    u64::from_str_radix(hex, 16).ok()
}

impl eframe::App for App {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.keys.pressed(VK_INSERT) {
            let next = if self.mode == Mode::Interact {
                Mode::Passthrough
            } else {
                Mode::Interact
            };
            self.set_mode(ctx, next);
        }
        if self.keys.pressed(VK_F9) {
            self.toggle_pause();
        }
        if self.target.as_ref().is_some_and(|t| !t.proc.is_alive()) {
            self.detach();
            self.status = "target exited · detached automatically".into();
        }
        if let Some(t) = &mut self.target
            && t.dbg.as_ref().is_some_and(|d| !d.is_alive())
        {
            t.dbg = None;
            self.status = "debugger session ended".into();
        }
        if let Some(hit) = self
            .target
            .as_ref()
            .and_then(|t| t.dbg.as_ref())
            .and_then(|d| {
                d.log
                    .lock()
                    .unwrap()
                    .iter()
                    .rev()
                    .find_map(|line| parse_write_hit(line))
            })
        {
            if self.last_writer != Some(hit) {
                self.last_writer = Some(hit);
                self.inspect_addr = format!("0x{hit:x}");
                self.status = format!("writer captured @ 0x{hit:x} · opened code view");
            }
        }
        self.poll_scan();
        self.apply_freezes();
        // Keep polling hotkeys and live values even when nothing moves.
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.hit_rects.clear();
        self.top_bar(&ctx);
        match self.mode {
            Mode::Interact => {
                self.process_window(&ctx);
                if self.target.is_some() {
                    self.scanner_window(&ctx);
                    if !self.table.is_empty() {
                        self.table_window(&ctx);
                    }
                    if self.selected_data.is_some() {
                        self.data_window(&ctx);
                    }
                    if !self.inspect_addr.is_empty() {
                        self.inspector_window(&ctx);
                    }
                }
            }
            Mode::Passthrough => self.hud(&ctx),
        }
        if self.mode == Mode::Interact {
            if let Some(hwnd) = self.hwnd {
                hwnd.set_panel_region(&self.hit_rects, ctx.pixels_per_point());
            }
        }
    }
}

// ---- panels -------------------------------------------------------------

impl App {
    fn top_bar(&mut self, ctx: &egui::Context) {
        let interact = self.mode == Mode::Interact;
        let shown = egui::Area::new("topbar".into())
            .anchor(Align2::CENTER_TOP, [0.0, 12.0])
            .interactable(interact)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(theme::glass_fill(if interact { 200 } else { 140 }))
                    .stroke(theme::edge())
                    .corner_radius(CornerRadius::same(255))
                    .inner_margin(egui::Margin::symmetric(16, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("◆ VELTER").strong().color(ACCENT));
                            ui.separator();
                            match &self.target {
                                Some(t) => {
                                    ui.label(RichText::new(&t.proc.name).strong());
                                    let (b, c) = match t.backend {
                                        Backend::Mono => ("mono", GOOD),
                                        Backend::Il2Cpp => ("il2cpp", ACCENT2),
                                        Backend::Unknown => ("native", DIM),
                                    };
                                    ui.label(RichText::new(b).small().color(c));
                                    let (s, c) = if t.paused {
                                        ("■ PAUSED", WARN)
                                    } else {
                                        ("● running", GOOD)
                                    };
                                    ui.label(RichText::new(s).color(c));
                                    if t.dbg.is_some() {
                                        ui.label(RichText::new("dbg").small().color(ACCENT));
                                    }
                                }
                                None => {
                                    ui.label(RichText::new("no target").color(DIM));
                                }
                            }
                            if !interact {
                                ui.label(RichText::new("Insert ↩").small().color(DIM));
                                return;
                            }
                            ui.separator();
                            if self.target.is_some() {
                                let paused = self.target.as_ref().is_some_and(|t| t.paused);
                                if ui
                                    .button(if paused { "▶ resume" } else { "⏸ pause" })
                                    .clicked()
                                {
                                    self.toggle_pause();
                                }
                                let dbg = self.target.as_ref().is_some_and(|t| t.dbg.is_some());
                                if ui
                                    .button(if dbg { "dp-" } else { "dp" })
                                    .on_hover_text("attach / detach debugger")
                                    .clicked()
                                {
                                    self.toggle_debugger();
                                }
                            }
                            if ui.button("hide").on_hover_text("Insert").clicked() {
                                self.set_mode(ui.ctx(), Mode::Passthrough);
                            }
                            if ui.button(RichText::new("✕").color(WARN)).clicked() {
                                self.detach();
                                ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                            }
                        });
                        if interact {
                            ui.label(RichText::new(&self.status).small().color(DIM));
                        }
                    });
            });
        self.hit_rects.push(shown.response.rect);
    }

    fn process_window(&mut self, ctx: &egui::Context) {
        let mut pick = None;
        let shown = egui::Window::new("process")
            .default_pos([24.0, 80.0])
            .default_size([280.0, 420.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        TextEdit::singleline(&mut self.proc_filter)
                            .hint_text("filter…")
                            .desired_width(170.0),
                    );
                    if ui.button("⟳").clicked() {
                        self.procs = list_processes();
                        self.procs.sort_by_key(|p| p.name.to_ascii_lowercase());
                    }
                });
                ui.separator();
                let f = self.proc_filter.to_ascii_lowercase();
                let cur = self.target.as_ref().map(|t| t.proc.pid);
                ScrollArea::vertical().show(ui, |ui| {
                    for p in self
                        .procs
                        .iter()
                        .filter(|p| p.name.to_ascii_lowercase().contains(&f))
                    {
                        let sel = cur == Some(p.pid);
                        let text = RichText::new(format!("{}  ·  {}", p.name, p.pid));
                        if ui.selectable_label(sel, text).clicked() && !sel {
                            pick = Some(p.clone());
                        }
                    }
                });
            });
        if let Some(shown) = shown {
            self.hit_rects.push(shown.response.rect);
        }
        if let Some(p) = pick {
            self.attach(&p);
        }
    }

    fn scanner_window(&mut self, ctx: &egui::Context) {
        let busy = self.scan_job.is_some();
        let mut add = None;
        let mut inspect = None;
        let shown = egui::Window::new("scanner")
            .default_pos([330.0, 80.0])
            .default_size([420.0, 520.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let locked = self.scanner.is_some();
                    ui.add_enabled_ui(!locked, |ui| {
                        egui::ComboBox::from_id_salt("ty")
                            .selected_text(self.scan_ty.name())
                            .width(80.0)
                            .show_ui(ui, |ui| {
                                for t in ValueType::ALL {
                                    ui.selectable_value(&mut self.scan_ty, t, t.name());
                                }
                            });
                    });
                    let r = ui.add(
                        TextEdit::singleline(&mut self.scan_text)
                            .hint_text("value, e.g. 300k")
                            .font(egui::TextStyle::Monospace)
                            .desired_width(150.0),
                    );
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !busy {
                        self.start_scan(self.scanner.is_none());
                    }
                });
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(!busy, |ui| {
                        if self.scanner.is_none() {
                            if ui
                                .button(RichText::new("first scan").color(ACCENT))
                                .clicked()
                            {
                                self.start_scan(true);
                            }
                        } else {
                            egui::ComboBox::from_id_salt("cmp")
                                .selected_text(NEXT_CMPS[self.next_cmp])
                                .width(90.0)
                                .show_ui(ui, |ui| {
                                    for (i, c) in NEXT_CMPS.iter().enumerate() {
                                        ui.selectable_value(&mut self.next_cmp, i, *c);
                                    }
                                });
                            if ui
                                .button(RichText::new("next scan").color(ACCENT))
                                .clicked()
                            {
                                self.start_scan(false);
                            }
                            if ui.button("new").clicked() {
                                self.scanner = None;
                            }
                        }
                    });
                    if busy {
                        ui.spinner();
                    }
                });
                ui.separator();

                let (Some(s), Some(t)) = (&self.scanner, &self.target) else {
                    ui.label(
                        RichText::new(
                            "search a value you can see in game, change it, then next scan",
                        )
                        .small()
                        .color(DIM),
                    );
                    return;
                };
                ui.label(RichText::new(format!("{} hits", s.hits.len())).color(ACCENT2));
                let row_h = 20.0;
                ScrollArea::vertical().auto_shrink(false).show_rows(
                    ui,
                    row_h,
                    s.hits.len(),
                    |ui, range| {
                        egui::Grid::new("hits")
                            .num_columns(4)
                            .striped(true)
                            .show(ui, |ui| {
                                for h in &s.hits[range] {
                                    let cur = t
                                        .proc
                                        .read_bits(h.addr, s.ty.size())
                                        .map(|b| Value::from_bits(s.ty, b));
                                    let prev = Value::from_bits(s.ty, h.bits);
                                    if ui
                                        .link(
                                            RichText::new(t.proc.symbolize(&t.modules, h.addr))
                                                .monospace(),
                                        )
                                        .clicked()
                                    {
                                        inspect = Some((h.addr, s.ty));
                                    }
                                    match cur {
                                        Some(v) if v.to_bits() != h.bits => {
                                            ui.label(
                                                RichText::new(v.to_string())
                                                    .monospace()
                                                    .color(WARN),
                                            );
                                        }
                                        Some(v) => {
                                            ui.monospace(v.to_string());
                                        }
                                        None => {
                                            ui.label(RichText::new("??").color(DIM));
                                        }
                                    }
                                    ui.label(
                                        RichText::new(prev.to_string()).monospace().color(DIM),
                                    );
                                    if ui.small_button("+").on_hover_text("add to table").clicked()
                                    {
                                        add = Some((h.addr, s.ty));
                                    }
                                    ui.end_row();
                                }
                            });
                    },
                );
            });
        if let Some(shown) = shown {
            self.hit_rects.push(shown.response.rect);
        }
        if let Some((a, ty)) = add {
            self.add_entry(a, ty);
        }
        if let Some((a, ty)) = inspect {
            self.selected_data = Some((a, ty));
        }
    }

    fn table_window(&mut self, ctx: &egui::Context) {
        let Some(t) = &self.target else { return };
        let proc = t.proc.clone();
        let mut remove = None;
        let mut status = None;
        let shown = egui::Window::new("table")
            .default_pos([780.0, 80.0])
            .default_size([460.0, 300.0])
            .show(ctx, |ui| {
                if self.table.is_empty() {
                    ui.label(
                        RichText::new("add addresses from the scanner with +")
                            .small()
                            .color(DIM),
                    );
                    return;
                }
                egui::Grid::new("table")
                    .num_columns(6)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label(RichText::new("address").small().color(DIM));
                        ui.label(RichText::new("type").small().color(DIM));
                        ui.label(RichText::new("value").small().color(DIM));
                        ui.label(RichText::new("freeze").small().color(DIM));
                        ui.label(RichText::new("pin").small().color(DIM));
                        ui.label("");
                        ui.end_row();
                        for (i, e) in self.table.iter_mut().enumerate() {
                            ui.monospace(&e.label);
                            ui.label(RichText::new(e.ty.name()).small().color(DIM));
                            let cur = proc
                                .read_bits(e.addr, e.ty.size())
                                .map(|b| Value::from_bits(e.ty, b));
                            let hint = cur.map_or("??".into(), |v| v.to_string());
                            let r = ui.add(
                                TextEdit::singleline(&mut e.edit)
                                    .hint_text(RichText::new(hint).color(Color32::WHITE))
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(110.0),
                            );
                            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                match Value::parse(e.ty, &e.edit) {
                                    Some(v) => {
                                        if let Err(err) = proc.write(e.addr, &v.to_bytes()) {
                                            status = Some(format!("write failed: {err}"));
                                        } else if e.frozen.is_some() {
                                            e.frozen = Some(v.to_bits());
                                        }
                                    }
                                    None => status = Some(format!("bad {} value", e.ty.name())),
                                }
                                e.edit.clear();
                            }
                            let mut frozen = e.frozen.is_some();
                            if ui.checkbox(&mut frozen, "").changed() {
                                e.frozen = if frozen {
                                    cur.map(|v| v.to_bits())
                                } else {
                                    None
                                };
                            }
                            ui.checkbox(&mut e.pinned, "")
                                .on_hover_text("show on HUD while hidden");
                            if ui.small_button("🗑").clicked() {
                                remove = Some(i);
                            }
                            ui.end_row();
                        }
                    });
            });
        if let Some(shown) = shown {
            self.hit_rects.push(shown.response.rect);
        }
        if let Some(i) = remove {
            self.table.remove(i);
        }
        if let Some(s) = status {
            self.status = s;
        }
    }

    fn data_window(&mut self, ctx: &egui::Context) {
        let Some((addr, ty)) = self.selected_data else {
            return;
        };
        let current = self
            .target
            .as_ref()
            .and_then(|t| t.proc.read_bits(addr, ty.size()))
            .map(|b| Value::from_bits(ty, b));
        let mut start_watch = false;
        let mut stop_watch = false;
        let mut close = false;
        let shown = egui::Window::new("data address")
            .default_pos([780.0, 390.0])
            .default_size([390.0, 150.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("0x{addr:016x}")).monospace().color(ACCENT));
                    ui.label(RichText::new(ty.name()).small().color(DIM));
                    ui.label(RichText::new(current.map_or("??".into(), |v| v.to_string())).monospace().size(18.0));
                });
                ui.label(RichText::new("This is data, not executable code. Capture its writer before opening disassembly.").small().color(DIM));
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("find what writes").color(WARN)).clicked() { start_watch = true; }
                    if ui.button("stop watch").clicked() { stop_watch = true; }
                    if ui.button("close").clicked() { close = true; }
                });
            });
        if let Some(shown) = shown {
            self.hit_rects.push(shown.response.rect);
        }
        if start_watch {
            let t = self.target.as_mut().unwrap();
            if t.dbg.is_none() {
                match Debugger::attach(t.proc.pid) {
                    Ok(d) => t.dbg = Some(d),
                    Err(e) => self.status = format!("dp failed: {e}"),
                }
            }
            if let Some(d) = &t.dbg {
                match d.watch_writes(addr, ty.size()) {
                    Ok(()) => {
                        d.log.lock().unwrap().clear();
                        self.last_writer = None;
                        self.inspect_addr.clear();
                        self.status = format!("watching writes to 0x{addr:x} · change it in game");
                    }
                    Err(e) => self.status = e.to_string(),
                }
            }
        }
        if stop_watch {
            if let Some(d) = self.target.as_ref().and_then(|t| t.dbg.as_ref()) {
                let _ = d.clear_watch();
            }
        }
        if close {
            self.selected_data = None;
        }
    }

    fn inspector_window(&mut self, ctx: &egui::Context) {
        let Some(proc) = self.target.as_ref().map(|t| t.proc.clone()) else {
            return;
        };
        let addr = self.resolve_address(&self.inspect_addr);
        let lines = addr.map(|a| disassemble(&proc, a, 320)).unwrap_or_default();
        let blocks = addr
            .map(|a| control_flow_graph(&proc, a, 512))
            .unwrap_or_default();
        let debug_log = self
            .target
            .as_ref()
            .and_then(|t| t.dbg.as_ref())
            .map(|d| d.log.lock().unwrap().clone())
            .unwrap_or_default();
        let mut apply = false;
        let mut close = false;
        let mut toggle_patch = None;
        let shown = egui::Window::new("code · live inspector")
            .default_pos([330.0, 630.0])
            .default_size([820.0, 360.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("address").small().color(DIM));
                    ui.add(
                        TextEdit::singleline(&mut self.inspect_addr)
                            .hint_text("0x7ff… or module.dll+1234")
                            .font(egui::TextStyle::Monospace)
                            .desired_width(240.0),
                    );
                    ui.label(RichText::new("patch bytes").small().color(DIM));
                    let edit = ui.add(
                        TextEdit::singleline(&mut self.patch_text)
                            .hint_text("90 90 90")
                            .font(egui::TextStyle::Monospace)
                            .desired_width(180.0),
                    );
                    if ui.button(RichText::new("apply").color(ACCENT)).clicked()
                        || (edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    {
                        apply = true;
                    }
                    if ui.small_button("close code").clicked() {
                        close = true;
                    }
                });
                ui.label(
                    RichText::new(
                        "Captured writer instruction. Patches preserve their original bytes.",
                    )
                    .small()
                    .color(DIM),
                );
                ui.separator();
                ui.columns(2, |cols| {
                    cols[0].label(RichText::new("LIVE X86-64").strong().color(ACCENT2));
                    ScrollArea::vertical()
                        .max_height(245.0)
                        .show(&mut cols[0], |ui| {
                            egui::Grid::new("disasm")
                                .num_columns(3)
                                .striped(true)
                                .show(ui, |ui| {
                                    for line in &lines {
                                        ui.label(
                                            RichText::new(format!("{:016X}", line.address))
                                                .monospace()
                                                .color(DIM),
                                        );
                                        let raw = line
                                            .bytes
                                            .iter()
                                            .map(|b| format!("{b:02X}"))
                                            .collect::<Vec<_>>()
                                            .join(" ");
                                        ui.label(
                                            RichText::new(format!("{raw:<24}"))
                                                .monospace()
                                                .small()
                                                .color(ACCENT),
                                        );
                                        ui.label(RichText::new(&line.text).monospace());
                                        ui.end_row();
                                    }
                                });
                        });
                    cols[1].label(RichText::new("PATCH STACK").strong().color(ACCENT2));
                    ScrollArea::vertical()
                        .max_height(245.0)
                        .show(&mut cols[1], |ui| {
                            ui.label(
                                RichText::new(format!("CFG · {} blocks", blocks.len()))
                                    .strong()
                                    .color(ACCENT),
                            );
                            for block in blocks.iter().take(24) {
                                egui::Frame::new()
                                    .fill(theme::glass_fill(120))
                                    .stroke(theme::edge())
                                    .corner_radius(CornerRadius::same(6))
                                    .inner_margin(egui::Margin::same(6))
                                    .show(ui, |ui| {
                                        ui.monospace(format!(
                                            "0x{:x}  ·  {} insn",
                                            block.start, block.instruction_count
                                        ));
                                        if !block.successors.is_empty() {
                                            ui.label(
                                                RichText::new(format!(
                                                    "↳ {}",
                                                    block
                                                        .successors
                                                        .iter()
                                                        .map(|x| format!("0x{x:x}"))
                                                        .collect::<Vec<_>>()
                                                        .join("  ·  ")
                                                ))
                                                .small()
                                                .color(ACCENT2),
                                            );
                                        }
                                    });
                            }
                            ui.separator();
                            ui.label(RichText::new("PATCHES").strong().color(ACCENT2));
                            if self.patches.is_empty() {
                                ui.label(RichText::new("no patches yet").small().color(DIM));
                            }
                            for (i, p) in self.patches.iter().enumerate().rev() {
                                ui.horizontal(|ui| {
                                    ui.monospace(format!("0x{:x}", p.addr));
                                    ui.label(
                                        RichText::new(format!("{} B", p.after.len()))
                                            .small()
                                            .color(DIM),
                                    );
                                    if ui
                                        .button(if p.active { "revert" } else { "reapply" })
                                        .clicked()
                                    {
                                        toggle_patch = Some(i);
                                    }
                                });
                            }
                            if !debug_log.is_empty() {
                                ui.separator();
                                ui.label(RichText::new("DEBUG EVENTS").strong().color(WARN));
                                for entry in debug_log.iter().rev().take(12).rev() {
                                    ui.label(RichText::new(entry).monospace().small());
                                }
                            }
                        });
                });
            });
        if let Some(shown) = shown {
            self.hit_rects.push(shown.response.rect);
        }
        if apply {
            self.apply_byte_patch();
        }
        if close {
            self.inspect_addr.clear();
        }
        if let Some(i) = toggle_patch {
            let p = &mut self.patches[i];
            let (bytes, next) = if p.active {
                (&p.before, false)
            } else {
                (&p.after, true)
            };
            match proc.write(p.addr, bytes) {
                Ok(()) => {
                    p.active = next;
                    self.status = if next {
                        "patch reapplied".into()
                    } else {
                        "patch reverted".into()
                    };
                }
                Err(e) => self.status = format!("patch toggle failed: {e}"),
            }
        }
    }

    /// What stays on screen while the overlay is click-through.
    fn hud(&self, ctx: &egui::Context) {
        let Some(t) = &self.target else { return };
        let pinned: Vec<_> = self.table.iter().filter(|e| e.pinned).collect();
        if pinned.is_empty() {
            return;
        }
        egui::Area::new("hud".into())
            .anchor(Align2::LEFT_TOP, [24.0, 80.0])
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(theme::glass_fill(150))
                    .stroke(theme::edge())
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        for e in pinned {
                            let v = t
                                .proc
                                .read_bits(e.addr, e.ty.size())
                                .map(|b| Value::from_bits(e.ty, b));
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&e.label).small().color(DIM));
                                ui.label(
                                    RichText::new(v.map_or("??".into(), |v| v.to_string()))
                                        .monospace()
                                        .size(18.0)
                                        .color(if e.frozen.is_some() {
                                            ACCENT
                                        } else {
                                            Color32::WHITE
                                        }),
                                );
                                if e.frozen.is_some() {
                                    ui.label(RichText::new("❄").color(ACCENT));
                                }
                            });
                        }
                    });
            });
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // Never leave the game frozen or debugged when the overlay closes.
        self.detach();
    }
}
