//! Managed-runtime metadata normalized for Velter's GUI and CLI.

mod il2cpp;
mod mono;

use std::path::Path;

pub use il2cpp::load_il2cpp_metadata;
pub use mono::load_mono_assembly;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeKind {
    Mono,
    Il2Cpp,
}

#[derive(Clone, Debug)]
pub struct RuntimeModel {
    pub kind: RuntimeKind,
    pub format_version: String,
    pub assemblies: Vec<Assembly>,
}

#[derive(Clone, Debug)]
pub struct Assembly {
    pub name: String,
    pub types: Vec<Type>,
}

#[derive(Clone, Debug)]
pub struct Type {
    pub namespace: String,
    pub name: String,
    pub token: u32,
    pub fields: Vec<Field>,
    pub methods: Vec<Method>,
}

#[derive(Clone, Debug)]
pub struct Field {
    pub name: String,
    pub token: u32,
    pub type_index: Option<i32>,
    pub offset: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct Method {
    pub name: String,
    pub token: u32,
    pub parameter_count: u16,
    pub metadata_index: usize,
    pub rva: Option<u64>,
}

impl RuntimeModel {
    pub fn type_count(&self) -> usize {
        self.assemblies.iter().map(|a| a.types.len()).sum()
    }
    pub fn method_count(&self) -> usize {
        self.assemblies
            .iter()
            .flat_map(|a| &a.types)
            .map(|t| t.methods.len())
            .sum()
    }
    pub fn field_count(&self) -> usize {
        self.assemblies
            .iter()
            .flat_map(|a| &a.types)
            .map(|t| t.fields.len())
            .sum()
    }

    pub fn search<'a>(&'a self, needle: &str) -> Vec<(&'a Assembly, &'a Type)> {
        let needle = needle.to_ascii_lowercase();
        let mut found = Vec::new();
        for a in &self.assemblies {
            for t in &a.types {
                let full = format!("{}.{}", t.namespace, t.name).to_ascii_lowercase();
                if full.contains(&needle)
                    || t.methods
                        .iter()
                        .any(|m| m.name.to_ascii_lowercase().contains(&needle))
                    || t.fields
                        .iter()
                        .any(|f| f.name.to_ascii_lowercase().contains(&needle))
                {
                    found.push((a, t));
                }
            }
        }
        found
    }
}

pub fn load_auto(path: impl AsRef<Path>) -> Result<RuntimeModel> {
    let path = path.as_ref();
    if path
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("global-metadata.dat"))
    {
        load_il2cpp_metadata(path)
    } else {
        load_mono_assembly(path)
    }
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Format(String),
    Bounds {
        kind: &'static str,
        index: i64,
        len: usize,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O: {error}"),
            Self::Format(message) => write!(f, "invalid or unsupported metadata: {message}"),
            Self::Bounds { kind, index, len } => {
                write!(
                    f,
                    "metadata index {kind}={index} is out of bounds (len={len})"
                )
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

fn checked_range(
    kind: &'static str,
    start: i32,
    count: usize,
    len: usize,
) -> Result<std::ops::Range<usize>> {
    // IL2CPP uses -1 as the start sentinel for an empty owned range.
    if count == 0 {
        return Ok(0..0);
    }
    let start = usize::try_from(start).map_err(|_| Error::Bounds {
        kind,
        index: start as i64,
        len,
    })?;
    let end = start
        .checked_add(count)
        .filter(|e| *e <= len)
        .ok_or(Error::Bounds {
            kind,
            index: start as i64 + count as i64,
            len,
        })?;
    Ok(start..end)
}
