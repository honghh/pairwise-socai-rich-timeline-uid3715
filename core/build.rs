use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};

fn main() {
    let crate_root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let sites_root = crate_root.join("src/sites");
    println!("cargo:rerun-if-changed={}", sites_root.display());

    let mut site_dirs = fs::read_dir(&sites_root)
        .expect("read src/sites")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .filter(|path| path.join("manifest.json").is_file())
        .collect::<Vec<_>>();
    site_dirs.sort();

    let mut generated =
        String::from("pub(crate) static BUILTIN_SITE_SKILLS: &[EmbeddedSiteSkill] = &[\n");
    for site_dir in site_dirs {
        append_site(&mut generated, &site_dir);
    }
    generated.push_str("];\n");

    let output =
        PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("site_skill_assets.rs");
    fs::write(output, generated).expect("write generated site skill assets");
}

fn append_site(generated: &mut String, site_dir: &Path) {
    let manifest_path = site_dir.join("manifest.json");
    assert_regular_file(&manifest_path, "site manifest");
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    let manifest_text = fs::read_to_string(&manifest_path).expect("read site manifest");
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_text).expect("parse site manifest");
    let id = manifest
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("site manifest id");
    let directory = site_dir
        .file_name()
        .and_then(|name| name.to_str())
        .expect("site directory name");
    assert_eq!(id, directory, "site manifest id must match directory");
    let canonical_site = fs::canonicalize(site_dir).expect("resolve site skill directory");

    let mut resources = BTreeSet::new();
    if let Some(notes) = manifest.get("notes").and_then(serde_json::Value::as_array) {
        for path in notes {
            resources.insert(resource_path(path, "note"));
        }
    }
    if let Some(tools) = manifest
        .get("browserTools")
        .and_then(serde_json::Value::as_object)
    {
        for tool in tools.values() {
            resources.insert(resource_path(
                tool.get("path").expect("browser tool path"),
                "browser tool",
            ));
        }
    }

    generated.push_str("    EmbeddedSiteSkill {\n");
    generated.push_str(&format!("        id: {id:?},\n"));
    generated.push_str(&format!(
        "        manifest: include_str!({:?}),\n",
        manifest_path.to_string_lossy()
    ));
    generated.push_str("        files: &[\n");
    for relative in resources {
        let source = site_dir.join(&relative);
        assert_regular_file(&source, "site skill resource");
        let canonical_source = fs::canonicalize(&source).expect("resolve site skill resource");
        assert!(
            canonical_source.starts_with(&canonical_site),
            "site skill resource escapes package: {}",
            source.display()
        );
        println!("cargo:rerun-if-changed={}", source.display());
        generated.push_str(&format!(
            "            EmbeddedSiteSkillFile {{ path: {relative:?}, contents: include_str!({:?}) }},\n",
            source.to_string_lossy()
        ));
    }
    generated.push_str("        ],\n    },\n");
}

fn resource_path(value: &serde_json::Value, label: &str) -> String {
    let path = value
        .as_str()
        .unwrap_or_else(|| panic!("{label} path must be a string"));
    let candidate = Path::new(path);
    assert!(
        !path.contains('\\') && !candidate.is_absolute(),
        "{label} path must be relative"
    );
    assert!(
        candidate
            .components()
            .all(|part| matches!(part, Component::Normal(_))),
        "{label} path must stay inside the site directory"
    );
    path.to_string()
}

fn assert_regular_file(path: &Path, label: &str) {
    let metadata = fs::symlink_metadata(path)
        .unwrap_or_else(|error| panic!("failed to stat {label} {}: {error}", path.display()));
    assert!(
        !metadata.file_type().is_symlink() && metadata.is_file(),
        "{label} must be a regular file: {}",
        path.display()
    );
}
