#[path = "support/evidence_bundle.rs"]
mod evidence_bundle;

use evidence_bundle::{
    ArtifactIdentity, ArtifactSetIdentity, Assessment, CANONICALIZATION, Classification,
    CurrentMaterial, EVIDENCE_BUNDLE_SCHEMA, EmptyIdentity, EvidenceBundle, ExternalObjectIdentity,
    ExternalObjectSetIdentity, MAX_CANONICAL_BUNDLE_BYTES, MaterialBaseline, MaterialScope,
    ObservedEffects, RepositoryCapture, RepositoryCaptureSpec, RepositoryWorktreeIdentity,
    ToolchainInput, ToolchainSource, Verification, canonicalize, capture_repository_worktree,
    evaluate, evaluate_json, sha256_identity,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

fn digest(label: &str) -> String {
    sha256_identity(label.as_bytes())
}

fn scope(include: &[&str], exclude: &[&str]) -> MaterialScope {
    MaterialScope {
        include: include.iter().map(|value| (*value).to_owned()).collect(),
        exclude: exclude.iter().map(|value| (*value).to_owned()).collect(),
    }
}

fn artifact(logical_id: &str, content: &str) -> ArtifactIdentity {
    ArtifactIdentity {
        logical_id: logical_id.to_owned(),
        digest: digest(content),
        size_bytes: u64::try_from(content.len()).expect("synthetic content length fits u64"),
    }
}

fn external(logical_id: &str, version: &str) -> ExternalObjectIdentity {
    ExternalObjectIdentity {
        logical_id: logical_id.to_owned(),
        locator: format!("https://example.invalid/objects/{logical_id}"),
        immutable_version: Some(version.to_owned()),
        content_digest: None,
    }
}

fn repository_identity(seed: &str) -> RepositoryWorktreeIdentity {
    RepositoryWorktreeIdentity {
        base_tree: "a".repeat(40),
        tracked_delta_digest: digest(&format!("tracked-{seed}")),
        untracked_manifest_digest: digest(&format!("untracked-{seed}")),
        toolchain_config_digest: digest(&format!("toolchain-{seed}")),
    }
}

fn bundle(material_baseline: MaterialBaseline) -> EvidenceBundle {
    EvidenceBundle {
        schema: EVIDENCE_BUNDLE_SCHEMA.to_owned(),
        canonicalization: CANONICALIZATION.to_owned(),
        material_baseline,
        verification: vec![
            Verification {
                check_id: "check-build".to_owned(),
                command_or_method: "synthetic deterministic build check".to_owned(),
                exit_status: evidence_bundle::ExitStatus::Code(0),
                output_identity: digest("build-output"),
                assessment: Assessment::Supports,
            },
            Verification {
                check_id: "check-review".to_owned(),
                command_or_method: "synthetic review observation".to_owned(),
                exit_status: evidence_bundle::ExitStatus::NotApplicable,
                output_identity: digest("review-output"),
                assessment: Assessment::Unknown,
            },
        ],
        observed_effects: ObservedEffects {
            changed_surfaces: vec!["zeta/output".to_owned(), "alpha/output".to_owned()],
        },
        counterevidence: vec!["counter-z".to_owned(), "counter-a".to_owned()],
        limits: vec!["limit-z".to_owned(), "limit-a".to_owned()],
    }
}

fn artifact_baseline(before: ArtifactSetIdentity, after: ArtifactSetIdentity) -> MaterialBaseline {
    MaterialBaseline::ArtifactSet {
        scope: scope(&["reports/z", "reports/a"], &["reports/archive"]),
        before,
        after,
    }
}

fn external_baseline(
    before: ExternalObjectSetIdentity,
    after: ExternalObjectSetIdentity,
) -> MaterialBaseline {
    MaterialBaseline::ExternalObjectSet {
        scope: scope(&["external/releases"], &[]),
        before,
        after,
    }
}

fn intrinsic_bundle() -> EvidenceBundle {
    bundle(MaterialBaseline::Intrinsic {
        scope: MaterialScope::default(),
        before: EmptyIdentity::default(),
        after: EmptyIdentity::default(),
        observation_identity: digest("complete intrinsic observation"),
    })
}

fn assert_invalid_json(value: Value) {
    let encoded = serde_json::to_vec(&value).expect("synthetic JSON encodes");
    let evaluation = evaluate_json(&encoded, None);
    assert_eq!(evaluation.classification, Classification::Invalid);
    assert!(!evaluation.valid);
    assert!(!evaluation.coherent);
    assert!(!evaluation.current_applicable);
    assert!(evaluation.canonical.is_none());
    assert!(!evaluation.errors.is_empty());
}

#[test]
fn canonical_json_normalizes_sets_and_preserves_verification_order() {
    let entries = vec![artifact("zeta", "z"), artifact("alpha", "a")];
    let mut first = bundle(artifact_baseline(
        ArtifactSetIdentity {
            entries: entries.clone(),
        },
        ArtifactSetIdentity {
            entries: entries.clone(),
        },
    ));
    if let MaterialBaseline::ArtifactSet { scope, .. } = &mut first.material_baseline {
        scope
            .include
            .extend(["reports/a".to_owned(), "reports/z".to_owned()]);
        scope.exclude.push("reports/archive".to_owned());
    }
    first
        .observed_effects
        .changed_surfaces
        .push("alpha/output".to_owned());
    first.counterevidence.push("counter-a".to_owned());
    first.limits.push("limit-z".to_owned());

    let mut second = first.clone();
    if let MaterialBaseline::ArtifactSet {
        scope,
        before,
        after,
    } = &mut second.material_baseline
    {
        scope.include.reverse();
        scope.exclude.reverse();
        before.entries.reverse();
        after.entries.reverse();
    }
    second.observed_effects.changed_surfaces.reverse();
    second.counterevidence.reverse();
    second.limits.reverse();

    let first_canonical = canonicalize(&first).expect("first bundle is valid");
    let repeated = canonicalize(&first).expect("repeat canonicalization succeeds");
    let second_canonical = canonicalize(&second).expect("second bundle is valid");
    assert_eq!(first_canonical.bytes, repeated.bytes);
    assert_eq!(first_canonical.identity, repeated.identity);
    assert_eq!(first_canonical.bytes, second_canonical.bytes);
    assert_eq!(first_canonical.identity, second_canonical.identity);

    let canonical_text =
        std::str::from_utf8(&first_canonical.bytes).expect("canonical JSON is UTF-8");
    assert!(canonical_text.starts_with("{\"canonicalization\":"));
    assert!(!canonical_text.contains('\n'));
    assert!(!canonical_text.contains("timestamp"));
    assert!(!canonical_text.contains("/home/"));
    assert_eq!(
        first_canonical.bundle.observed_effects.changed_surfaces,
        ["alpha/output", "zeta/output"]
    );

    let mut semantic_reordering = first.clone();
    semantic_reordering.verification.reverse();
    assert_ne!(
        first_canonical.bytes,
        canonicalize(&semantic_reordering)
            .expect("reordered verification remains valid")
            .bytes,
        "verification order is semantic and must not be normalized"
    );

    let current = CurrentMaterial::ArtifactSet {
        scope: scope(
            &["reports/z", "reports/a", "reports/z"],
            &["reports/archive"],
        ),
        identity: ArtifactSetIdentity {
            entries: entries.into_iter().rev().collect(),
        },
    };
    let evaluation = evaluate(&first, Some(&current));
    assert_eq!(evaluation.classification, Classification::CurrentApplicable);
    assert!(evaluation.valid && evaluation.coherent && evaluation.current_applicable);
}

#[test]
fn all_four_baseline_kinds_can_be_current_applicable() {
    let repository = repository_identity("same");
    let repository_scope = scope(&["src", "Cargo.lock"], &[]);
    let repository_bundle = bundle(MaterialBaseline::RepositoryWorktree {
        scope: repository_scope.clone(),
        before: repository.clone(),
        after: repository.clone(),
    });
    let repository_current = CurrentMaterial::RepositoryWorktree(RepositoryCapture {
        scope: repository_scope,
        identity: repository,
    });

    let artifacts = ArtifactSetIdentity {
        entries: vec![artifact("report", "report bytes")],
    };
    let artifact_bundle = bundle(artifact_baseline(artifacts.clone(), artifacts.clone()));
    let artifact_current = CurrentMaterial::ArtifactSet {
        scope: scope(&["reports/z", "reports/a"], &["reports/archive"]),
        identity: artifacts,
    };

    let external_objects = ExternalObjectSetIdentity {
        entries: vec![
            ExternalObjectIdentity {
                logical_id: "snapshot".to_owned(),
                locator: "https://example.invalid/objects/snapshot".to_owned(),
                immutable_version: None,
                content_digest: Some(digest("snapshot-v1")),
            },
            external("release", "release-v1"),
        ],
    };
    let external_bundle = bundle(external_baseline(
        external_objects.clone(),
        external_objects.clone(),
    ));
    let external_current = CurrentMaterial::ExternalObjectSet {
        scope: scope(&["external/releases"], &[]),
        identity: external_objects,
    };

    for (candidate, current) in [
        (&repository_bundle, Some(&repository_current)),
        (&artifact_bundle, Some(&artifact_current)),
        (&external_bundle, Some(&external_current)),
        (&intrinsic_bundle(), None),
    ] {
        let evaluation = evaluate(candidate, current);
        assert_eq!(evaluation.classification, Classification::CurrentApplicable);
        assert!(evaluation.valid);
        assert!(evaluation.coherent);
        assert!(evaluation.current_applicable);
        assert!(evaluation.errors.is_empty());
    }
}

#[test]
fn unsupported_or_malformed_common_fields_are_invalid() {
    let valid = serde_json::to_value(intrinsic_bundle()).expect("bundle serializes");

    let mut unsupported_schema = valid.clone();
    unsupported_schema["schema"] = json!("mobius.evidence-bundle.v2");
    assert_invalid_json(unsupported_schema);

    let mut unsupported_canonicalization = valid.clone();
    unsupported_canonicalization["canonicalization"] = json!("mobius.canonical-json.v2");
    assert_invalid_json(unsupported_canonicalization);

    let mut missing_field = valid.clone();
    missing_field
        .as_object_mut()
        .expect("bundle is an object")
        .remove("limits");
    assert_invalid_json(missing_field);

    let mut unknown_timestamp = valid.clone();
    unknown_timestamp["timestamp"] = json!("2026-07-20T00:00:00Z");
    assert_invalid_json(unknown_timestamp);

    let mut malformed_digest = valid.clone();
    malformed_digest["verification"][0]["output_identity"] =
        json!(format!("sha256:{}", "A".repeat(64)));
    assert_invalid_json(malformed_digest);

    let mut empty_verification = valid.clone();
    empty_verification["verification"] = json!([]);
    assert_invalid_json(empty_verification);

    let mut duplicate_check = valid.clone();
    duplicate_check["verification"][1]["check_id"] = json!("check-build");
    assert_invalid_json(duplicate_check);

    let mut invalid_exit_status = valid.clone();
    invalid_exit_status["verification"][0]["exit_status"] = json!("success");
    assert_invalid_json(invalid_exit_status);

    let mut personal_path = valid;
    personal_path["verification"][0]["command_or_method"] =
        json!("read /home/example/private-output");
    assert_invalid_json(personal_path);
}

#[test]
fn canonical_bundle_budget_accepts_the_boundary_and_rejects_oversize() {
    let mut candidate = intrinsic_bundle();
    candidate.limits = vec!["x".to_owned()];
    let base_len = canonicalize(&candidate)
        .expect("small bundle is valid")
        .bytes
        .len();
    let exact_text_len = 1 + (MAX_CANONICAL_BUNDLE_BYTES - base_len);
    candidate.limits = vec!["x".repeat(exact_text_len)];
    assert_eq!(
        canonicalize(&candidate)
            .expect("the exact byte budget is admitted")
            .bytes
            .len(),
        MAX_CANONICAL_BUNDLE_BYTES
    );

    candidate.limits = vec!["x".repeat(exact_text_len + 1)];
    let evaluation = evaluate(&candidate, None);
    assert_eq!(evaluation.classification, Classification::Invalid);
    assert!(!evaluation.valid);
    assert!(evaluation.canonical.is_none());
    assert!(
        evaluation.errors[0].contains("exceeds 131072 bytes"),
        "oversize failure must identify the fixed contract budget"
    );
}

#[test]
fn unsafe_material_locators_are_invalid() {
    let artifacts = ArtifactSetIdentity {
        entries: vec![artifact("report", "report")],
    };
    let valid = serde_json::to_value(bundle(artifact_baseline(artifacts.clone(), artifacts)))
        .expect("bundle serializes");

    for locator in [
        "",
        ".",
        "..",
        "/absolute",
        "C:/absolute",
        "a//b",
        "a/./b",
        "a/../b",
        "a\\b",
        "~/private",
    ] {
        let mut candidate = valid.clone();
        candidate["material_baseline"]["scope"]["include"] = json!([locator]);
        assert_invalid_json(candidate);
    }
}

#[test]
fn kind_specific_missing_or_unstable_identities_are_invalid() {
    let repository = repository_identity("valid");
    let repository_bundle = bundle(MaterialBaseline::RepositoryWorktree {
        scope: scope(&["src"], &[]),
        before: repository.clone(),
        after: repository,
    });
    let mut missing_repository_field =
        serde_json::to_value(repository_bundle).expect("bundle serializes");
    let mut malformed_repository_digest = missing_repository_field.clone();
    malformed_repository_digest["material_baseline"]["before"]["tracked_delta_digest"] =
        json!("sha256:not-a-digest");
    assert_invalid_json(malformed_repository_digest);
    missing_repository_field["material_baseline"]["before"]
        .as_object_mut()
        .expect("before is an object")
        .remove("tracked_delta_digest");
    assert_invalid_json(missing_repository_field);

    let duplicated = ArtifactSetIdentity {
        entries: vec![artifact("same", "one"), artifact("same", "two")],
    };
    assert_invalid_json(
        serde_json::to_value(bundle(artifact_baseline(duplicated.clone(), duplicated)))
            .expect("bundle serializes"),
    );

    let empty_artifacts = ArtifactSetIdentity {
        entries: Vec::new(),
    };
    assert_invalid_json(
        serde_json::to_value(bundle(artifact_baseline(
            empty_artifacts.clone(),
            empty_artifacts,
        )))
        .expect("bundle serializes"),
    );

    let mutable_only = ExternalObjectSetIdentity {
        entries: vec![ExternalObjectIdentity {
            logical_id: "release".to_owned(),
            locator: "https://example.invalid/releases/latest".to_owned(),
            immutable_version: None,
            content_digest: None,
        }],
    };
    assert_invalid_json(
        serde_json::to_value(bundle(external_baseline(
            mutable_only.clone(),
            mutable_only,
        )))
        .expect("bundle serializes"),
    );

    let unstable_version = ExternalObjectSetIdentity {
        entries: vec![external("release", "latest")],
    };
    assert_invalid_json(
        serde_json::to_value(bundle(external_baseline(
            unstable_version.clone(),
            unstable_version,
        )))
        .expect("bundle serializes"),
    );

    let duplicate_external = ExternalObjectSetIdentity {
        entries: vec![
            external("release", "release-v1"),
            external("release", "release-v2"),
        ],
    };
    assert_invalid_json(
        serde_json::to_value(bundle(external_baseline(
            duplicate_external.clone(),
            duplicate_external,
        )))
        .expect("bundle serializes"),
    );

    let malformed_external_digest = ExternalObjectSetIdentity {
        entries: vec![ExternalObjectIdentity {
            logical_id: "snapshot".to_owned(),
            locator: "https://example.invalid/objects/snapshot".to_owned(),
            immutable_version: None,
            content_digest: Some("sha256:short".to_owned()),
        }],
    };
    assert_invalid_json(
        serde_json::to_value(bundle(external_baseline(
            malformed_external_digest.clone(),
            malformed_external_digest,
        )))
        .expect("bundle serializes"),
    );

    let mut intrinsic_with_scope = intrinsic_bundle();
    if let MaterialBaseline::Intrinsic { scope, .. } = &mut intrinsic_with_scope.material_baseline {
        scope.include.push("mutable/input".to_owned());
    }
    assert_invalid_json(serde_json::to_value(intrinsic_with_scope).expect("bundle serializes"));
}

#[test]
fn incoherent_superseded_and_unverifiable_remain_distinct() {
    let old = ArtifactSetIdentity {
        entries: vec![artifact("report", "old")],
    };
    let new = ArtifactSetIdentity {
        entries: vec![artifact("report", "new")],
    };

    let incoherent = bundle(artifact_baseline(old.clone(), new.clone()));
    let current_new = CurrentMaterial::ArtifactSet {
        scope: scope(&["reports/a", "reports/z"], &["reports/archive"]),
        identity: new.clone(),
    };
    let incoherent_result = evaluate(&incoherent, Some(&current_new));
    assert_eq!(incoherent_result.classification, Classification::Incoherent);
    assert!(incoherent_result.valid);
    assert!(!incoherent_result.coherent);

    let coherent_old = bundle(artifact_baseline(old.clone(), old.clone()));
    let superseded = evaluate(&coherent_old, Some(&current_new));
    assert_eq!(superseded.classification, Classification::Superseded);
    assert!(superseded.valid && superseded.coherent);
    assert!(!superseded.current_applicable);

    let unavailable = evaluate(&coherent_old, None);
    assert_eq!(unavailable.classification, Classification::Unverifiable);
    assert!(unavailable.valid && unavailable.coherent);
    assert!(!unavailable.errors.is_empty());

    let wrong_scope = CurrentMaterial::ArtifactSet {
        scope: scope(&["reports/other"], &[]),
        identity: old.clone(),
    };
    assert_eq!(
        evaluate(&coherent_old, Some(&wrong_scope)).classification,
        Classification::Unverifiable
    );

    let current_old = CurrentMaterial::ArtifactSet {
        scope: scope(&["reports/z", "reports/a"], &["reports/archive"]),
        identity: old,
    };
    assert_eq!(
        evaluate(&coherent_old, Some(&current_old)).classification,
        Classification::CurrentApplicable
    );
}

#[test]
fn artifact_content_and_external_version_changes_supersede_evidence() {
    let artifact_v1 = ArtifactSetIdentity {
        entries: vec![artifact("report", "v1")],
    };
    let artifact_v2 = ArtifactSetIdentity {
        entries: vec![artifact("report", "v2")],
    };
    let artifact_bundle = bundle(artifact_baseline(artifact_v1.clone(), artifact_v1));
    let artifact_now = CurrentMaterial::ArtifactSet {
        scope: scope(&["reports/a", "reports/z"], &["reports/archive"]),
        identity: artifact_v2,
    };
    assert_eq!(
        evaluate(&artifact_bundle, Some(&artifact_now)).classification,
        Classification::Superseded
    );

    let external_v1 = ExternalObjectSetIdentity {
        entries: vec![external("release", "release-v1")],
    };
    let external_v2 = ExternalObjectSetIdentity {
        entries: vec![external("release", "release-v2")],
    };
    let external_bundle = bundle(external_baseline(external_v1.clone(), external_v1));
    let external_now = CurrentMaterial::ExternalObjectSet {
        scope: scope(&["external/releases"], &[]),
        identity: external_v2,
    };
    assert_eq!(
        evaluate(&external_bundle, Some(&external_now)).classification,
        Classification::Superseded
    );
}

struct GitWorkspace {
    root: PathBuf,
}

impl GitWorkspace {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("mobius-evidence-bundle-test-{}", Uuid::new_v4()));
        fs::create_dir(&root).expect("temporary Git workspace is created");
        let workspace = Self { root };
        workspace.git(&["init", "--quiet"]);
        workspace.git(&["config", "user.email", "tests@example.invalid"]);
        workspace.git(&["config", "user.name", "Mobius Contract Test"]);
        workspace.write("tracked.txt", b"tracked-v1\n");
        workspace.write("config/toolchain.txt", b"toolchain-v1\n");
        workspace.git(&["add", "--all"]);
        workspace.git(&["commit", "--quiet", "-m", "initial"]);
        workspace
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn write(&self, locator: &str, bytes: &[u8]) {
        let path = self.root.join(locator);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("fixture parent directory is created");
        }
        fs::write(path, bytes).expect("fixture file is written");
    }

    fn remove_file(&self, locator: &str) {
        fs::remove_file(self.root.join(locator)).expect("fixture file is removed");
    }

    fn git(&self, arguments: &[&str]) -> Vec<u8> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(arguments)
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .output()
            .expect("Git executes in the synthetic workspace");
        assert!(
            output.status.success(),
            "Git fixture command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }

    fn git_line(&self, arguments: &[&str]) -> String {
        String::from_utf8(self.git(arguments))
            .expect("Git fixture identity is UTF-8")
            .trim()
            .to_owned()
    }
}

