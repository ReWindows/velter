//! Cheat-Engine style value scanning: first scan, then narrow with next scans.

use rayon::prelude::*;
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_NOACCESS,
    PAGE_READWRITE, PAGE_WRITECOPY,
};

use crate::value::bits_at;
use crate::{Process, Value, ValueType};

/// Values are assumed to be 4-byte aligned (what CE calls "fast scan").
const ALIGN: usize = 4;
const CHUNK: usize = 4 << 20;
/// Hard cap so scanning for `0` doesn't eat all RAM.
pub const MAX_HITS: usize = 20_000_000;
const USER_SPACE_END: u64 = 0x7FFF_FFFF_0000;

#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub base: u64,
    pub size: u64,
}

/// Committed, writable, accessible memory: where game values live.
pub fn writable_regions(p: &Process) -> Vec<Region> {
    let writable =
        PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY;
    let mut out = Vec::new();
    let mut addr = 0u64;
    while addr < USER_SPACE_END {
        let Some(mbi) = p.query(addr) else { break };
        let base = mbi.BaseAddress as u64;
        let size = mbi.RegionSize as u64;
        if mbi.State == MEM_COMMIT
            && mbi.Protect & writable != 0
            && mbi.Protect & (PAGE_GUARD | PAGE_NOACCESS) == 0
        {
            out.push(Region { base, size });
        }
        let next = base.saturating_add(size);
        if next <= addr {
            break;
        }
        addr = next;
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScanCmp {
    Exact(Value),
    Changed,
    Unchanged,
    Increased,
    Decreased,
}

impl ScanCmp {
    pub fn label(&self) -> &'static str {
        match self {
            ScanCmp::Exact(_) => "exact",
            ScanCmp::Changed => "changed",
            ScanCmp::Unchanged => "unchanged",
            ScanCmp::Increased => "increased",
            ScanCmp::Decreased => "decreased",
        }
    }
}

/// Builds a predicate over (previous bits, current bits).
fn predicate(ty: ValueType, cmp: ScanCmp) -> impl Fn(u64, u64) -> bool + Sync {
    let target = match cmp {
        ScanCmp::Exact(v) => Some(v),
        _ => None,
    };
    move |prev: u64, cur: u64| {
        let c = Value::from_bits(ty, cur);
        match cmp {
            ScanCmp::Exact(_) => {
                let t = target.unwrap();
                if ty.is_float() {
                    // Games rarely store the exact decimal you see on screen.
                    let (a, b) = (c.as_f64(), t.as_f64());
                    a.is_finite() && (a - b).abs() <= (b.abs() * 1e-4).max(0.01)
                } else {
                    cur == t.to_bits()
                }
            }
            ScanCmp::Changed => cur != prev,
            ScanCmp::Unchanged => cur == prev,
            ScanCmp::Increased => c.as_f64() > Value::from_bits(ty, prev).as_f64(),
            ScanCmp::Decreased => c.as_f64() < Value::from_bits(ty, prev).as_f64(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub addr: u64,
    /// Value at the last scan, as raw bits.
    pub bits: u64,
}

pub struct Scanner {
    pub ty: ValueType,
    pub hits: Vec<Hit>,
    pub truncated: bool,
    pub scans: u32,
}

impl Scanner {
    /// First scan. Only `Exact` makes sense here (there's no previous value yet).
    pub fn first(p: &Process, ty: ValueType, value: Value) -> Scanner {
        let size = ty.size();
        let pred = predicate(ty, ScanCmp::Exact(value));
        let regions = writable_regions(p);

        // Split regions into chunks so big heaps parallelize too.
        let chunks: Vec<(u64, usize)> = regions
            .iter()
            .flat_map(|r| {
                (0..r.size)
                    .step_by(CHUNK)
                    .map(move |off| (r.base + off, (r.size - off).min(CHUNK as u64) as usize))
            })
            .collect();

        let per_chunk: Vec<Vec<Hit>> = chunks
            .par_iter()
            .map_init(
                || vec![0u8; CHUNK],
                |buf, &(base, len)| {
                    let n = p.read(base, &mut buf[..len]);
                    let mut hits = Vec::new();
                    let mut off = 0;
                    while off + size <= n {
                        let bits = bits_at(ty, &buf[off..]);
                        if pred(0, bits) {
                            hits.push(Hit {
                                addr: base + off as u64,
                                bits,
                            });
                        }
                        off += ALIGN;
                    }
                    hits
                },
            )
            .collect();

        let mut hits = Vec::new();
        let mut truncated = false;
        for h in per_chunk {
            if hits.len() + h.len() > MAX_HITS {
                hits.extend_from_slice(&h[..MAX_HITS - hits.len()]);
                truncated = true;
                break;
            }
            hits.extend(h);
        }
        Scanner {
            ty,
            hits,
            truncated,
            scans: 1,
        }
    }

    /// Next scan: keep only hits whose current value satisfies `cmp`.
    pub fn next(&mut self, p: &Process, cmp: ScanCmp) {
        let ty = self.ty;
        let size = ty.size();
        let pred = predicate(ty, cmp);

        // Hits are sorted by address; read them in spans of nearby addresses.
        const SPAN: u64 = 64 * 1024;
        let mut groups: Vec<&[Hit]> = Vec::new();
        let mut start = 0;
        for i in 1..=self.hits.len() {
            if i == self.hits.len() || self.hits[i].addr - self.hits[start].addr >= SPAN {
                groups.push(&self.hits[start..i]);
                start = i;
            }
        }

        let kept: Vec<Vec<Hit>> = groups
            .par_iter()
            .map_init(
                || vec![0u8; SPAN as usize + 8],
                |buf, g| {
                    let base = g[0].addr;
                    let len = (g[g.len() - 1].addr - base) as usize + size;
                    let n = p.read(base, &mut buf[..len]);
                    g.iter()
                        .filter_map(|h| {
                            let off = (h.addr - base) as usize;
                            if off + size > n {
                                return None;
                            }
                            let cur = bits_at(ty, &buf[off..]);
                            pred(h.bits, cur).then_some(Hit {
                                addr: h.addr,
                                bits: cur,
                            })
                        })
                        .collect()
                },
            )
            .collect();

        self.hits = kept.into_iter().flatten().collect();
        self.scans += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scan our own process for a value we control.
    #[test]
    fn finds_and_narrows_own_value() {
        let me = std::process::id();
        let p = Process::open(me, "self").unwrap();
        let mut secret = Box::new(0x1337_4242i32);
        let addr = &*secret as *const i32 as u64;

        let mut s = Scanner::first(&p, ValueType::I32, Value::I32(0x1337_4242));
        assert!(s.hits.iter().any(|h| h.addr == addr));

        *secret = 0x1337_4243;
        std::hint::black_box(&secret);
        s.next(&p, ScanCmp::Increased);
        assert!(s.hits.iter().any(|h| h.addr == addr));
        s.next(&p, ScanCmp::Exact(Value::I32(0x1337_4243)));
        assert!(s.hits.iter().any(|h| h.addr == addr));
    }
}
