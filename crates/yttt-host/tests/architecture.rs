const FORBIDDEN_CONCRETE_TRANSPORTS: &[&str] = &[
    "yttt-transport-local",
    "iroh",
    "iroh-quinn",
    "quinn",
    "libp2p",
];

#[test]
fn host_production_dependencies_exclude_concrete_transports() {
    let manifest = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));
    let forbidden = forbidden_production_deps(manifest);
    assert!(
        forbidden.is_empty(),
        "yttt-host production dependencies must not include concrete transports: {forbidden:?}"
    );
}

fn forbidden_production_deps(manifest: &str) -> Vec<&'static str> {
    let tables = production_dependency_tables(manifest);
    FORBIDDEN_CONCRETE_TRANSPORTS
        .iter()
        .copied()
        .filter(|name| tables.iter().any(|table| declares_dependency(table, name)))
        .collect()
}

fn production_dependency_tables(manifest: &str) -> Vec<&str> {
    let mut tables = Vec::new();
    for line in manifest.lines() {
        if !is_production_dependency_table(line.trim()) {
            continue;
        }
        let start = manifest
            .find(line)
            .expect("table header must exist in manifest");
        let body_start = start + line.len();
        let body_end = manifest[body_start..]
            .find("\n[")
            .map(|offset| body_start + offset)
            .unwrap_or(manifest.len());
        tables.push(&manifest[body_start..body_end]);
    }
    tables
}

fn is_production_dependency_table(header: &str) -> bool {
    let Some(name) = header
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    else {
        return false;
    };
    name == "dependencies"
        || (name.ends_with(".dependencies")
            && !name.ends_with(".dev-dependencies")
            && !name.ends_with(".build-dependencies"))
}

fn declares_dependency(table: &str, name: &str) -> bool {
    table.lines().any(|line| {
        let line = line.trim();
        !line.starts_with('#')
            && (line == name
                || line.starts_with(&format!("{name} "))
                || line.starts_with(&format!("{name}=")))
    })
}