impl Drop for GitWorkspace {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.root) {
            eprintln!(
                "failed to remove synthetic Evidence Bundle workspace {}: {error}",
                self.root.display()
            );
        }
    }
}

fn repository_spec() -> RepositoryCaptureSpec {
    RepositoryCaptureSpec {
        scope: scope(
            &[
                "untracked.txt",
                "tracked.txt",
                "build",
                "target",
                ".tmp",
                ".mobius",
                "tracked.txt",
            ],
            &[],
        ),
        resolved_cargo_target_dirs: Some(vec![
            "build/cargo-target".to_owned(),
            "target".to_owned(),
            "build/cargo-target".to_owned(),
        ]),
        toolchain: vec![
            ToolchainInput {
                logical_id: "rust-version".to_owned(),
                source: ToolchainSource::Value {
                    bytes: b"rustc-synthetic-v1".to_vec(),
                },
            },
            ToolchainInput {
                logical_id: "build-config".to_owned(),
                source: ToolchainSource::File {
                    locator: "config/toolchain.txt".to_owned(),
                },
            },
        ],
    }
}

#[test]
fn repository_capture_is_repeatable_and_detects_each_material_surface() {
    let workspace = GitWorkspace::new();
    workspace.write("target/committed.txt", b"root target v1\n");
    workspace.write("build/cargo-target/committed.txt", b"custom target v1\n");
    workspace.git(&[
        "add",
        "target/committed.txt",
        "build/cargo-target/committed.txt",
    ]);
    workspace.git(&["commit", "--quiet", "-m", "add managed target fixtures"]);
    let spec = repository_spec();

    let clean =
        capture_repository_worktree(workspace.root(), &spec).expect("clean capture succeeds");
    let repeated =
        capture_repository_worktree(workspace.root(), &spec).expect("repeat capture succeeds");
    assert_eq!(clean, repeated);
    assert_eq!(
        clean.identity.base_tree,
        workspace.git_line(&["rev-parse", "HEAD^{tree}"])
    );
    assert_eq!(
        clean.scope.include,
        [
            ".mobius",
            ".tmp",
            "build",
            "target",
            "tracked.txt",
            "untracked.txt"
        ]
    );
    assert_eq!(
        clean.scope.exclude,
        [".mobius", ".tmp", "build/cargo-target", "target"]
    );

    workspace.write("tracked.txt", b"tracked-v2\n");
    let tracked =
        capture_repository_worktree(workspace.root(), &spec).expect("tracked capture succeeds");
    assert_ne!(
        clean.identity.tracked_delta_digest,
        tracked.identity.tracked_delta_digest
    );
    assert_eq!(
        clean.identity.untracked_manifest_digest,
        tracked.identity.untracked_manifest_digest
    );
    assert_eq!(
        clean.identity.toolchain_config_digest,
        tracked.identity.toolchain_config_digest
    );

    workspace.write("tracked.txt", b"tracked-v1\n");
    assert_eq!(
        clean,
        capture_repository_worktree(workspace.root(), &spec)
            .expect("restored tracked capture succeeds")
    );

    workspace.write("untracked.txt", b"untracked-v1\0binary");
    let untracked =
        capture_repository_worktree(workspace.root(), &spec).expect("untracked capture succeeds");
    assert_eq!(
        clean.identity.tracked_delta_digest,
        untracked.identity.tracked_delta_digest
    );
    assert_ne!(
        clean.identity.untracked_manifest_digest,
        untracked.identity.untracked_manifest_digest
    );
    workspace.remove_file("untracked.txt");

    workspace.write("config/toolchain.txt", b"toolchain-v2\n");
    let toolchain =
        capture_repository_worktree(workspace.root(), &spec).expect("toolchain capture succeeds");
    assert_eq!(
        clean.identity.tracked_delta_digest,
        toolchain.identity.tracked_delta_digest
    );
    assert_eq!(
        clean.identity.untracked_manifest_digest,
        toolchain.identity.untracked_manifest_digest
    );
    assert_ne!(
        clean.identity.toolchain_config_digest,
        toolchain.identity.toolchain_config_digest
    );
    workspace.write("config/toolchain.txt", b"toolchain-v1\n");

    let mut reordered_spec = spec.clone();
    reordered_spec.toolchain.reverse();
    assert_eq!(
        clean,
        capture_repository_worktree(workspace.root(), &reordered_spec)
            .expect("toolchain input order is normalized")
    );
    let mut duplicate_toolchain_spec = spec.clone();
    duplicate_toolchain_spec
        .toolchain
        .push(spec.toolchain[0].clone());
    assert!(
        capture_repository_worktree(workspace.root(), &duplicate_toolchain_spec).is_err(),
        "duplicate declared toolchain ids must fail closed"
    );
    let version_input = reordered_spec
        .toolchain
        .iter_mut()
        .find(|input| input.logical_id == "rust-version")
        .expect("synthetic version input is declared");
    if let ToolchainSource::Value { bytes } = &mut version_input.source {
        *bytes = b"rustc-synthetic-v2".to_vec();
    } else {
        panic!("synthetic version input must carry value bytes");
    }
    let tool_version = capture_repository_worktree(workspace.root(), &reordered_spec)
        .expect("changed tool version capture succeeds");
    assert_ne!(
        clean.identity.toolchain_config_digest,
        tool_version.identity.toolchain_config_digest
    );

    workspace.write(".mobius/state", b"managed");
    workspace.write(".tmp/log", b"temporary");
    workspace.write("target/committed.txt", b"root target v2\n");
    workspace.write("build/cargo-target/committed.txt", b"custom target v2\n");
    workspace.write("target/build", b"generated");
    workspace.write("target/debug/deep/generated", b"generated descendant");
    workspace.write("build/cargo-target/build", b"custom generated");
    workspace.write(
        "build/cargo-target/debug/deep/generated",
        b"custom generated descendant",
    );
    assert_eq!(
        clean,
        capture_repository_worktree(workspace.root(), &spec)
            .expect("resolved managed state does not affect capture")
    );

    workspace.write(
        "build/cargo-target-sibling/ordinary.txt",
        b"ordinary sibling material",
    );
    let sibling = capture_repository_worktree(workspace.root(), &spec)
        .expect("a custom-target prefix sibling remains capturable");
    assert_ne!(
        clean.identity.untracked_manifest_digest, sibling.identity.untracked_manifest_digest,
        "custom target exclusion must stop at a locator boundary"
    );
    workspace.remove_file("build/cargo-target-sibling/ordinary.txt");

    for locator in [
        ".mobius/state",
        ".tmp/log",
        "target/build",
        "target/debug/deep/generated",
        "build/cargo-target/build",
        "build/cargo-target/debug/deep/generated",
    ] {
        let mut managed_toolchain_spec = spec.clone();
        managed_toolchain_spec.toolchain = vec![ToolchainInput {
            logical_id: "managed-input".to_owned(),
            source: ToolchainSource::File {
                locator: locator.to_owned(),
            },
        }];
        let error = capture_repository_worktree(workspace.root(), &managed_toolchain_spec)
            .expect_err("managed toolchain inputs must fail closed");
        assert!(
            error
                .to_string()
                .contains("names managed or generated state"),
            "unexpected error for {locator:?}: {error}"
        );
    }
}

