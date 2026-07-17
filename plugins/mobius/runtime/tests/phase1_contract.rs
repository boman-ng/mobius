use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn runtime_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn files_below(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path).expect("source directory must be readable") {
            let entry = entry.expect("directory entry must be readable");
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[test]
fn package_declares_one_mobius_binary_and_no_library_target() {
    let root = runtime_root();
    let manifest =
        fs::read_to_string(root.join("Cargo.toml")).expect("Cargo.toml must be readable");

    assert_eq!(manifest.matches("[[bin]]").count(), 1);
    assert!(manifest.contains("name = \"mobius\""));
    assert!(!manifest.contains("[lib]"));
    assert!(!root.join("src/lib.rs").exists());
    assert!(!root.join("src/bin").exists());
}

#[test]
fn runtime_modes_are_explicit_and_closed_over_the_supported_surface() {
    let binary = env!("CARGO_BIN_EXE_mobius");
    let help = Command::new(binary)
        .arg("--help")
        .output()
        .expect("mobius help must start");
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).expect("help must be UTF-8");
    for mode in ["mcp", "audit", "doctor", "report", "hook"] {
        assert!(help.contains(mode), "help omitted {mode}");
    }

    let mcp = Command::new(binary)
        .arg("mcp")
        .output()
        .expect("MCP mode must accept EOF");
    assert!(mcp.status.success());
    assert!(mcp.stdout.is_empty());

    let removed_read_mode = Command::new(binary)
        .arg("read")
        .output()
        .expect("removed read mode must fail cleanly");
    assert_eq!(removed_read_mode.status.code(), Some(2));
    let error = String::from_utf8(removed_read_mode.stderr).expect("error must be UTF-8");
    assert!(error.contains("\"code\":\"invalid_invocation\""));

    let unknown = Command::new(binary)
        .arg("mutate")
        .output()
        .expect("unknown mode must fail cleanly");
    assert_eq!(unknown.status.code(), Some(2));
    assert!(
        String::from_utf8(unknown.stderr)
            .expect("error must be UTF-8")
            .contains("\"code\":\"invalid_invocation\"")
    );
}

#[test]
fn domain_source_has_no_outward_dependency_or_ambient_input() {
    let domain = runtime_root().join("src/domain");
    let forbidden = [
        "std::fs",
        "std::env",
        "std::time",
        "crate::application",
        "crate::infrastructure",
        "crate::presentation",
        "crate::transport",
    ];

    for path in files_below(&domain) {
        let source = fs::read_to_string(&path).expect("domain source must be UTF-8");
        for pattern in forbidden {
            assert!(
                !source.contains(pattern),
                "{} contains forbidden dependency {pattern}",
                path.display()
            );
        }
    }
}
