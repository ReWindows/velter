use std::path::Path;

use crate::{
    Assembly, Error, Field, Method, Result, RuntimeKind, RuntimeModel, Type, checked_range,
};

const MAGIC: u32 = 0xFAB1_1BAF;

#[derive(Clone, Copy)]
struct Layout {
    label: &'static str,
    images_word: usize,
    type_stride: usize,
    type_field_start: usize,
    type_method_start: usize,
    type_method_count: usize,
    type_field_count: usize,
    type_token: usize,
    method_stride: usize,
    method_token: usize,
    method_parameter_count: usize,
}

const V24_1: Layout = Layout {
    label: "24.1",
    images_word: 44,
    type_stride: 100,
    type_field_start: 44,
    type_method_start: 48,
    type_method_count: 76,
    type_field_count: 80,
    type_token: 96,
    method_stride: 52,
    method_token: 40,
    method_parameter_count: 50,
};

const V31: Layout = Layout {
    label: "31",
    images_word: 42,
    type_stride: 88,
    type_field_start: 32,
    type_method_start: 36,
    type_method_count: 64,
    type_field_count: 68,
    type_token: 84,
    method_stride: 36,
    method_token: 24,
    method_parameter_count: 34,
};

pub fn load_il2cpp_metadata(path: impl AsRef<Path>) -> Result<RuntimeModel> {
    let bytes = std::fs::read(path)?;
    if read_u32(&bytes, 0, "header.magic")? != MAGIC {
        return Err(Error::Format("invalid IL2CPP metadata magic".into()));
    }
    let raw_version = read_u32(&bytes, 4, "header.version")?;
    let layout = match raw_version {
        24 if looks_like_v24_1(&bytes)? => V24_1,
        24 => {
            return Err(Error::Format(
                "metadata v24 layout is not v24.1; Unity version selection is required".into(),
            ));
        }
        31 => V31,
        other => {
            return Err(Error::Format(format!(
                "IL2CPP metadata v{other} is not implemented by Velter"
            )));
        }
    };
    load_layout(&bytes, layout)
}

fn looks_like_v24_1(bytes: &[u8]) -> Result<bool> {
    let images = section(bytes, V24_1.images_word, "images")?;
    if images.is_empty() || images.len() % 40 != 0 {
        return Ok(false);
    }
    Ok((0..images.len() / 40).all(|i| le_u32(images, i * 40 + 28) == Some(1)))
}

fn load_layout(bytes: &[u8], layout: Layout) -> Result<RuntimeModel> {
    let strings = section(bytes, 6, "strings")?;
    let image_data = section(bytes, layout.images_word, "images")?;
    let type_data = section(bytes, 40, "type definitions")?;
    let method_data = section(bytes, 12, "method definitions")?;
    let field_data = section(bytes, 24, "field definitions")?;
    validate_stride(image_data, 40, "images", layout.label)?;
    validate_stride(
        type_data,
        layout.type_stride,
        "type definitions",
        layout.label,
    )?;
    validate_stride(
        method_data,
        layout.method_stride,
        "method definitions",
        layout.label,
    )?;
    validate_stride(field_data, 12, "field definitions", layout.label)?;

    let image_count = image_data.len() / 40;
    let type_count = type_data.len() / layout.type_stride;
    let method_count = method_data.len() / layout.method_stride;
    let field_count = field_data.len() / 12;
    let mut assemblies = Vec::with_capacity(image_count);

    for image_index in 0..image_count {
        let image = image_index * 40;
        let name = metadata_string(strings, read_u32(image_data, image, "image.name")?)?;
        let type_start = read_i32(image_data, image + 8, "image.type_start")?;
        let owned_types = read_u32(image_data, image + 12, "image.type_count")? as usize;
        let type_range = checked_range("image.type", type_start, owned_types, type_count)?;
        let mut types = Vec::with_capacity(owned_types);

        for type_index in type_range {
            let base = type_index * layout.type_stride;
            let field_start = read_i32(
                type_data,
                base + layout.type_field_start,
                "type.field_start",
            )?;
            let method_start = read_i32(
                type_data,
                base + layout.type_method_start,
                "type.method_start",
            )?;
            let owned_methods = read_u16(
                type_data,
                base + layout.type_method_count,
                "type.method_count",
            )? as usize;
            let owned_fields = read_u16(
                type_data,
                base + layout.type_field_count,
                "type.field_count",
            )? as usize;

            let fields = checked_range("type.field", field_start, owned_fields, field_count)?
                .map(|index| {
                    let at = index * 12;
                    Ok(Field {
                        name: metadata_string(strings, read_u32(field_data, at, "field.name")?)?,
                        type_index: Some(read_i32(field_data, at + 4, "field.type")?),
                        token: read_u32(field_data, at + 8, "field.token")?,
                        offset: None,
                    })
                })
                .collect::<Result<Vec<_>>>()?;

            let methods = checked_range("type.method", method_start, owned_methods, method_count)?
                .map(|index| {
                    let at = index * layout.method_stride;
                    Ok(Method {
                        name: metadata_string(strings, read_u32(method_data, at, "method.name")?)?,
                        token: read_u32(method_data, at + layout.method_token, "method.token")?,
                        parameter_count: read_u16(
                            method_data,
                            at + layout.method_parameter_count,
                            "method.parameter_count",
                        )?,
                        metadata_index: index,
                        rva: None,
                    })
                })
                .collect::<Result<Vec<_>>>()?;

            types.push(Type {
                name: metadata_string(strings, read_u32(type_data, base, "type.name")?)?,
                namespace: metadata_string(
                    strings,
                    read_u32(type_data, base + 4, "type.namespace")?,
                )?,
                token: read_u32(type_data, base + layout.type_token, "type.token")?,
                fields,
                methods,
            });
        }
        assemblies.push(Assembly { name, types });
    }

    Ok(RuntimeModel {
        kind: RuntimeKind::Il2Cpp,
        format_version: layout.label.into(),
        assemblies,
    })
}

