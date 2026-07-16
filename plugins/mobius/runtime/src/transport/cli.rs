use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::PathBuf;

use serde::Serialize;
use serde_json::json;

use crate::application::service::{
    AuditRequest, CoreService, ProjectBinding, ReadQuery, ReadRequest, ServiceError,
};
use crate::domain::{ObjectiveId, ProjectId};
use crate::error::MobiusError;
use crate::presentation::report::{CurrentReportState, ReportRenderer, ReportScope};

pub(crate) fn run(mode: &str, arguments: &[OsString]) -> Result<(), MobiusError> {
    let project_root = std::env::current_dir().map_err(|error| {
        MobiusError::operation(
            "project_root_unavailable",
            format!("cannot resolve current project root: {error}"),
        )
    })?;
    let service = CoreService::new(vec![project_root.clone()]);
    match mode {
        "read" => run_read(&service, &project_root, arguments),
        "audit" => run_audit(&service, &project_root, arguments),
        "doctor" => run_doctor(&service, project_root, arguments),
        "report" => run_report(&service, &project_root, arguments),
        _ => Err(MobiusError::invalid_invocation("unknown CLI adapter")),
    }
}

fn run_read(
    service: &CoreService,
    project_root: &std::path::Path,
    arguments: &[OsString],
) -> Result<(), MobiusError> {
    if !(1..=2).contains(&arguments.len()) {
        return Err(MobiusError::invalid_invocation(
            "usage: mobius read <project-id> [<query-json>|-]",
        ));
    }
    let project_id = ProjectId::new(argument(arguments, 0, "project id")?);
    let query_text = match arguments.get(1).and_then(|value| value.to_str()) {
        Some("-") | None => read_stdin()?,
        Some(value) => value.to_owned(),
    };
    let query = serde_json::from_str::<ReadQuery>(&query_text).map_err(|error| {
        MobiusError::invalid_invocation(format!("invalid typed read query JSON: {error}"))
    })?;
    let response = service
        .read(ReadRequest {
            binding: binding(project_root, project_id),
            query,
        })
        .map_err(service_error)?;
    write_json(&response)
}

fn run_audit(
    service: &CoreService,
    project_root: &std::path::Path,
    arguments: &[OsString],
) -> Result<(), MobiusError> {
    if arguments.len() != 1 {
        return Err(MobiusError::invalid_invocation(
            "usage: mobius audit <project-id>",
        ));
    }
    let response = service
        .audit(AuditRequest {
            binding: binding(
                project_root,
                ProjectId::new(argument(arguments, 0, "project id")?),
            ),
            maintenance: None,
        })
        .map_err(service_error)?;
    write_json(&response)
}

fn run_doctor(
    service: &CoreService,
    project_root: PathBuf,
    arguments: &[OsString],
) -> Result<(), MobiusError> {
    if !arguments.is_empty() {
        return Err(MobiusError::invalid_invocation("usage: mobius doctor"));
    }
    let response = service.doctor(project_root).map_err(service_error)?;
    write_json(&response)
}

fn run_report(
    service: &CoreService,
    project_root: &std::path::Path,
    arguments: &[OsString],
) -> Result<(), MobiusError> {
    if arguments.len() != 4 {
        return Err(MobiusError::invalid_invocation(
            "usage: mobius report <project-id> <objective-id> <session-ref> <slug>",
        ));
    }
    let project_id = ProjectId::new(argument(arguments, 0, "project id")?);
    let objective_id = ObjectiveId::new(argument(arguments, 1, "objective id")?);
    let session_ref = argument(arguments, 2, "session ref")?;
    let slug = argument(arguments, 3, "slug")?;
    let binding = binding(project_root, project_id);
    let snapshot = service
        .report_snapshot(&binding, &objective_id)
        .map_err(service_error)?;
    let renderer = ReportRenderer::initialize(project_root)
        .map_err(|error| MobiusError::operation("report_failed", error.to_string()))?;
    let publication = renderer
        .refresh(&ReportScope { session_ref, slug }, &snapshot)
        .map_err(|error| MobiusError::operation("report_failed", error.to_string()))?;
    write_json(&json!({
        "schema": "mobius.report-publication.v1",
        "generation_path": publication.generation_path,
        "current_path": publication.current_path,
        "source_heads": {
            "project_seq": publication.source_heads.project_seq,
            "objective_seq": publication.source_heads.objective_seq
        },
        "previous_state": report_state_name(&publication.previous_state)
    }))
}

fn report_state_name(state: &CurrentReportState) -> &'static str {
    match state {
        CurrentReportState::Missing => "missing",
        CurrentReportState::Fresh { .. } => "fresh",
        CurrentReportState::Stale { .. } => "stale",
        CurrentReportState::Incomplete { .. } => "incomplete",
        CurrentReportState::Invalid { .. } => "invalid",
    }
}

fn binding(project_root: &std::path::Path, project_id: ProjectId) -> ProjectBinding {
    ProjectBinding {
        project_root: project_root.to_path_buf(),
        project_id,
    }
}

fn argument(
    arguments: &[OsString],
    index: usize,
    name: &'static str,
) -> Result<String, MobiusError> {
    arguments
        .get(index)
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| MobiusError::invalid_invocation(format!("{name} must be non-empty UTF-8")))
}

fn read_stdin() -> Result<String, MobiusError> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|error| MobiusError::operation("input_failed", error.to_string()))?;
    Ok(input)
}

fn write_json(value: &impl Serialize) -> Result<(), MobiusError> {
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    serde_json::to_writer(&mut writer, value)
        .map_err(|error| MobiusError::internal(format!("encode CLI output: {error}")))?;
    writer
        .write_all(b"\n")
        .map_err(|error| MobiusError::operation("output_failed", error.to_string()))
}

fn service_error(error: ServiceError) -> MobiusError {
    MobiusError::operation(error.code, error.message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_state_names_are_presentation_only() {
        assert_eq!(report_state_name(&CurrentReportState::Missing), "missing");
        assert_eq!(
            report_state_name(&CurrentReportState::Incomplete {
                reason: "partial generation".to_owned(),
            }),
            "incomplete"
        );
    }
}
