use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::path::Path;
use std::process::Command;

pub const EVIDENCE_BUNDLE_SCHEMA: &str = "mobius.evidence-bundle.v1";
pub const CANONICALIZATION: &str = "mobius.canonical-json.v1";
pub const MAX_CANONICAL_BUNDLE_BYTES: usize = 131_072;
const REPOSITORY_ROOT_EXCLUDES: [&str; 2] = [".mobius", ".tmp"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractError {
    message: String,
}

impl ContractError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ContractError {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceBundle {
    pub schema: String,
    pub canonicalization: String,
    pub material_baseline: MaterialBaseline,
    pub verification: Vec<Verification>,
    pub observed_effects: ObservedEffects,
    pub counterevidence: Vec<String>,
    pub limits: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaterialBaseline {
    RepositoryWorktree {
        scope: MaterialScope,
        before: RepositoryWorktreeIdentity,
        after: RepositoryWorktreeIdentity,
    },
    ArtifactSet {
        scope: MaterialScope,
        before: ArtifactSetIdentity,
        after: ArtifactSetIdentity,
    },
    ExternalObjectSet {
        scope: MaterialScope,
        before: ExternalObjectSetIdentity,
        after: ExternalObjectSetIdentity,
    },
    Intrinsic {
        scope: MaterialScope,
        before: EmptyIdentity,
        after: EmptyIdentity,
        observation_identity: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialScope {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryWorktreeIdentity {
    pub base_tree: String,
    pub tracked_delta_digest: String,
    pub untracked_manifest_digest: String,
    pub toolchain_config_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSetIdentity {
    pub entries: Vec<ArtifactIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactIdentity {
    pub logical_id: String,
    pub digest: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalObjectSetIdentity {
    pub entries: Vec<ExternalObjectIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalObjectIdentity {
    pub logical_id: String,
    pub locator: String,
    pub immutable_version: Option<String>,
    pub content_digest: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyIdentity {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verification {
    pub check_id: String,
    pub command_or_method: String,
    pub exit_status: ExitStatus,
    pub output_identity: String,
    pub assessment: Assessment,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExitStatus {
    Code(i64),
    NotApplicable,
}

impl Serialize for ExitStatus {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Code(code) => serializer.serialize_i64(*code),
            Self::NotApplicable => serializer.serialize_str("not_applicable"),
        }
    }
}

impl<'de> Deserialize<'de> for ExitStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Representation {
            Code(i64),
            Marker(String),
        }

        match Representation::deserialize(deserializer)? {
            Representation::Code(code) => Ok(Self::Code(code)),
            Representation::Marker(marker) if marker == "not_applicable" => Ok(Self::NotApplicable),
            Representation::Marker(marker) => Err(de::Error::custom(format!(
                "unsupported exit status marker {marker:?}"
            ))),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Assessment {
    Supports,
    Contradicts,
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedEffects {
    pub changed_surfaces: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Classification {
    Invalid,
    Incoherent,
    CurrentApplicable,
    Superseded,
    Unverifiable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalEvidence {
    pub bundle: EvidenceBundle,
    pub bytes: Vec<u8>,
    pub identity: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evaluation {
    pub classification: Classification,
    pub valid: bool,
    pub coherent: bool,
    pub current_applicable: bool,
    pub canonical: Option<CanonicalEvidence>,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CurrentMaterial {
    RepositoryWorktree(RepositoryCapture),
    ArtifactSet {
        scope: MaterialScope,
        identity: ArtifactSetIdentity,
    },
    ExternalObjectSet {
        scope: MaterialScope,
        identity: ExternalObjectSetIdentity,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryCaptureSpec {
    pub scope: MaterialScope,
    pub resolved_cargo_target_dirs: Option<Vec<String>>,
    pub toolchain: Vec<ToolchainInput>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryCapture {
    pub scope: MaterialScope,
    pub identity: RepositoryWorktreeIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolchainInput {
    pub logical_id: String,
    pub source: ToolchainSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolchainSource {
    File { locator: String },
    Value { bytes: Vec<u8> },
}

pub fn sha256_identity(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut identity = String::with_capacity(7 + digest.len() * 2);
    identity.push_str("sha256:");
    for byte in digest {
        identity.push(char::from(HEX[usize::from(byte >> 4)]));
        identity.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    identity
}

pub fn canonicalize(bundle: &EvidenceBundle) -> Result<CanonicalEvidence, ContractError> {
    let mut normalized = bundle.clone();
    normalize_bundle(&mut normalized)?;
    let bytes = canonical_json_bytes(&normalized)?;
    if bytes.len() > MAX_CANONICAL_BUNDLE_BYTES {
        return Err(ContractError::new(format!(
            "canonical Evidence Bundle exceeds {MAX_CANONICAL_BUNDLE_BYTES} bytes"
        )));
    }
    let identity = sha256_identity(&bytes);
    Ok(CanonicalEvidence {
        bundle: normalized,
        bytes,
        identity,
    })
}

pub fn evaluate_json(input: &[u8], current: Option<&CurrentMaterial>) -> Evaluation {
    match serde_json::from_slice::<EvidenceBundle>(input) {
        Ok(bundle) => evaluate(&bundle, current),
        Err(error) => invalid_evaluation(format!("bundle decoding failed: {error}")),
    }
}

pub fn evaluate(bundle: &EvidenceBundle, current: Option<&CurrentMaterial>) -> Evaluation {
    let canonical = match canonicalize(bundle) {
        Ok(canonical) => canonical,
        Err(error) => return invalid_evaluation(error.to_string()),
    };

    if !baseline_is_coherent(&canonical.bundle.material_baseline) {
        return Evaluation {
            classification: Classification::Incoherent,
            valid: true,
            coherent: false,
            current_applicable: false,
            canonical: Some(canonical),
            errors: Vec::new(),
        };
    }

    if matches!(
        canonical.bundle.material_baseline,
        MaterialBaseline::Intrinsic { .. }
    ) {
        return Evaluation {
            classification: Classification::CurrentApplicable,
            valid: true,
            coherent: true,
            current_applicable: true,
            canonical: Some(canonical),
            errors: Vec::new(),
        };
    }

    let Some(current) = current else {
        return unverifiable_evaluation(canonical, "current material capture was not supplied");
    };

    match current_matches(&canonical.bundle.material_baseline, current) {
        Ok(true) => Evaluation {
            classification: Classification::CurrentApplicable,
            valid: true,
            coherent: true,
            current_applicable: true,
            canonical: Some(canonical),
            errors: Vec::new(),
        },
        Ok(false) => Evaluation {
            classification: Classification::Superseded,
            valid: true,
            coherent: true,
            current_applicable: false,
            canonical: Some(canonical),
            errors: Vec::new(),
        },
        Err(error) => unverifiable_evaluation(canonical, error.to_string()),
    }
}

pub fn capture_repository_worktree(
    root: &Path,
    spec: &RepositoryCaptureSpec,
) -> Result<RepositoryCapture, ContractError> {
    let root = fs::canonicalize(root).map_err(|error| {
        ContractError::new(format!("failed to canonicalize repository root: {error}"))
    })?;
    if !root.is_dir() {
        return Err(ContractError::new(
            "repository root must be a readable directory",
        ));
    }

    require_repository_root(&root)?;

    let managed_excludes = repository_managed_excludes(spec.resolved_cargo_target_dirs.as_deref())?;
    let mut scope = spec.scope.clone();
    scope.exclude.extend(managed_excludes.iter().cloned());
    normalize_repository_scope(&mut scope)?;
    let base_tree = git_line(&root, ["rev-parse", "--verify", "HEAD^{tree}"])?;
    validate_git_object_id(&base_tree, "material_baseline base_tree")?;

    let tracked_paths = tracked_paths(&root, &scope)?;
    let tracked_delta = tracked_diff(&root, &tracked_paths)?;
    let untracked_manifest = untracked_manifest(&root, &scope)?;
    let toolchain_manifest = toolchain_manifest(&root, &spec.toolchain, &managed_excludes)?;

    Ok(RepositoryCapture {
        scope,
        identity: RepositoryWorktreeIdentity {
            base_tree,
            tracked_delta_digest: sha256_identity(&tracked_delta),
            untracked_manifest_digest: sha256_identity(&untracked_manifest),
            toolchain_config_digest: sha256_identity(&toolchain_manifest),
        },
    })
}

fn invalid_evaluation(error: String) -> Evaluation {
    Evaluation {
        classification: Classification::Invalid,
        valid: false,
        coherent: false,
        current_applicable: false,
        canonical: None,
        errors: vec![error],
    }
}

fn unverifiable_evaluation(canonical: CanonicalEvidence, error: impl Into<String>) -> Evaluation {
    Evaluation {
        classification: Classification::Unverifiable,
        valid: true,
        coherent: true,
        current_applicable: false,
        canonical: Some(canonical),
        errors: vec![error.into()],
    }
}

fn normalize_bundle(bundle: &mut EvidenceBundle) -> Result<(), ContractError> {
    if bundle.schema != EVIDENCE_BUNDLE_SCHEMA {
        return Err(ContractError::new(format!(
            "unsupported Evidence Bundle schema {:?}",
            bundle.schema
        )));
    }
    if bundle.canonicalization != CANONICALIZATION {
        return Err(ContractError::new(format!(
            "unsupported canonicalization {:?}",
            bundle.canonicalization
        )));
    }

    normalize_baseline(&mut bundle.material_baseline)?;
    if bundle.verification.is_empty() {
        return Err(ContractError::new("verification must be nonempty"));
    }
    let mut check_ids = BTreeSet::new();
    for verification in &bundle.verification {
        validate_local_id(&verification.check_id, "verification check_id")?;
        if !check_ids.insert(verification.check_id.as_str()) {
            return Err(ContractError::new(format!(
                "duplicate verification check_id {:?}",
                verification.check_id
            )));
        }
        validate_text(
            &verification.command_or_method,
            "verification command_or_method",
        )?;
        validate_sha256(
            &verification.output_identity,
            "verification output_identity",
        )?;
    }

    normalize_locator_set(
        &mut bundle.observed_effects.changed_surfaces,
        "observed_effects changed_surfaces",
    )?;
    normalize_text_set(&mut bundle.counterevidence, "counterevidence")?;
    normalize_text_set(&mut bundle.limits, "limits")?;
    Ok(())
}

fn normalize_baseline(baseline: &mut MaterialBaseline) -> Result<(), ContractError> {
    match baseline {
        MaterialBaseline::RepositoryWorktree {
            scope,
            before,
            after,
        } => {
            normalize_repository_scope(scope)?;
            validate_repository_identity(before, "repository_worktree before")?;
            validate_repository_identity(after, "repository_worktree after")?;
        }
        MaterialBaseline::ArtifactSet {
            scope,
            before,
            after,
        } => {
            normalize_nonempty_scope(scope)?;
            normalize_artifact_set(before, "artifact_set before")?;
            normalize_artifact_set(after, "artifact_set after")?;
        }
        MaterialBaseline::ExternalObjectSet {
            scope,
            before,
            after,
        } => {
            normalize_nonempty_scope(scope)?;
            normalize_external_set(before, "external_object_set before")?;
            normalize_external_set(after, "external_object_set after")?;
        }
        MaterialBaseline::Intrinsic {
            scope,
            observation_identity,
            ..
        } => {
            normalize_scope(scope)?;
            if !scope.include.is_empty() || !scope.exclude.is_empty() {
                return Err(ContractError::new("intrinsic baseline scope must be empty"));
            }
            validate_sha256(observation_identity, "intrinsic observation_identity")?;
        }
    }
    Ok(())
}

fn normalize_repository_scope(scope: &mut MaterialScope) -> Result<(), ContractError> {
    normalize_scope(scope)?;
    if scope.include.is_empty() {
        return Err(ContractError::new(
            "repository_worktree scope include must be nonempty",
        ));
    }
    scope
        .exclude
        .extend(REPOSITORY_ROOT_EXCLUDES.into_iter().map(str::to_owned));
    scope
        .exclude
        .sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    scope.exclude.dedup();
    Ok(())
}

fn repository_managed_excludes(
    resolved_cargo_target_dirs: Option<&[String]>,
) -> Result<Vec<String>, ContractError> {
    let Some(resolved_cargo_target_dirs) = resolved_cargo_target_dirs else {
        return Err(ContractError::new(
            "Cargo target directories were not resolved",
        ));
    };

    let mut excludes = resolved_cargo_target_dirs.to_vec();
    normalize_locator_set(&mut excludes, "resolved Cargo target directories")?;
    excludes.extend(REPOSITORY_ROOT_EXCLUDES.into_iter().map(str::to_owned));
    excludes.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    excludes.dedup();
    Ok(excludes)
}

fn normalize_nonempty_scope(scope: &mut MaterialScope) -> Result<(), ContractError> {
    normalize_scope(scope)?;
    if scope.include.is_empty() {
        return Err(ContractError::new(
            "material scope include must be nonempty",
        ));
    }
    Ok(())
}

fn normalize_scope(scope: &mut MaterialScope) -> Result<(), ContractError> {
    normalize_locator_set(&mut scope.include, "material scope include")?;
    normalize_locator_set(&mut scope.exclude, "material scope exclude")?;
    Ok(())
}

fn normalize_locator_set(values: &mut Vec<String>, field: &str) -> Result<(), ContractError> {
    for value in values.iter() {
        validate_locator(value, field)?;
    }
    values.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    values.dedup();
    Ok(())
}

fn normalize_text_set(values: &mut Vec<String>, field: &str) -> Result<(), ContractError> {
    for value in values.iter() {
        validate_text(value, field)?;
    }
    values.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    values.dedup();
    Ok(())
}

fn normalize_artifact_set(
    identity: &mut ArtifactSetIdentity,
    field: &str,
) -> Result<(), ContractError> {
    if identity.entries.is_empty() {
        return Err(ContractError::new(format!(
            "{field} entries must be nonempty"
        )));
    }
    let mut logical_ids = BTreeSet::new();
    for entry in &identity.entries {
        validate_local_id(&entry.logical_id, &format!("{field} logical_id"))?;
        if !logical_ids.insert(entry.logical_id.as_str()) {
            return Err(ContractError::new(format!(
                "{field} contains duplicate logical_id {:?}",
                entry.logical_id
            )));
        }
        validate_sha256(&entry.digest, &format!("{field} digest"))?;
    }
    identity
        .entries
        .sort_by(|left, right| left.logical_id.as_bytes().cmp(right.logical_id.as_bytes()));
    Ok(())
}

fn normalize_external_set(
    identity: &mut ExternalObjectSetIdentity,
    field: &str,
) -> Result<(), ContractError> {
    if identity.entries.is_empty() {
        return Err(ContractError::new(format!(
            "{field} entries must be nonempty"
        )));
    }
    let mut logical_ids = BTreeSet::new();
    for entry in &identity.entries {
        validate_local_id(&entry.logical_id, &format!("{field} logical_id"))?;
        if !logical_ids.insert(entry.logical_id.as_str()) {
            return Err(ContractError::new(format!(
                "{field} contains duplicate logical_id {:?}",
                entry.logical_id
            )));
        }
        validate_external_locator(&entry.locator, &format!("{field} locator"))?;
        if let Some(version) = &entry.immutable_version {
            validate_immutable_version(version, &format!("{field} immutable_version"))?;
        }
        if let Some(digest) = &entry.content_digest {
            validate_sha256(digest, &format!("{field} content_digest"))?;
        }
        if entry.immutable_version.is_none() && entry.content_digest.is_none() {
            return Err(ContractError::new(format!(
                "{field} entry {:?} has only a mutable locator",
                entry.logical_id
            )));
        }
    }
    identity
        .entries
        .sort_by(|left, right| left.logical_id.as_bytes().cmp(right.logical_id.as_bytes()));
    Ok(())
}

fn validate_repository_identity(
    identity: &RepositoryWorktreeIdentity,
    field: &str,
) -> Result<(), ContractError> {
    validate_git_object_id(&identity.base_tree, &format!("{field} base_tree"))?;
    validate_sha256(
        &identity.tracked_delta_digest,
        &format!("{field} tracked_delta_digest"),
    )?;
    validate_sha256(
        &identity.untracked_manifest_digest,
        &format!("{field} untracked_manifest_digest"),
    )?;
    validate_sha256(
        &identity.toolchain_config_digest,
        &format!("{field} toolchain_config_digest"),
    )?;
    Ok(())
}

fn validate_sha256(value: &str, field: &str) -> Result<(), ContractError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(ContractError::new(format!(
            "{field} must use a sha256 identity"
        )));
    };
    if hex.len() != 64
        || !hex
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
        return Err(ContractError::new(format!(
            "{field} must contain 64 lowercase hexadecimal characters"
        )));
    }
    Ok(())
}

fn validate_git_object_id(value: &str, field: &str) -> Result<(), ContractError> {
    if !matches!(value.len(), 40 | 64)
        || !value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
        return Err(ContractError::new(format!(
            "{field} must be a complete lowercase Git object id"
        )));
    }
    Ok(())
}

fn validate_locator(value: &str, field: &str) -> Result<(), ContractError> {
    if value.is_empty() {
        return Err(ContractError::new(format!(
            "{field} contains an empty locator"
        )));
    }
    if value.starts_with('/')
        || value.starts_with('\\')
        || looks_like_windows_absolute(value)
        || value.contains('\\')
    {
        return Err(ContractError::new(format!(
            "{field} locator {value:?} must be project-relative with '/' separators"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(ContractError::new(format!(
            "{field} locator {value:?} contains control characters"
        )));
    }
    for segment in value.split('/') {
        if segment.is_empty() || matches!(segment, "." | ".." | "~") {
            return Err(ContractError::new(format!(
                "{field} locator {value:?} contains an unsafe path segment"
            )));
        }
    }
    Ok(())
}

fn looks_like_windows_absolute(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

fn validate_local_id(value: &str, field: &str) -> Result<(), ContractError> {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return Err(ContractError::new(format!("{field} must be nonempty")));
    };
    if !first.is_ascii_alphanumeric()
        || !bytes
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(ContractError::new(format!(
            "{field} must be a stable ASCII local id"
        )));
    }
    Ok(())
}

fn validate_text(value: &str, field: &str) -> Result<(), ContractError> {
    if value.is_empty() || value.trim() != value {
        return Err(ContractError::new(format!(
            "{field} must be nonempty without surrounding whitespace"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(ContractError::new(format!(
            "{field} must not contain control characters"
        )));
    }
    if contains_personal_path(value) {
        return Err(ContractError::new(format!(
            "{field} must not contain an absolute personal path"
        )));
    }
    Ok(())
}

fn validate_external_locator(value: &str, field: &str) -> Result<(), ContractError> {
    validate_text(value, field)
}

fn validate_immutable_version(value: &str, field: &str) -> Result<(), ContractError> {
    validate_text(value, field)?;
    let lowercase = value.to_ascii_lowercase();
    if matches!(
        lowercase.as_str(),
        "latest" | "current" | "head" | "main" | "master" | "tip" | "unstable"
    ) {
        return Err(ContractError::new(format!(
            "{field} names a mutable version"
        )));
    }
    Ok(())
}

fn contains_personal_path(value: &str) -> bool {
    value.contains("/home/")
        || value.contains("/Users/")
        || value.contains("\\Users\\")
        || value.contains(":\\Users\\")
}

fn baseline_is_coherent(baseline: &MaterialBaseline) -> bool {
    match baseline {
        MaterialBaseline::RepositoryWorktree { before, after, .. } => before == after,
        MaterialBaseline::ArtifactSet { before, after, .. } => before == after,
        MaterialBaseline::ExternalObjectSet { before, after, .. } => before == after,
        MaterialBaseline::Intrinsic { .. } => true,
    }
}

fn current_matches(
    baseline: &MaterialBaseline,
    current: &CurrentMaterial,
) -> Result<bool, ContractError> {
    match (baseline, current) {
        (
            MaterialBaseline::RepositoryWorktree { scope, after, .. },
            CurrentMaterial::RepositoryWorktree(capture),
        ) => {
            let mut current = capture.clone();
            normalize_repository_scope(&mut current.scope)?;
            validate_repository_identity(&current.identity, "current repository_worktree")?;
            if scope != &current.scope {
                return Err(ContractError::new(
                    "current repository_worktree capture used a different scope",
                ));
            }
            Ok(after == &current.identity)
        }
        (
            MaterialBaseline::ArtifactSet { scope, after, .. },
            CurrentMaterial::ArtifactSet {
                scope: current_scope,
                identity,
            },
        ) => {
            let mut current_scope = current_scope.clone();
            let mut identity = identity.clone();
            normalize_nonempty_scope(&mut current_scope)?;
            normalize_artifact_set(&mut identity, "current artifact_set")?;
            if scope != &current_scope {
                return Err(ContractError::new(
                    "current artifact_set capture used a different scope",
                ));
            }
            Ok(after == &identity)
        }
        (
            MaterialBaseline::ExternalObjectSet { scope, after, .. },
            CurrentMaterial::ExternalObjectSet {
                scope: current_scope,
                identity,
            },
        ) => {
            let mut current_scope = current_scope.clone();
            let mut identity = identity.clone();
            normalize_nonempty_scope(&mut current_scope)?;
            normalize_external_set(&mut identity, "current external_object_set")?;
            if scope != &current_scope {
                return Err(ContractError::new(
                    "current external_object_set capture used a different scope",
                ));
            }
            Ok(after == &identity)
        }
        (MaterialBaseline::Intrinsic { .. }, _) => Ok(true),
        _ => Err(ContractError::new(
            "current material kind does not match the bundle baseline kind",
        )),
    }
}

fn canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, ContractError> {
    let value = serde_json::to_value(value)
        .map_err(|error| ContractError::new(format!("canonical serialization failed: {error}")))?;
    serde_json::to_vec(&sort_json_value(value))
        .map_err(|error| ContractError::new(format!("canonical encoding failed: {error}")))
}

fn sort_json_value(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(sort_json_value).collect()),
        Value::Object(values) => {
            let mut entries = values.into_iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.as_bytes().cmp(right.as_bytes()));
            let mut sorted = Map::new();
            for (key, value) in entries {
                sorted.insert(key, sort_json_value(value));
            }
            Value::Object(sorted)
        }
        primitive => primitive,
    }
}

fn require_repository_root(root: &Path) -> Result<(), ContractError> {
    let top_level = git_line(root, ["rev-parse", "--show-toplevel"])?;
    let top_level = fs::canonicalize(&top_level).map_err(|error| {
        ContractError::new(format!(
            "failed to canonicalize Git top-level path: {error}"
        ))
    })?;
    if top_level != root {
        return Err(ContractError::new(
            "capture root must be the canonical Git project root",
        ));
    }
    Ok(())
}

fn tracked_paths(root: &Path, scope: &MaterialScope) -> Result<Vec<String>, ContractError> {
    let mut paths = BTreeSet::new();
    for (subcommand, fixed) in [
        ("ls-tree", ["-r", "-z", "--name-only", "HEAD"].as_slice()),
        ("ls-files", ["--cached", "-z"].as_slice()),
    ] {
        let mut arguments = vec![
            OsString::from("--literal-pathspecs"),
            OsString::from(subcommand),
        ];
        arguments.extend(fixed.iter().map(OsString::from));
        arguments.push(OsString::from("--"));
        arguments.extend(scope.include.iter().map(OsString::from));
        let output = run_git(root, arguments)?;
        for path in parse_nul_paths(&output, subcommand)? {
            if path_in_scope(&path, scope) {
                paths.insert(path);
            }
        }
    }
    Ok(paths.into_iter().collect())
}

fn tracked_diff(root: &Path, paths: &[String]) -> Result<Vec<u8>, ContractError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let mut arguments = [
        "--literal-pathspecs",
        "-c",
        "diff.algorithm=myers",
        "-c",
        "diff.indentHeuristic=false",
        "-c",
        "diff.suppressBlankEmpty=false",
        "diff",
        "--binary",
        "--full-index",
        "--no-renames",
        "--no-ext-diff",
        "--no-color",
        "--no-textconv",
        "--unified=3",
        "--no-relative",
        "--ita-visible-in-index",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        "--submodule=short",
        "--ignore-submodules=none",
        "HEAD",
        "--",
    ]
    .into_iter()
    .map(OsString::from)
    .collect::<Vec<_>>();
    arguments.extend(paths.iter().map(OsString::from));
    run_git(root, arguments)
}

#[derive(Serialize)]
struct FileManifestEntry {
    locator: String,
    kind: &'static str,
    size: u64,
    digest: String,
}

fn untracked_manifest(root: &Path, scope: &MaterialScope) -> Result<Vec<u8>, ContractError> {
    let mut arguments = [
        "--literal-pathspecs",
        "ls-files",
        "--others",
        "--exclude-standard",
        "-z",
        "--",
    ]
    .into_iter()
    .map(OsString::from)
    .collect::<Vec<_>>();
    arguments.extend(scope.include.iter().map(OsString::from));
    let output = run_git(root, arguments)?;
    let mut paths = parse_nul_paths(&output, "ls-files --others")?
        .into_iter()
        .filter(|path| path_in_scope(path, scope))
        .collect::<Vec<_>>();
    paths.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    paths.dedup();

    let mut entries = Vec::with_capacity(paths.len());
    for locator in paths {
        let material = read_file_material(root, &locator)?;
        entries.push(FileManifestEntry {
            locator,
            kind: material.kind,
            size: material.size,
            digest: material.digest,
        });
    }
    canonical_json_bytes(&entries)
}

struct FileMaterial {
    kind: &'static str,
    size: u64,
    digest: String,
}

fn read_file_material(root: &Path, locator: &str) -> Result<FileMaterial, ContractError> {
    validate_locator(locator, "captured file")?;
    let path = root.join(locator);
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        ContractError::new(format!(
            "failed to inspect captured file {locator:?}: {error}"
        ))
    })?;

    let (kind, bytes) = if metadata.file_type().is_symlink() {
        let parent = path
            .parent()
            .ok_or_else(|| ContractError::new("captured symlink has no parent"))?;
        require_contained(root, parent, locator)?;
        let target = fs::read_link(&path).map_err(|error| {
            ContractError::new(format!(
                "failed to read captured symlink {locator:?}: {error}"
            ))
        })?;
        ("symlink", target.as_os_str().as_encoded_bytes().to_vec())
    } else if metadata.is_file() {
        require_contained(root, &path, locator)?;
        let bytes = fs::read(&path).map_err(|error| {
            ContractError::new(format!("failed to read captured file {locator:?}: {error}"))
        })?;
        ("regular", bytes)
    } else {
        return Err(ContractError::new(format!(
            "captured path {locator:?} is neither a regular file nor a symlink"
        )));
    };

    let size = u64::try_from(bytes.len())
        .map_err(|_| ContractError::new("captured file size does not fit u64"))?;
    Ok(FileMaterial {
        kind,
        size,
        digest: sha256_identity(&bytes),
    })
}

fn require_contained(root: &Path, path: &Path, locator: &str) -> Result<(), ContractError> {
    let canonical = fs::canonicalize(path).map_err(|error| {
        ContractError::new(format!(
            "failed to resolve captured path {locator:?}: {error}"
        ))
    })?;
    if !canonical.starts_with(root) {
        return Err(ContractError::new(format!(
            "captured path {locator:?} escapes the repository root"
        )));
    }
    Ok(())
}

#[derive(Serialize)]
struct ToolchainManifestEntry {
    logical_id: String,
    kind: &'static str,
    locator: Option<String>,
    size: u64,
    digest: String,
}

fn toolchain_manifest(
    root: &Path,
    inputs: &[ToolchainInput],
    managed_excludes: &[String],
) -> Result<Vec<u8>, ContractError> {
    let mut inputs = inputs.to_vec();
    let mut logical_ids = BTreeSet::new();
    for input in &inputs {
        validate_local_id(&input.logical_id, "toolchain logical_id")?;
        if !logical_ids.insert(input.logical_id.as_str()) {
            return Err(ContractError::new(format!(
                "duplicate toolchain logical_id {:?}",
                input.logical_id
            )));
        }
    }
    inputs.sort_by(|left, right| left.logical_id.as_bytes().cmp(right.logical_id.as_bytes()));

    let mut entries = Vec::with_capacity(inputs.len());
    for input in inputs {
        let (kind, locator, size, digest) = match input.source {
            ToolchainSource::File { locator } => {
                validate_locator(&locator, "toolchain file locator")?;
                if managed_excludes
                    .iter()
                    .any(|exclude| locator_covers(exclude, &locator))
                {
                    return Err(ContractError::new(format!(
                        "toolchain file locator {locator:?} names managed or generated state"
                    )));
                }
                let material = read_file_material(root, &locator)?;
                (material.kind, Some(locator), material.size, material.digest)
            }
            ToolchainSource::Value { bytes } => {
                let size = u64::try_from(bytes.len())
                    .map_err(|_| ContractError::new("toolchain value size does not fit u64"))?;
                ("value", None, size, sha256_identity(&bytes))
            }
        };
        entries.push(ToolchainManifestEntry {
            logical_id: input.logical_id,
            kind,
            locator,
            size,
            digest,
        });
    }
    canonical_json_bytes(&entries)
}

fn path_in_scope(path: &str, scope: &MaterialScope) -> bool {
    scope
        .include
        .iter()
        .any(|include| locator_covers(include, path))
        && !scope
            .exclude
            .iter()
            .any(|exclude| locator_covers(exclude, path))
}

fn locator_covers(locator: &str, path: &str) -> bool {
    path == locator
        || path
            .strip_prefix(locator)
            .is_some_and(|remainder| remainder.starts_with('/'))
}

fn parse_nul_paths(bytes: &[u8], command: &str) -> Result<Vec<String>, ContractError> {
    let mut paths = Vec::new();
    for raw in bytes.split(|byte| *byte == 0).filter(|raw| !raw.is_empty()) {
        let path = std::str::from_utf8(raw)
            .map_err(|_| ContractError::new(format!("{command} returned a non-UTF-8 path")))?;
        validate_locator(path, command)?;
        paths.push(path.to_owned());
    }
    Ok(paths)
}

fn git_line<I, S>(root: &Path, arguments: I) -> Result<String, ContractError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = run_git(root, arguments)?;
    let line = std::str::from_utf8(&output)
        .map_err(|_| ContractError::new("Git returned non-UTF-8 identity output"))?
        .trim_end_matches(['\r', '\n']);
    if line.is_empty() || line.contains(['\r', '\n']) {
        return Err(ContractError::new(
            "Git identity output must contain exactly one nonempty line",
        ));
    }
    Ok(line.to_owned())
}

fn run_git<I, S>(root: &Path, arguments: I) -> Result<Vec<u8>, ContractError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new("git");
    command.arg("-C").arg(root).args(arguments);
    for variable in [
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CEILING_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
        "GIT_DIFF_OPTS",
        "GIT_DIR",
        "GIT_EXTERNAL_DIFF",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_WORK_TREE",
    ] {
        command.env_remove(variable);
    }
    let output = command
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_PAGER", "cat")
        .output()
        .map_err(|error| ContractError::new(format!("failed to execute Git: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ContractError::new(format!(
            "Git command failed with {}: {}",
            output.status,
            stderr.trim()
        )));
    }
    Ok(output.stdout)
}