#[test]
fn repository_capture_keeps_ordinary_nested_target_material() {
    let workspace = GitWorkspace::new();
    workspace.write("src/target/tracked.txt", b"nested-tracked-v1\n");
    workspace.git(&["add", "src/target/tracked.txt"]);
    workspace.git(&["commit", "--quiet", "-m", "add nested target material"]);

    let spec = RepositoryCaptureSpec {
        scope: scope(&["src"], &[]),
        resolved_cargo_target_dirs: Some(vec!["target".to_owned()]),
        toolchain: vec![ToolchainInput {
            logical_id: "nested-target-config".to_owned(),
            source: ToolchainSource::File {
                locator: "src/target/tracked.txt".to_owned(),
            },
        }],
    };
    let clean = capture_repository_worktree(workspace.root(), &spec)
        .expect("ordinary nested target capture succeeds");
    assert_eq!(clean.scope.exclude, [".mobius", ".tmp", "target"]);

    workspace.write("src/target/tracked.txt", b"nested-tracked-v2\n");
    let tracked = capture_repository_worktree(workspace.root(), &spec)
        .expect("nested tracked material remains capturable");
    assert_ne!(
        clean.identity.tracked_delta_digest,
        tracked.identity.tracked_delta_digest
    );
    assert_ne!(
        clean.identity.toolchain_config_digest, tracked.identity.toolchain_config_digest,
        "an ordinary nested target path may be a declared toolchain input"
    );

    workspace.write("src/target/tracked.txt", b"nested-tracked-v1\n");
    assert_eq!(
        clean,
        capture_repository_worktree(workspace.root(), &spec)
            .expect("restored nested target capture succeeds")
    );

    workspace.write("src/target/untracked.txt", b"nested-untracked\n");
    let untracked = capture_repository_worktree(workspace.root(), &spec)
        .expect("nested untracked material remains capturable");
    assert_ne!(
        clean.identity.untracked_manifest_digest,
        untracked.identity.untracked_manifest_digest
    );
}

