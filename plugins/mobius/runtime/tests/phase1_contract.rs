use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

fn spawn_session_start(binary: &str, plugin_data: &Path) -> Child {
    Command::new(binary)
        .args(["hook", "session-start"])
        .env("PLUGIN_DATA", plugin_data)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("SessionStart process must spawn")
}

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
    for mode in ["mcp", "read", "audit", "doctor", "report", "hook"] {
        assert!(help.contains(mode), "help omitted {mode}");
    }
    assert!(help.contains("mobius hook session-start"));

    let mcp = Command::new(binary)
        .arg("mcp")
        .output()
        .expect("MCP mode must accept EOF");
    assert!(mcp.status.success());
    assert!(mcp.stdout.is_empty());

    let missing_read_input = Command::new(binary)
        .arg("read")
        .output()
        .expect("read mode must start");
    assert_eq!(missing_read_input.status.code(), Some(2));
    let error = String::from_utf8(missing_read_input.stderr).expect("error must be UTF-8");
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
fn concurrent_session_starts_emit_exactly_one_onboarding_context() {
    let binary = env!("CARGO_BIN_EXE_mobius");
    let plugin_data = std::env::temp_dir().join(format!(
        "mobius-session-start-contract-{}",
        uuid::Uuid::new_v4()
    ));
    let request = br#"{"session_id":"session-1","transcript_path":null,"cwd":"/project","hook_event_name":"SessionStart","model":"host-model","permission_mode":"default","source":"startup"}
"#;

    let mut children = [
        spawn_session_start(binary, &plugin_data),
        spawn_session_start(binary, &plugin_data),
    ];
    for child in &mut children {
        child
            .stdin
            .take()
            .expect("SessionStart stdin must be piped")
            .write_all(request)
            .expect("SessionStart input must be writable");
    }
    let outputs = children.map(|child| {
        let output = child
            .wait_with_output()
            .expect("SessionStart process must finish");
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        output.stdout
    });
    assert_eq!(
        outputs.iter().filter(|output| !output.is_empty()).count(),
        1,
        "the atomic claim must allow exactly one onboarding context"
    );
    let emitted = outputs
        .iter()
        .find(|output| !output.is_empty())
        .expect("one SessionStart must emit context");
    let emitted: serde_json::Value =
        serde_json::from_slice(emitted).expect("SessionStart output must be JSON");
    assert_eq!(
        emitted["hookSpecificOutput"]["hookEventName"],
        "SessionStart"
    );
    assert!(plugin_data.join("judge-onboarding-v1.claimed").is_file());

    let mut later = spawn_session_start(binary, &plugin_data);
    later
        .stdin
        .take()
        .expect("later SessionStart stdin must be piped")
        .write_all(request)
        .expect("later SessionStart input must be writable");
    let later = later
        .wait_with_output()
        .expect("later SessionStart process must finish");
    assert!(later.status.success());
    assert!(later.stdout.is_empty());
    assert!(later.stderr.is_empty());

    fs::remove_dir_all(plugin_data).expect("test plugin data must be removable");
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
