use crate::{Assembly, Error, Field, Method, Result, RuntimeKind, RuntimeModel, Type};
use dnfile::DnPe;
use dnfile::stream::meta_data_tables::mdtables::{Field as DnField, MethodDef, TypeDef};
use std::path::Path;

pub fn load_mono_assembly(path: impl AsRef<Path>) -> Result<RuntimeModel> {
    let bytes = std::fs::read(path.as_ref())?;
    let pe = DnPe::parse(&bytes).map_err(|e| Error::Format(e.to_string()))?;
    let clr = pe.net().map_err(|e| Error::Format(e.to_string()))?;
    let assembly_name = clr.assembly().map(|a| a.name.clone()).unwrap_or_else(|_| {
        path.as_ref()
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    });
    let type_table = clr
        .md_table("TypeDef")
        .map_err(|e| Error::Format(e.to_string()))?;
    let method_table = clr
        .md_table("MethodDef")
        .map_err(|e| Error::Format(e.to_string()))?;
    let field_table = clr
        .md_table("Field")
        .map_err(|e| Error::Format(e.to_string()))?;
    let mut types = Vec::with_capacity(type_table.row_count());
    for i in 0..type_table.row_count() {
        let def = type_table
            .row::<TypeDef>(i)
            .map_err(|e| Error::Format(e.to_string()))?;
        let mut methods = Vec::with_capacity(def.method_list.len());
        for index in &def.method_list {
            let Some(row) = index.row_index.checked_sub(1) else {
                continue;
            };
            let m = method_table
                .row::<MethodDef>(row)
                .map_err(|e| Error::Format(e.to_string()))?;
            methods.push(Method {
                name: m.name.clone(),
                token: 0x0600_0001 + row as u32,
                parameter_count: 0,
                metadata_index: row,
                rva: Some(m.rva as u64),
            });
        }
        let mut fields = Vec::with_capacity(def.field_list.len());
        for index in &def.field_list {
            let Some(row) = index.row_index.checked_sub(1) else {
                continue;
            };
            let f = field_table
                .row::<DnField>(row)
                .map_err(|e| Error::Format(e.to_string()))?;
            fields.push(Field {
                name: f.name.clone(),
                token: 0x0400_0001 + row as u32,
                type_index: None,
                offset: None,
            });
        }
        types.push(Type {
            namespace: def.type_namespace.clone(),
            name: def.type_name.clone(),
            token: 0x0200_0001 + i as u32,
            fields,
            methods,
        });
    }
    Ok(RuntimeModel {
        kind: RuntimeKind::Mono,
        format_version: "ECMA-335".into(),
        assemblies: vec![Assembly {
            name: assembly_name,
            types,
        }],
    })
}
