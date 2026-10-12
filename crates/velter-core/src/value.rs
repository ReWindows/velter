use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ValueType {
    I32,
    I64,
    F32,
    F64,
}

impl ValueType {
    pub const ALL: [ValueType; 4] = [Self::I32, Self::I64, Self::F32, Self::F64];

    pub fn size(self) -> usize {
        match self {
            Self::I32 | Self::F32 => 4,
            Self::I64 | Self::F64 => 8,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::I32 => "int32",
            Self::I64 => "int64",
            Self::F32 => "float",
            Self::F64 => "double",
        }
    }

    pub fn is_float(self) -> bool {
        matches!(self, Self::F32 | Self::F64)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
}

impl Value {
    pub fn parse(ty: ValueType, s: &str) -> Option<Value> {
        let s = s.trim().replace(['_', ','], "");
        Some(match ty {
            ValueType::I32 => Value::I32(parse_int(&s)? as i32),
            ValueType::I64 => Value::I64(parse_int(&s)?),
            ValueType::F32 => Value::F32(s.parse().ok()?),
            ValueType::F64 => Value::F64(s.parse().ok()?),
        })
    }

    pub fn ty(self) -> ValueType {
        match self {
            Value::I32(_) => ValueType::I32,
            Value::I64(_) => ValueType::I64,
            Value::F32(_) => ValueType::F32,
            Value::F64(_) => ValueType::F64,
        }
    }

    /// Decode from the low `ty.size()` bytes of `bits` (little endian).
    pub fn from_bits(ty: ValueType, bits: u64) -> Value {
        match ty {
            ValueType::I32 => Value::I32(bits as u32 as i32),
            ValueType::I64 => Value::I64(bits as i64),
            ValueType::F32 => Value::F32(f32::from_bits(bits as u32)),
            ValueType::F64 => Value::F64(f64::from_bits(bits)),
        }
    }

    pub fn to_bits(self) -> u64 {
        match self {
            Value::I32(v) => v as u32 as u64,
            Value::I64(v) => v as u64,
            Value::F32(v) => v.to_bits() as u64,
            Value::F64(v) => v.to_bits(),
        }
    }

    pub fn to_bytes(self) -> Vec<u8> {
        self.to_bits().to_le_bytes()[..self.ty().size()].to_vec()
    }

    pub fn as_f64(self) -> f64 {
        match self {
            Value::I32(v) => v as f64,
            Value::I64(v) => v as f64,
            Value::F32(v) => v as f64,
            Value::F64(v) => v,
        }
    }
}

/// Accepts decimal or 0x-prefixed hex, and k/m suffixes ("300k").
fn parse_int(s: &str) -> Option<i64> {
    let lower = s.to_ascii_lowercase();
    if let Some(hex) = lower.strip_prefix("0x") {
        return i64::from_str_radix(hex, 16).ok();
    }
    let (num, mul) = match lower.as_bytes().last()? {
        b'k' => (&lower[..lower.len() - 1], 1_000),
        b'm' => (&lower[..lower.len() - 1], 1_000_000),
        _ => (lower.as_str(), 1),
    };
    num.parse::<i64>().ok()?.checked_mul(mul)
}

/// Read a `ty`-sized little-endian value out of `bytes` as raw bits.
#[inline]
pub(crate) fn bits_at(ty: ValueType, bytes: &[u8]) -> u64 {
    if ty.size() == 4 {
        u32::from_le_bytes(bytes[..4].try_into().unwrap()) as u64
    } else {
        u64::from_le_bytes(bytes[..8].try_into().unwrap())
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::I32(v) => write!(f, "{v}"),
            Value::I64(v) => write!(f, "{v}"),
            Value::F32(v) => write!(f, "{v}"),
            Value::F64(v) => write!(f, "{v}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_suffixes_and_hex() {
        assert_eq!(
            Value::parse(ValueType::I32, "300k"),
            Some(Value::I32(300_000))
        );
        assert_eq!(
            Value::parse(ValueType::I64, "2m"),
            Some(Value::I64(2_000_000))
        );
        assert_eq!(Value::parse(ValueType::I32, "0x10"), Some(Value::I32(16)));
        assert_eq!(
            Value::parse(ValueType::I32, "1,000"),
            Some(Value::I32(1000))
        );
    }

    #[test]
    fn bits_roundtrip() {
        for v in [
            Value::I32(-5),
            Value::I64(-5),
            Value::F32(1.5),
            Value::F64(-2.25),
        ] {
            assert_eq!(Value::from_bits(v.ty(), v.to_bits()), v);
        }
    }
}
