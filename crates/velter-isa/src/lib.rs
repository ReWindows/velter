//! Small declarative ISA format. It is intentionally strict so generated
//! decoders fail at build time instead of silently choosing an encoding.

use std::collections::BTreeSet;
use std::fmt;
use velter_ir::Endian;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Register {
    pub name: String,
    pub index: u16,
    pub bits: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Instruction {
    pub name: String,
    pub mask: u64,
    pub value: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IsaSpec {
    pub architecture: String,
    pub bits: u16,
    pub endian: Endian,
    pub delay_slots: u8,
    pub registers: Vec<Register>,
    pub instructions: Vec<Instruction>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}
impl std::error::Error for ParseError {}

impl IsaSpec {
    pub fn parse(source: &str) -> Result<Self, ParseError> {
        let mut architecture = None;
        let mut bits = None;
        let mut endian = None;
        let mut delay_slots = 0;
        let mut registers = Vec::new();
        let mut instructions = Vec::new();
        for (index, raw) in source.lines().enumerate() {
            let line = index + 1;
            let text = raw.split('#').next().unwrap_or("").trim();
            if text.is_empty() {
                continue;
            }
            let parts: Vec<_> = text.split_whitespace().collect();
            let err = |message: &str| ParseError {
                line,
                message: message.into(),
            };
            match parts.as_slice() {
                ["architecture", value] => architecture = Some((*value).to_owned()),
                ["bits", value] => bits = Some(parse_u16(value, line)?),
                ["endian", "little"] => endian = Some(Endian::Little),
                ["endian", "big"] => endian = Some(Endian::Big),
                ["delay_slots", value] => {
                    delay_slots = value.parse().map_err(|_| err("invalid delay slot count"))?
                }
                ["register", name, index, width] => registers.push(Register {
                    name: (*name).into(),
                    index: parse_u16(index, line)?,
                    bits: parse_u16(width, line)?,
                }),
                ["instruction", name, mask, value] => instructions.push(Instruction {
                    name: (*name).into(),
                    mask: parse_number(mask, line)?,
                    value: parse_number(value, line)?,
                }),
                _ => return Err(err("unknown or malformed directive")),
            }
        }
        let missing = |name: &str| ParseError {
            line: 0,
            message: format!("missing {name} directive"),
        };
        let spec = Self {
            architecture: architecture.ok_or_else(|| missing("architecture"))?,
            bits: bits.ok_or_else(|| missing("bits"))?,
            endian: endian.ok_or_else(|| missing("endian"))?,
            delay_slots,
            registers,
            instructions,
        };
        spec.validate()?;
        Ok(spec)
    }

    pub fn decode(&self, word: u64) -> Option<&Instruction> {
        self.instructions
            .iter()
            .find(|item| word & item.mask == item.value)
    }

    fn validate(&self) -> Result<(), ParseError> {
        if self.bits == 0 || self.bits > 64 {
            return Err(ParseError {
                line: 0,
                message: "architecture width must be 1..=64".into(),
            });
        }
        let mut names = BTreeSet::new();
        let mut indexes = BTreeSet::new();
        for register in &self.registers {
            if register.bits == 0 {
                return Err(ParseError {
                    line: 0,
                    message: format!("register {} has zero width", register.name),
                });
            }
            if !names.insert(&register.name) || !indexes.insert(register.index) {
                return Err(ParseError {
                    line: 0,
                    message: "duplicate register name or index".into(),
                });
            }
        }
        for (position, instruction) in self.instructions.iter().enumerate() {
            if instruction.value & !instruction.mask != 0 {
                return Err(ParseError {
                    line: 0,
                    message: format!("{} has value bits outside its mask", instruction.name),
                });
            }
            for other in &self.instructions[..position] {
                let shared = instruction.mask & other.mask;
                if instruction.value & shared == other.value & shared {
                    return Err(ParseError {
                        line: 0,
                        message: format!(
                            "ambiguous encodings: {} and {}",
                            other.name, instruction.name
                        ),
                    });
                }
            }
        }
        Ok(())
    }
}

fn parse_u16(text: &str, line: usize) -> Result<u16, ParseError> {
    text.parse().map_err(|_| ParseError {
        line,
        message: format!("invalid integer: {text}"),
    })
}

fn parse_number(text: &str, line: usize) -> Result<u64, ParseError> {
    let result = text
        .strip_prefix("0x")
        .map_or_else(|| text.parse(), |hex| u64::from_str_radix(hex, 16));
    result.map_err(|_| ParseError {
        line,
        message: format!("invalid number: {text}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const PSX: &str = "architecture psx-r3000a\nbits 32\nendian little\ndelay_slots 1\nregister r0 0 32\ninstruction nop 0xffffffff 0x00000000\ninstruction j 0xfc000000 0x08000000\n";

    #[test]
    fn parses_and_decodes_psx_subset() {
        let isa = IsaSpec::parse(PSX).unwrap();
        assert_eq!(isa.delay_slots, 1);
        assert_eq!(isa.decode(0).unwrap().name, "nop");
        assert_eq!(isa.decode(0x0800_1234).unwrap().name, "j");
    }

    #[test]
    fn rejects_ambiguous_encodings() {
        let source = format!("{PSX}instruction also_nop 0xffffffff 0x00000000\n");
        assert!(
            IsaSpec::parse(&source)
                .unwrap_err()
                .message
                .contains("ambiguous")
        );
    }
}
