use velter_runtime::load_auto;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: velter-meta <assembly.dll|global-metadata.dat> [search]");
        std::process::exit(2);
    };
    let started = std::time::Instant::now();
    match load_auto(&path) {
        Ok(model) => {
            println!(
                "{:?} v{} · {} assemblies · {} types · {} fields · {} methods · {:?}",
                model.kind,
                model.format_version,
                model.assemblies.len(),
                model.type_count(),
                model.field_count(),
                model.method_count(),
                started.elapsed()
            );
            if let Some(query) = args.next() {
                for (assembly, ty) in model.search(&query).into_iter().take(100) {
                    println!(
                        "{} :: {}.{} · {} fields · {} methods",
                        assembly.name,
                        ty.namespace,
                        ty.name,
                        ty.fields.len(),
                        ty.methods.len()
                    );
                    for method in ty
                        .methods
                        .iter()
                        .filter(|m| {
                            m.name
                                .to_ascii_lowercase()
                                .contains(&query.to_ascii_lowercase())
                        })
                        .take(10)
                    {
                        println!(
                            "  0x{:08x} {}({} params)",
                            method.token, method.name, method.parameter_count
                        );
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("velter-meta: {e}");
            std::process::exit(1);
        }
    }
}