#[test]
fn repository_capture_fails_closed_for_unresolved_or_invalid_target_directories() {
    let workspace = GitWorkspace::new();
    let mut spec = repository_spec();
    spec.resolved_cargo_target_dirs = None;
    let unresolved = capture_repository_worktree(workspace.root(), &spec)
        .expect_err("unresolved Cargo target directories must fail closed");
    assert!(
        unresolved
            .to_string()
            .contains("Cargo target directories were not resolved"),
        "unexpected unresolved-target error: {unresolved}"
    );

    for locator in [
        "/absolute-target",
        "build/../target",
        "build//target",
        "build\\target",
    ] {
        spec.resolved_cargo_target_dirs = Some(vec![locator.to_owned()]);
        let invalid = capture_repository_worktree(workspace.root(), &spec)
            .expect_err("invalid resolved target locators must fail closed");
        assert!(
            invalid
                .to_string()
                .contains("resolved Cargo target directories"),
            "unexpected invalid-target error for {locator:?}: {invalid}"
        );
    }
}

#[test]
fn repository_before_after_and_current_drift_are_classified() {
    let workspace = GitWorkspace::new();
    let spec = repository_spec();
    let before =
        capture_repository_worktree(workspace.root(), &spec).expect("before capture succeeds");
    let coherent_bundle = bundle(MaterialBaseline::RepositoryWorktree {
        scope: before.scope.clone(),
        before: before.identity.clone(),
        after: before.identity.clone(),
    });
    let current = CurrentMaterial::RepositoryWorktree(before.clone());
    assert_eq!(
        evaluate(&coherent_bundle, Some(&current)).classification,
        Classification::CurrentApplicable
    );

    workspace.write("tracked.txt", b"drifted\n");
    let after =
        capture_repository_worktree(workspace.root(), &spec).expect("after capture succeeds");
    let drifted_bundle = bundle(MaterialBaseline::RepositoryWorktree {
        scope: before.scope.clone(),
        before: before.identity.clone(),
        after: after.identity.clone(),
    });
    assert_eq!(
        evaluate(
            &drifted_bundle,
            Some(&CurrentMaterial::RepositoryWorktree(after.clone()))
        )
        .classification,
        Classification::Incoherent
    );
    assert_eq!(
        evaluate(
            &coherent_bundle,
            Some(&CurrentMaterial::RepositoryWorktree(after))
        )
        .classification,
        Classification::Superseded
    );

    let canonical = canonicalize(&coherent_bundle).expect("captured bundle is canonical");
    let root_text = workspace.root().to_string_lossy();
    assert!(
        !canonical
            .bytes
            .windows(root_text.len())
            .any(|window| window == root_text.as_bytes())
    );
}

#[cfg(unix)]
#[test]
fn untracked_symlink_hashes_target_bytes_without_following() {
    use std::os::unix::fs::symlink;

    let workspace = GitWorkspace::new();
    let spec = RepositoryCaptureSpec {
        scope: scope(&["link"], &[]),
        resolved_cargo_target_dirs: Some(vec!["target".to_owned()]),
        toolchain: Vec::new(),
    };
    symlink("outside-v1", workspace.root().join("link")).expect("first symlink is created");
    let first =
        capture_repository_worktree(workspace.root(), &spec).expect("first capture succeeds");

    workspace.write("outside-v1", b"target contents are not followed");
    let target_changed =
        capture_repository_worktree(workspace.root(), &spec).expect("target capture succeeds");
    assert_eq!(first, target_changed);

    workspace.remove_file("link");
    symlink("outside-v2", workspace.root().join("link")).expect("second symlink is created");
    let link_changed = capture_repository_worktree(workspace.root(), &spec)
        .expect("changed link capture succeeds");
    assert_ne!(
        first.identity.untracked_manifest_digest,
        link_changed.identity.untracked_manifest_digest
    );
}