fn section<'a>(bytes: &'a [u8], offset_word: usize, name: &'static str) -> Result<&'a [u8]> {
    let offset = read_u32(bytes, offset_word * 4, name)? as usize;
    let size = read_u32(bytes, (offset_word + 1) * 4, name)? as usize;
    let end = offset
        .checked_add(size)
        .ok_or_else(|| Error::Format(format!("{name} range overflow")))?;
    bytes.get(offset..end).ok_or_else(|| {
        Error::Format(format!(
            "{name} range {offset:#x}..{end:#x} exceeds file size {:#x}",
            bytes.len()
        ))
    })
}

fn validate_stride(data: &[u8], stride: usize, name: &str, version: &str) -> Result<()> {
    if data.len() % stride != 0 {
        Err(Error::Format(format!(
            "{name} size {} is not divisible by v{version} stride {stride}",
            data.len()
        )))
    } else {
        Ok(())
    }
}

fn metadata_string(strings: &[u8], index: u32) -> Result<String> {
    let tail = strings.get(index as usize..).ok_or(Error::Bounds {
        kind: "string",
        index: index as i64,
        len: strings.len(),
    })?;
    let end = tail
        .iter()
        .position(|b| *b == 0)
        .ok_or_else(|| Error::Format(format!("unterminated metadata string at {index:#x}")))?;
    Ok(String::from_utf8_lossy(&tail[..end]).into_owned())
}

fn le_u32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
}
fn read_u32(data: &[u8], at: usize, name: &str) -> Result<u32> {
    le_u32(data, at).ok_or_else(|| Error::Format(format!("truncated {name} at {at:#x}")))
}
fn read_i32(data: &[u8], at: usize, name: &str) -> Result<i32> {
    Ok(read_u32(data, at, name)? as i32)
}
fn read_u16(data: &[u8], at: usize, name: &str) -> Result<u16> {
    let raw = data
        .get(at..at + 2)
        .ok_or_else(|| Error::Format(format!("truncated {name} at {at:#x}")))?;
    Ok(u16::from_le_bytes(
        raw.try_into().expect("checked two-byte slice"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_bad_magic() {
        let path = std::env::temp_dir().join("velter-bad-metadata.dat");
        std::fs::write(&path, [0u8; 16]).unwrap();
        let result = load_il2cpp_metadata(&path);
        std::fs::remove_file(path).ok();
        assert!(matches!(result, Err(Error::Format(_))));
    }
}
