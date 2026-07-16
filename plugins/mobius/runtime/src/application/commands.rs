//! Caller-facing mutation commands.
//!
//! This surface is intentionally distinct from `domain::TransitionInput`. In particular, callers
//! cannot construct a ReviewPacket for Seal; the application service materializes it from the
//! locked current prestate and then creates the model-level transition input.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::{
    AbandonInput, ActivateObjectiveInput, AddRouteInput, AttemptId, CheckWaitInput, DecisionInput,
    HeadBinding, InstallMapInput, ProjectId, RecordEvidenceInput, RequestRemapInput,
    ReviseObjectiveInput, SealReason, SelectRouteInput, StartAttemptInput, TransitionInput,
    TransitionKind,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SealAttemptCommand {
    pub attempt: AttemptId,
    pub seal_reason: SealReason,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum MutationCommand {
    ActivateObjective(ActivateObjectiveInput),
    InstallMap(InstallMapInput),
    AddRoute(AddRouteInput),
    SelectRoute(SelectRouteInput),
    StartAttempt(StartAttemptInput),
    RecordEvidence(RecordEvidenceInput),
    SealAttempt(SealAttemptCommand),
    Decision(DecisionInput),
    CheckWait(CheckWaitInput),
    RequestRemap(RequestRemapInput),
    ReviseObjective(ReviseObjectiveInput),
    Abandon(AbandonInput),
}

impl MutationCommand {
    pub const fn kind(&self) -> TransitionKind {
        match self {
            Self::ActivateObjective(_) => TransitionKind::ActivateObjective,
            Self::InstallMap(_) => TransitionKind::InstallMap,
            Self::AddRoute(_) => TransitionKind::AddRoute,
            Self::SelectRoute(_) => TransitionKind::SelectRoute,
            Self::StartAttempt(_) => TransitionKind::StartAttempt,
            Self::RecordEvidence(_) => TransitionKind::RecordEvidence,
            Self::SealAttempt(_) => TransitionKind::SealAttempt,
            Self::Decision(_) => TransitionKind::Decision,
            Self::CheckWait(_) => TransitionKind::CheckWait,
            Self::RequestRemap(_) => TransitionKind::RequestRemap,
            Self::ReviseObjective(_) => TransitionKind::ReviseObjective,
            Self::Abandon(_) => TransitionKind::Abandon,
        }
    }

    /// Convert commands whose transition input is already complete. Seal returns its restricted
    /// command so the service must take the unique Packet materialization path.
    pub fn into_direct_transition(self) -> Result<TransitionInput, SealAttemptCommand> {
        match self {
            Self::ActivateObjective(input) => Ok(TransitionInput::ActivateObjective(input)),
            Self::InstallMap(input) => Ok(TransitionInput::InstallMap(input)),
            Self::AddRoute(input) => Ok(TransitionInput::AddRoute(input)),
            Self::SelectRoute(input) => Ok(TransitionInput::SelectRoute(input)),
            Self::StartAttempt(input) => Ok(TransitionInput::StartAttempt(input)),
            Self::RecordEvidence(input) => Ok(TransitionInput::RecordEvidence(input)),
            Self::SealAttempt(command) => Err(command),
            Self::Decision(input) => Ok(TransitionInput::Decision(input)),
            Self::CheckWait(input) => Ok(TransitionInput::CheckWait(input)),
            Self::RequestRemap(input) => Ok(TransitionInput::RequestRemap(input)),
            Self::ReviseObjective(input) => Ok(TransitionInput::ReviseObjective(input)),
            Self::Abandon(input) => Ok(TransitionInput::Abandon(input)),
        }
    }
}

/// The exact required fields of each externally tagged command payload. The MCP schema and
/// `next_actions` read model share this table so guidance cannot drift from the accepted wire.
pub(crate) const fn required_input_fields(kind: TransitionKind) -> &'static [&'static str] {
    match kind {
        TransitionKind::ActivateObjective | TransitionKind::ReviseObjective => {
            &["objective_spec", "confirmation"]
        }
        TransitionKind::InstallMap => &["map", "initial_routes", "cover", "carry"],
        TransitionKind::AddRoute | TransitionKind::SelectRoute => &["route"],
        TransitionKind::StartAttempt => &["attempt"],
        TransitionKind::RecordEvidence => &["evidence"],
        TransitionKind::SealAttempt => &["attempt", "seal_reason"],
        TransitionKind::Decision => &["decision"],
        TransitionKind::CheckWait => &["wait_condition", "evidence", "judgment"],
        TransitionKind::RequestRemap => &["reason"],
        TransitionKind::Abandon => &["reason", "confirmation"],
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ApplyTransitionRequest {
    /// Untrusted host input. Admission resolves and verifies this path before any state access.
    pub project_root: PathBuf,
    pub project_id: ProjectId,
    pub expected_heads: HeadBinding,
    /// Project-scoped idempotency key. Empty values are rejected by live admission.
    pub request_id: String,
    pub command: MutationCommand,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::encode_canonical;

    #[test]
    fn caller_seal_command_cannot_contain_a_packet_or_evidence_selection() {
        let command = MutationCommand::SealAttempt(SealAttemptCommand {
            attempt: AttemptId::new("attempt-1"),
            seal_reason: SealReason::Submitted,
        });

        assert_eq!(command.kind(), TransitionKind::SealAttempt);
        let restricted = command
            .into_direct_transition()
            .expect_err("Seal must require Core materialization");
        assert_eq!(restricted.attempt, AttemptId::new("attempt-1"));
        assert_eq!(restricted.seal_reason, SealReason::Submitted);
    }

    #[test]
    fn seal_command_has_one_closed_external_tagged_wire_shape() {
        let command = MutationCommand::SealAttempt(SealAttemptCommand {
            attempt: AttemptId::new("attempt-1"),
            seal_reason: SealReason::Submitted,
        });
        let bytes = encode_canonical(&command).expect("closed command must serialize");
        assert_eq!(
            String::from_utf8(bytes).expect("JSON must be UTF-8"),
            r#"{"seal_attempt":{"attempt":"attempt-1","seal_reason":"submitted"}}"#
        );

        for forbidden in ["packet", "trail_prefix", "evidence_selection"] {
            let json = format!(
                r#"{{"seal_attempt":{{"attempt":"attempt-1","seal_reason":"submitted","{forbidden}":[]}}}}"#
            );
            assert!(
                serde_json::from_str::<MutationCommand>(&json).is_err(),
                "caller field {forbidden} must be rejected"
            );
        }
    }

    #[test]
    fn apply_request_is_closed_and_canonically_hashable() {
        let request = ApplyTransitionRequest {
            project_root: PathBuf::from("project"),
            project_id: ProjectId::new("project-1"),
            expected_heads: HeadBinding {
                expected_project_seq: 7,
                expected_objective_seq: 3,
            },
            request_id: "request-1".into(),
            command: MutationCommand::SealAttempt(SealAttemptCommand {
                attempt: AttemptId::new("attempt-1"),
                seal_reason: SealReason::BoundReached,
            }),
        };
        let bytes = encode_canonical(&request).expect("request must serialize");
        assert_eq!(
            String::from_utf8(bytes.clone()).expect("JSON must be UTF-8"),
            r#"{"project_root":"project","project_id":"project-1","expected_heads":{"expected_project_seq":7,"expected_objective_seq":3},"request_id":"request-1","command":{"seal_attempt":{"attempt":"attempt-1","seal_reason":"bound_reached"}}}"#
        );
        assert_eq!(
            serde_json::from_slice::<ApplyTransitionRequest>(&bytes).expect("request must decode"),
            request
        );

        let unknown = String::from_utf8(bytes).unwrap().replacen(
            r#"{"project_root":"project"#,
            r#"{"unknown":true,"project_root":"project"#,
            1,
        );
        assert!(serde_json::from_str::<ApplyTransitionRequest>(&unknown).is_err());
    }
}
