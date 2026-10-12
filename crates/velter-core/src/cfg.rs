use crate::Process;
use iced_x86::{Decoder, DecoderOptions, FlowControl};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct BasicBlock {
    pub start: u64,
    pub end: u64,
    pub instruction_count: usize,
    pub successors: Vec<u64>,
}

/// Recover basic blocks and direct edges from a bounded live code window.
pub fn control_flow_graph(process: &Process, address: u64, byte_count: usize) -> Vec<BasicBlock> {
    let mut bytes = vec![0; byte_count.clamp(1, 16384)];
    let read = process.read(address, &mut bytes);
    bytes.truncate(read);
    let end = address + bytes.len() as u64;
    let mut decoder = Decoder::with_ip(64, &bytes, address, DecoderOptions::NONE);
    let mut instructions = Vec::new();
    while decoder.can_decode() {
        instructions.push(decoder.decode());
    }
    if instructions.is_empty() {
        return Vec::new();
    }
    let mut leaders = BTreeSet::from([address]);
    for i in &instructions {
        let next = i.next_ip();
        match i.flow_control() {
            FlowControl::ConditionalBranch => {
                let target = i.near_branch_target();
                if (address..end).contains(&target) {
                    leaders.insert(target);
                }
                if next < end {
                    leaders.insert(next);
                }
            }
            FlowControl::UnconditionalBranch => {
                let target = i.near_branch_target();
                if (address..end).contains(&target) {
                    leaders.insert(target);
                }
                if next < end {
                    leaders.insert(next);
                }
            }
            FlowControl::Return | FlowControl::IndirectBranch | FlowControl::Exception => {
                if next < end {
                    leaders.insert(next);
                }
            }
            _ => {}
        }
    }
    let mut groups: Vec<Vec<&iced_x86::Instruction>> = Vec::new();
    for i in &instructions {
        if leaders.contains(&i.ip()) || groups.is_empty() {
            groups.push(Vec::new());
        }
        groups.last_mut().unwrap().push(i);
    }
    groups
        .into_iter()
        .filter(|g| !g.is_empty())
        .map(|g| {
            let first = g[0];
            let last = g[g.len() - 1];
            let next = last.next_ip();
            let mut successors = Vec::new();
            match last.flow_control() {
                FlowControl::ConditionalBranch => {
                    successors.push(last.near_branch_target());
                    if next < end {
                        successors.push(next);
                    }
                }
                FlowControl::UnconditionalBranch => successors.push(last.near_branch_target()),
                FlowControl::Return | FlowControl::IndirectBranch | FlowControl::Exception => {}
                _ => {
                    if next < end {
                        successors.push(next);
                    }
                }
            }
            successors.retain(|x| (address..end).contains(x));
            successors.sort_unstable();
            successors.dedup();
            BasicBlock {
                start: first.ip(),
                end: last.next_ip(),
                instruction_count: g.len(),
                successors,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_conditional_flow() {
        let code = Box::new([0x75u8, 0x02, 0x90, 0xC3, 0x90, 0xC3]);
        let address = code.as_ptr() as u64;
        let process = Process::open(std::process::id(), "self").unwrap();
        let blocks = control_flow_graph(&process, address, code.len());
        assert!(blocks.len() >= 3);
        assert_eq!(blocks[0].successors.len(), 2);
    }
}
