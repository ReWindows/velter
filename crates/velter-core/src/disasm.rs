use iced_x86::{Decoder, DecoderOptions, Formatter, IntelFormatter};

use crate::Process;

#[derive(Clone, Debug)]
pub struct Instruction {
    pub address: u64,
    pub bytes: Vec<u8>,
    pub text: String,
}

/// Decode a live x86-64 code window. Short reads are accepted at page edges.
pub fn disassemble(process: &Process, address: u64, byte_count: usize) -> Vec<Instruction> {
    let mut bytes = vec![0; byte_count.clamp(1, 4096)];
    let read = process.read(address, &mut bytes);
    bytes.truncate(read);
    let mut decoder = Decoder::with_ip(64, &bytes, address, DecoderOptions::NONE);
    let mut formatter = IntelFormatter::new();
    formatter.options_mut().set_digit_separator("_");
    formatter.options_mut().set_first_operand_char_index(10);
    let mut lines = Vec::new();
    while decoder.can_decode() {
        let instruction = decoder.decode();
        let start = (instruction.ip() - address) as usize;
        let end = start.saturating_add(instruction.len()).min(bytes.len());
        let mut text = String::new();
        formatter.format(&instruction, &mut text);
        lines.push(Instruction {
            address: instruction.ip(),
            bytes: bytes[start..end].to_vec(),
            text,
        });
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_live_bytes() {
        let code = Box::new([0x48u8, 0x89, 0xD8, 0xC3]); // mov rax,rbx; ret
        let address = code.as_ptr() as u64;
        let process = Process::open(std::process::id(), "self").unwrap();
        let lines = disassemble(&process, address, code.len());
        assert_eq!(lines.len(), 2);
        assert!(lines[0].text.starts_with("mov"));
        assert_eq!(lines[1].text, "ret");
    }
}
