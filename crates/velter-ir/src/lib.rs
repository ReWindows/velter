//! Typed, architecture-neutral intermediate representation used by analysis,
//! emulation, rewriting, and decompilation.

use std::collections::BTreeSet;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Endian {
    Little,
    Big,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct RegisterId(pub String);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct BlockId(pub u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Provenance {
    Static,
    Runtime,
    Imported,
    User,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Evidence {
    pub provenance: Provenance,
    pub confidence: u8,
}

impl Evidence {
    pub fn new(provenance: Provenance, confidence: u8) -> Self {
        Self {
            provenance,
            confidence: confidence.min(100),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Value {
    Register(RegisterId),
    Temporary(u32),
    Constant { value: u128, bits: u16 },
}

impl Value {
    fn validate(&self) -> Result<(), ValidationError> {
        if matches!(self, Self::Constant { bits: 0, .. }) {
            return Err(ValidationError::ZeroWidthValue);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    And,
    Or,
    Xor,
    Shl,
    Shr,
    Equal,
    LessSigned,
    LessUnsigned,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Operation {
    Copy {
        dst: Value,
        src: Value,
    },
    Binary {
        dst: Value,
        op: BinaryOp,
        lhs: Value,
        rhs: Value,
    },
    Load {
        dst: Value,
        address: Value,
        bits: u16,
        endian: Endian,
    },
    Store {
        address: Value,
        value: Value,
        bits: u16,
        endian: Endian,
    },
    Intrinsic {
        name: String,
        outputs: Vec<Value>,
        inputs: Vec<Value>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Terminator {
    Return,
    Jump(BlockId),
    Branch {
        condition: Value,
        taken: BlockId,
        not_taken: BlockId,
    },
    /// A branch whose following instruction executes before control transfers.
    Delayed {
        slots: u8,
        branch: Box<Terminator>,
    },
    Trap {
        code: u32,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BasicBlock {
    pub id: BlockId,
    pub address: u64,
    pub operations: Vec<Operation>,
    pub terminator: Terminator,
    pub evidence: Evidence,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Function {
    pub name: String,
    pub entry: BlockId,
    pub blocks: Vec<BasicBlock>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidationError {
    MissingEntry(BlockId),
    DuplicateBlock(BlockId),
    MissingSuccessor { from: BlockId, to: BlockId },
    InvalidDelaySlots,
    NestedDelayedBranch,
    ZeroWidthMemory,
    ZeroWidthValue,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ValidationError {}

impl Function {
    pub fn validate(&self) -> Result<(), ValidationError> {
        let mut ids = BTreeSet::new();
        for block in &self.blocks {
            if !ids.insert(block.id) {
                return Err(ValidationError::DuplicateBlock(block.id));
            }
        }
        if !ids.contains(&self.entry) {
            return Err(ValidationError::MissingEntry(self.entry));
        }
        for block in &self.blocks {
            for operation in &block.operations {
                validate_operation(operation)?;
            }
            validate_terminator(block.id, &block.terminator, &ids, false)?;
        }
        Ok(())
    }
}

fn validate_operation(operation: &Operation) -> Result<(), ValidationError> {
    match operation {
        Operation::Copy { dst, src } => {
            dst.validate()?;
            src.validate()?;
        }
        Operation::Binary { dst, lhs, rhs, .. } => {
            dst.validate()?;
            lhs.validate()?;
            rhs.validate()?;
        }
        Operation::Load {
            dst, address, bits, ..
        } => {
            if *bits == 0 {
                return Err(ValidationError::ZeroWidthMemory);
            }
            dst.validate()?;
            address.validate()?;
        }
        Operation::Store {
            address,
            value,
            bits,
            ..
        } => {
            if *bits == 0 {
                return Err(ValidationError::ZeroWidthMemory);
            }
            address.validate()?;
            value.validate()?;
        }
        Operation::Intrinsic {
            outputs, inputs, ..
        } => {
            for value in outputs.iter().chain(inputs) {
                value.validate()?;
            }
        }
    }
    Ok(())
}

fn validate_terminator(
    from: BlockId,
    term: &Terminator,
    ids: &BTreeSet<BlockId>,
    nested: bool,
) -> Result<(), ValidationError> {
    let check = |to| {
        if ids.contains(&to) {
            Ok(())
        } else {
            Err(ValidationError::MissingSuccessor { from, to })
        }
    };
    match term {
        Terminator::Return | Terminator::Trap { .. } => Ok(()),
        Terminator::Jump(to) => check(*to),
        Terminator::Branch {
            condition,
            taken,
            not_taken,
        } => {
            condition.validate()?;
            check(*taken)?;
            check(*not_taken)
        }
        Terminator::Delayed { slots, branch } => {
            if nested {
                return Err(ValidationError::NestedDelayedBranch);
            }
            if *slots == 0 {
                return Err(ValidationError::InvalidDelaySlots);
            }
            validate_terminator(from, branch, ids, true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn evidence() -> Evidence {
        Evidence::new(Provenance::Static, 90)
    }

    #[test]
    fn validates_delay_slot_cfg() {
        let function = Function {
            name: "branch".into(),
            entry: BlockId(0),
            blocks: vec![
                BasicBlock {
                    id: BlockId(0),
                    address: 0x8000_0000,
                    operations: vec![],
                    terminator: Terminator::Delayed {
                        slots: 1,
                        branch: Box::new(Terminator::Jump(BlockId(1))),
                    },
                    evidence: evidence(),
                },
                BasicBlock {
                    id: BlockId(1),
                    address: 0x8000_0008,
                    operations: vec![],
                    terminator: Terminator::Return,
                    evidence: evidence(),
                },
            ],
        };
        assert_eq!(function.validate(), Ok(()));
    }

    #[test]
    fn rejects_missing_successor() {
        let function = Function {
            name: "bad".into(),
            entry: BlockId(0),
            blocks: vec![BasicBlock {
                id: BlockId(0),
                address: 0,
                operations: vec![],
                terminator: Terminator::Jump(BlockId(9)),
                evidence: evidence(),
            }],
        };
        assert!(matches!(
            function.validate(),
            Err(ValidationError::MissingSuccessor { .. })
        ));
    }
}
