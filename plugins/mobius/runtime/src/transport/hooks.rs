use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::application::admission::admit_project_root;
use crate::application::service::{
    CoreService, ProjectBinding as ServiceProjectBinding, ReadQuery, ReadRequest, ReadResult,
};
use crate::domain::{ObjectiveId, ObjectiveState};
use crate::error::MobiusError;
use crate::infrastructure::sqlite::SqliteStore;

const COMPLETION_MARKER: &str = "MOBIUS_OBJECTIVE_ACHIEVED: ";
const STATE_BOUNDARY_REASON: &str =
    "Mobius Core-owned state must be changed through the Mobius MCP service";

pub(crate) fn run(arguments: &[OsString]) -> Result<(), MobiusError> {
    let Some(handler) = arguments.first().and_then(|value| value.to_str()) else {
        return Err(MobiusError::invalid_invocation(
            "hook mode requires `pre-tool-use` or `stop`",
        ));
    };
    if arguments.len() != 1 {
        return Err(MobiusError::invalid_invocation(
            "hook mode accepts exactly one handler",
        ));
    }
    if !matches!(handler, "pre-tool-use" | "stop") {
        return Err(MobiusError::invalid_invocation(format!(
            "unknown hook handler `{handler}`"
        )));
    }

    let input = read_stdin()?;
    let output = if handler == "pre-tool-use" {
        let input = serde_json::from_str::<PreToolUseInput>(&input)
            .map_err(|error| invalid_hook_input("PreToolUse", error))?;
        pre_tool_use_output(&input)
    } else {
        let input = serde_json::from_str::<StopInput>(&input)
            .map_err(|error| invalid_hook_input("Stop", error))?;
        stop_output(&input)
    };

    if let Some(output) = output {
        write_output(&output)?;
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct PreToolUseInput {
    #[serde(default)]
    cwd: Option<PathBuf>,
    tool_name: String,
    #[serde(default)]
    tool_input: Value,
}

#[derive(Debug, Deserialize)]
struct StopInput {
    cwd: PathBuf,
    #[serde(default)]
    last_assistant_message: Option<String>,
    #[serde(default)]
    stop_hook_active: bool,
}

fn read_stdin() -> Result<String, MobiusError> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|error| MobiusError::operation("hook_input_failed", error.to_string()))?;
    Ok(input)
}

fn write_output(output: &Value) -> Result<(), MobiusError> {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    serde_json::to_writer(&mut lock, output)
        .map_err(|error| MobiusError::internal(format!("encode hook output: {error}")))?;
    lock.write_all(b"\n")
        .map_err(|error| MobiusError::operation("hook_output_failed", error.to_string()))
}

fn invalid_hook_input(event: &str, error: serde_json::Error) -> MobiusError {
    MobiusError::operation(
        "invalid_hook_input",
        format!("invalid {event} hook input: {error}"),
    )
}

fn pre_tool_use_output(input: &PreToolUseInput) -> Option<Value> {
    let targets = mutation_targets(input);
    let bound_project_roots = bound_hook_project_roots(input, &targets);
    pre_tool_use_decision(input, &targets, &bound_project_roots)
}

fn pre_tool_use_decision(
    input: &PreToolUseInput,
    targets: &[String],
    bound_project_roots: &[PathBuf],
) -> Option<Value> {
    let targets_managed_state = targets_core_state(targets, None)
        || bound_project_roots
            .iter()
            .any(|root| targets_core_state(targets, Some(root)));
    if !targets_managed_state && !operation_destroys_core_descendants(input, bound_project_roots) {
        return None;
    }

    Some(json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": STATE_BOUNDARY_REASON
        }
    }))
}

fn mutation_targets(input: &PreToolUseInput) -> Vec<String> {
    match input.tool_name.as_str() {
        "apply_patch" => apply_patch_targets(&input.tool_input, input.cwd.as_deref()),
        "Edit" | "Write" => structured_file_targets(
            &input.tool_input,
            StructuredMutation::Write,
            input.cwd.as_deref(),
        ),
        "Bash" | "Shell" | "exec_command" => {
            shell_mutation_targets(&input.tool_input, input.cwd.as_deref())
        }
        "write_stdin" => write_stdin_mutation_targets(&input.tool_input, input.cwd.as_deref()),
        name if name.starts_with("mcp__") => {
            mcp_mutation_targets(name, &input.tool_input, input.cwd.as_deref())
        }
        _ => Vec::new(),
    }
}

fn apply_patch_targets(value: &Value, cwd: Option<&Path>) -> Vec<String> {
    let patch = match value {
        Value::Object(values) => values.get("command").and_then(Value::as_str),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) => None,
        Value::String(_) => None,
    };
    let Some(patch) = patch else {
        return Vec::new();
    };

    patch
        .lines()
        .filter_map(|line| {
            [
                "*** Add File: ",
                "*** Update File: ",
                "*** Delete File: ",
                "*** Move to: ",
                "*** Move to File: ",
            ]
            .iter()
            .find_map(|prefix| line.strip_prefix(prefix))
        })
        .map(|target| resolved_target_string(target, cwd))
        .collect()
}

#[derive(Clone, Copy)]
enum StructuredMutation {
    Write,
    Copy,
    Move,
}

fn mcp_mutation_targets(tool_name: &str, value: &Value, cwd: Option<&Path>) -> Vec<String> {
    let name = tool_name.to_ascii_lowercase();
    if ![
        "write", "edit", "delete", "remove", "rename", "move", "copy", "create", "replace",
        "append", "patch", "upload", "execute",
    ]
    .iter()
    .any(|signal| name.contains(signal))
    {
        return Vec::new();
    }

    let mutation = if name.contains("move") || name.contains("rename") {
        StructuredMutation::Move
    } else if name.contains("copy") {
        StructuredMutation::Copy
    } else {
        StructuredMutation::Write
    };
    let mut targets = structured_file_targets(value, mutation, cwd);
    if name.contains("execute") {
        targets.extend(shell_mutation_targets(value, cwd));
    }
    targets
}

fn structured_file_targets(
    value: &Value,
    mutation: StructuredMutation,
    hook_cwd: Option<&Path>,
) -> Vec<String> {
    let Some(values) = value.as_object() else {
        return Vec::new();
    };
    let mut keys = vec![
        "path",
        "file_path",
        "target",
        "target_path",
        "destination",
        "destination_path",
        "new_path",
    ];
    if matches!(mutation, StructuredMutation::Move) {
        keys.extend(["source", "source_path", "old_path"]);
    }

    effective_tool_cwds(value, hook_cwd)
        .into_iter()
        .flat_map(|cwd| {
            keys.iter()
                .filter_map(|key| values.get(*key).and_then(Value::as_str))
                .map(|target| resolved_target_string(target, cwd.as_deref()))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn shell_mutation_targets(value: &Value, hook_cwd: Option<&Path>) -> Vec<String> {
    let command = match value {
        Value::String(value) => Some(value.as_str()),
        Value::Object(values) => values
            .get("cmd")
            .or_else(|| values.get("command"))
            .and_then(Value::as_str),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) => None,
    };
    let Some(command) = command else {
        return Vec::new();
    };

    effective_tool_cwds(value, hook_cwd)
        .into_iter()
        .flat_map(|cwd| shell_command_mutation_targets(command, cwd.as_deref()))
        .collect()
}

fn operation_destroys_core_descendants(input: &PreToolUseInput, project_roots: &[PathBuf]) -> bool {
    match input.tool_name.as_str() {
        "Bash" | "Shell" | "exec_command" => shell_tool_destroys_core_descendants(
            &input.tool_input,
            input.cwd.as_deref(),
            project_roots,
        ),
        "write_stdin" => input
            .tool_input
            .as_object()
            .and_then(|values| values.get("chars"))
            .and_then(Value::as_str)
            .is_some_and(|command| {
                shell_command_destroys_core_descendants(
                    command,
                    input.cwd.as_deref(),
                    project_roots,
                )
            }),
        name if name.starts_with("mcp__") => {
            effective_tool_cwds(&input.tool_input, input.cwd.as_deref())
                .into_iter()
                .any(|cwd| {
                    structured_mcp_destroys_core_descendants(
                        name,
                        &input.tool_input,
                        cwd.as_deref(),
                        project_roots,
                    )
                })
                || (name.to_ascii_lowercase().contains("execute")
                    && shell_tool_destroys_core_descendants(
                        &input.tool_input,
                        input.cwd.as_deref(),
                        project_roots,
                    ))
        }
        _ => false,
    }
}

fn bound_hook_project_roots(input: &PreToolUseInput, targets: &[String]) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(cwd) = input.cwd.as_deref() {
        push_bound_project_root(&mut roots, cwd);
    }
    if let Some(cwd) = effective_tool_cwd(&input.tool_input, input.cwd.as_deref()) {
        push_bound_project_root(&mut roots, &cwd);
    }
    if let Some(values) = input.tool_input.as_object() {
        for cwd in ["workdir", "cwd"]
            .into_iter()
            .filter_map(|key| values.get(key).and_then(Value::as_str))
        {
            push_bound_project_root(&mut roots, &normalized_path(cwd, input.cwd.as_deref()));
        }
    }
    for target in targets {
        let target = Path::new(target);
        if target.is_absolute() {
            push_bound_project_root(&mut roots, target);
        }
    }
    roots
}

fn push_bound_project_root(roots: &mut Vec<PathBuf>, candidate: &Path) {
    if let Some(root) = find_bound_project_root(candidate) {
        push_unique_project_root(roots, root);
    }
    if let Ok(physical) = fs::canonicalize(candidate) {
        if physical != candidate {
            if let Some(root) = find_bound_project_root(&physical) {
                push_unique_project_root(roots, root);
            }
        }
    }
}

fn push_unique_project_root(roots: &mut Vec<PathBuf>, root: PathBuf) {
    if !roots.contains(&root) {
        roots.push(root);
    }
}

fn shell_tool_destroys_core_descendants(
    value: &Value,
    hook_cwd: Option<&Path>,
    project_roots: &[PathBuf],
) -> bool {
    let command = match value {
        Value::String(command) => Some(command.as_str()),
        Value::Object(values) => values
            .get("cmd")
            .or_else(|| values.get("command"))
            .and_then(Value::as_str),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) => None,
    };
    let Some(command) = command else {
        return false;
    };
    effective_tool_cwds(value, hook_cwd)
        .into_iter()
        .any(|cwd| shell_command_destroys_core_descendants(command, cwd.as_deref(), project_roots))
}

fn effective_tool_cwd(value: &Value, hook_cwd: Option<&Path>) -> Option<PathBuf> {
    effective_tool_cwds(value, hook_cwd)
        .into_iter()
        .next()
        .flatten()
}

fn effective_tool_cwds(value: &Value, hook_cwd: Option<&Path>) -> Vec<Option<PathBuf>> {
    let mut candidates = Vec::new();
    if let Some(values) = value.as_object() {
        for cwd in ["workdir", "cwd"]
            .into_iter()
            .filter_map(|key| values.get(key).and_then(Value::as_str))
        {
            let candidate = Some(resolved_path(cwd, hook_cwd, true));
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
    }
    if candidates.is_empty() {
        candidates.push(filesystem_cwd(hook_cwd));
    }
    candidates
}

fn shell_command_destroys_core_descendants(
    command: &str,
    effective_cwd: Option<&Path>,
    project_roots: &[PathBuf],
) -> bool {
    struct PendingCommand {
        command: String,
        cwd: Option<PathBuf>,
        ambiguous_cwd: bool,
        bound_roots: Vec<PathBuf>,
    }

    let mut initial_roots = project_roots.to_vec();
    if let Some(cwd) = effective_cwd {
        push_bound_project_root(&mut initial_roots, cwd);
    }
    let mut pending = vec![PendingCommand {
        command: command.trim().to_owned(),
        cwd: effective_cwd.map(Path::to_path_buf),
        ambiguous_cwd: false,
        bound_roots: initial_roots,
    }];
    while let Some(pending_command) = pending.pop() {
        if pending_command.command.is_empty() {
            continue;
        }
        let Some(plan) = shell_execution_plan(
            &pending_command.command,
            pending_command.cwd.clone(),
            pending_command.ambiguous_cwd,
        ) else {
            let blocked = if pending_command.ambiguous_cwd {
                shell_segment_destroys_core_descendants_from_unknown(
                    &pending_command.command,
                    &pending_command.bound_roots,
                )
            } else {
                shell_segment_destroys_core_descendants(
                    &pending_command.command,
                    pending_command.cwd.as_deref(),
                    &pending_command.bound_roots,
                )
            };
            if blocked {
                return true;
            }
            continue;
        };
        let mut bound_roots = pending_command.bound_roots;
        for root in plan.bound_roots {
            push_unique_project_root(&mut bound_roots, root);
        }
        for execution in plan.executions {
            for segment in execution.commands {
                for sequential_cwd in &execution.cwds.known {
                    if shell_segment_destroys_core_descendants(
                        segment,
                        sequential_cwd.as_deref(),
                        &bound_roots,
                    ) {
                        return true;
                    }
                    if let LiteralShellCommandParse::Exec(command) = literal_shell_command(segment)
                    {
                        if let Some(payload) = static_shell_wrapper_payload(&command) {
                            let cwd = apply_filesystem_cwd_overrides(
                                sequential_cwd.as_deref(),
                                &payload.cwd_overrides,
                            );
                            let mut wrapper_roots = bound_roots.clone();
                            if let Some(cwd) = cwd.as_deref() {
                                push_bound_project_root(&mut wrapper_roots, cwd);
                            }
                            pending.push(PendingCommand {
                                command: payload.command.clone(),
                                cwd,
                                ambiguous_cwd: false,
                                bound_roots: wrapper_roots,
                            });
                        }
                    }
                }
                if execution.cwds.ambiguous {
                    if shell_segment_destroys_core_descendants_from_unknown(segment, &bound_roots) {
                        return true;
                    }
                    if let LiteralShellCommandParse::Exec(command) = literal_shell_command(segment)
                    {
                        if let Some(payload) = static_shell_wrapper_payload(&command) {
                            let cwd = apply_filesystem_cwd_overrides(None, &payload.cwd_overrides)
                                .filter(|cwd| cwd.is_absolute());
                            let mut wrapper_roots = bound_roots.clone();
                            if let Some(cwd) = cwd.as_deref() {
                                push_bound_project_root(&mut wrapper_roots, cwd);
                            }
                            pending.push(PendingCommand {
                                command: payload.command.clone(),
                                ambiguous_cwd: cwd.is_none(),
                                cwd,
                                bound_roots: wrapper_roots,
                            });
                        }
                    }
                }
            }
        }
    }
    false
}

fn shell_segment_destroys_core_descendants(
    segment: &str,
    effective_cwd: Option<&Path>,
    project_roots: &[PathBuf],
) -> bool {
    let LiteralShellCommandParse::Exec(command) = literal_shell_command(segment) else {
        return false;
    };
    let effective_cwd = apply_filesystem_cwd_overrides(effective_cwd, &command.cwd_overrides);
    let mut roots = project_roots.to_vec();
    if let Some(cwd) = effective_cwd.as_deref() {
        push_bound_project_root(&mut roots, cwd);
    }
    roots.iter().any(|project_root| {
        literal_command_destroys_matching_scope(&command, effective_cwd.as_deref(), |scope| {
            path_includes_project_root(scope, project_root)
        })
    }) || literal_command_destroys_matching_scope(&command, effective_cwd.as_deref(), |scope| {
        scope.is_absolute()
            && destructive_scope_contains_bound_project(scope, destructive_symlink_policy(&command))
    })
}

fn shell_segment_destroys_core_descendants_from_unknown(
    segment: &str,
    project_roots: &[PathBuf],
) -> bool {
    let LiteralShellCommandParse::Exec(command) = literal_shell_command(segment) else {
        return project_roots
            .iter()
            .any(|root| shell_segment_may_mutate_core_from_unknown(segment, root));
    };
    let exact_override = apply_filesystem_cwd_overrides(None, &command.cwd_overrides)
        .filter(|cwd| cwd.is_absolute());
    let mut roots = project_roots.to_vec();
    if let Some(cwd) = exact_override.as_deref() {
        push_bound_project_root(&mut roots, cwd);
    }
    if command.operation == "git" {
        if let Some(GitCleanScope::Exact(scope)) =
            git_clean_scope(&command, exact_override.as_deref())
        {
            push_bound_project_root(&mut roots, &scope);
        }
    }
    let destroys_bound_descendant = |cwd: Option<&Path>| {
        literal_command_destroys_matching_scope(&command, cwd, |scope| {
            scope.is_absolute()
                && destructive_scope_contains_bound_project(
                    scope,
                    destructive_symlink_policy(&command),
                )
        })
    };
    if destroys_bound_descendant(exact_override.as_deref()) {
        return true;
    }
    if roots
        .iter()
        .any(|root| shell_segment_may_mutate_core_from_unknown(segment, root))
    {
        return true;
    }
    roots.iter().any(|project_root| {
        let target_is_ancestor = |target: &str| {
            exact_override.as_deref().map_or_else(
                || path_may_be_project_ancestor_from_unknown(target, project_root),
                |cwd| path_is_project_ancestor(target, Some(cwd), project_root),
            )
        };
        match command.operation.as_str() {
            "rm" if rm_is_recursive(&command.arguments) => rm_recursive_targets(&command.arguments)
                .into_iter()
                .any(target_is_ancestor),
            "mv" => move_destroys_project_ancestor(&command, target_is_ancestor),
            "git" => exact_override.as_deref().map_or_else(
                || git_clean_may_destroy_project_from_unknown(&command, project_root),
                |cwd| git_clean_destroys_project_ancestor(&command, Some(cwd), project_root),
            ),
            "find" => find_delete_destroys_project_ancestor(&command, target_is_ancestor),
            "chmod" | "chown" | "chgrp" => {
                recursive_metadata_change_destroys_project_ancestor(&command, target_is_ancestor)
            }
            _ => false,
        }
    })
}

struct LiteralShellCommand {
    operation: String,
    arguments: Vec<ShellWord>,
    cwd_overrides: Vec<String>,
    cwd_changing_shell_builtin: bool,
    conservative_git_pathspecs: bool,
}

enum LiteralShellCommandParse {
    Exec(LiteralShellCommand),
    NoExec,
    Opaque,
}

#[derive(Clone)]
enum ShellWord {
    Static(String),
    Unsupported,
}

impl ShellWord {
    fn static_value(&self) -> Option<&str> {
        match self {
            Self::Static(value) => Some(value),
            Self::Unsupported => None,
        }
    }
}

fn static_arguments(arguments: &[ShellWord]) -> Option<Vec<&str>> {
    arguments.iter().map(ShellWord::static_value).collect()
}

fn literal_shell_command(segment: &str) -> LiteralShellCommandParse {
    let mut reader = StaticShellWordReader::new(segment);
    let mut words = Vec::new();
    loop {
        if reader.skip_static_redirection() {
            continue;
        }
        let Some(word) = reader.next_word() else {
            break;
        };
        words.push(word);
    }
    let word_refs = words
        .iter()
        .map_while(ShellWord::static_value)
        .collect::<Vec<_>>();
    let location = match shell_command_location(&word_refs) {
        CommandResolution::Exec(location) => location,
        CommandResolution::NoExec => return LiteralShellCommandParse::NoExec,
        CommandResolution::Incomplete if word_refs.len() == words.len() => {
            return LiteralShellCommandParse::NoExec;
        }
        CommandResolution::Incomplete => return LiteralShellCommandParse::Opaque,
        CommandResolution::Opaque => return LiteralShellCommandParse::Opaque,
    };
    let Some(operation_word) = words.get(location.index).and_then(ShellWord::static_value) else {
        return LiteralShellCommandParse::Opaque;
    };
    let operation = operation_word
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(operation_word)
        .to_ascii_lowercase();
    let conservative_git_pathspecs = word_refs[..location.index]
        .iter()
        .any(|word| word.starts_with("GIT_ICASE_PATHSPECS="));
    let arguments = words[location.index + 1..].to_vec();
    LiteralShellCommandParse::Exec(LiteralShellCommand {
        operation,
        arguments,
        cwd_overrides: location.cwd_overrides,
        cwd_changing_shell_builtin: location.preserves_shell_builtin_context
            && operation_word == "cd",
        conservative_git_pathspecs,
    })
}

struct StaticShellWrapperPayload {
    command: String,
    cwd_overrides: Vec<String>,
}

fn static_shell_wrapper_payload(
    command: &LiteralShellCommand,
) -> Option<StaticShellWrapperPayload> {
    if !matches!(command.operation.as_str(), "sh" | "bash") {
        return None;
    }

    let mut command_string_mode = false;
    let mut arguments = command.arguments.iter();
    while let Some(argument) = arguments.next().and_then(ShellWord::static_value) {
        if argument == "--" {
            let payload = command_string_mode
                .then(|| arguments.next().and_then(ShellWord::static_value))
                .flatten()?;
            return Some(StaticShellWrapperPayload {
                command: payload.to_owned(),
                cwd_overrides: command.cwd_overrides.clone(),
            });
        }
        if argument.starts_with("--") {
            if command_string_mode {
                return None;
            }
            match argument {
                "--rcfile" | "--init-file" => {
                    arguments.next()?.static_value()?;
                }
                "--debug" | "--debugger" | "--dump-po-strings" | "--dump-strings" | "--login"
                | "--noediting" | "--noprofile" | "--norc" | "--posix" | "--pretty-print"
                | "--restricted" | "--verbose" => {}
                _ => return None,
            }
            continue;
        }
        if (argument.starts_with('-') || argument.starts_with('+')) && argument.len() > 1 {
            let options = &argument[1..];
            command_string_mode |= options.contains('c');
            for _ in options.chars().filter(|option| matches!(option, 'o' | 'O')) {
                arguments.next()?.static_value()?;
            }
            continue;
        }
        return command_string_mode.then(|| StaticShellWrapperPayload {
            command: argument.to_owned(),
            cwd_overrides: command.cwd_overrides.clone(),
        });
    }
    None
}

struct StaticShellWordReader<'input> {
    input: &'input str,
    index: usize,
    halted: bool,
}

impl<'input> StaticShellWordReader<'input> {
    fn new(input: &'input str) -> Self {
        Self {
            input,
            index: 0,
            halted: false,
        }
    }

    fn next_word(&mut self) -> Option<ShellWord> {
        if self.halted {
            return None;
        }
        let bytes = self.input.as_bytes();
        while bytes
            .get(self.index)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            self.index += 1;
        }
        if bytes.get(self.index) == Some(&b'#') {
            return None;
        }

        let mut word = String::new();
        let mut started = false;
        let mut supported = true;
        while let Some(byte) = bytes.get(self.index).copied() {
            if byte.is_ascii_whitespace() {
                break;
            }
            match byte {
                b'\'' => {
                    started = true;
                    self.index += 1;
                    let start = self.index;
                    while bytes.get(self.index) != Some(&b'\'') {
                        self.index += 1;
                        if self.index >= bytes.len() {
                            self.halted = true;
                            return Some(ShellWord::Unsupported);
                        }
                    }
                    word.push_str(&self.input[start..self.index]);
                    self.index += 1;
                }
                b'"' => {
                    started = true;
                    self.index += 1;
                    loop {
                        let Some(byte) = bytes.get(self.index).copied() else {
                            self.halted = true;
                            return Some(ShellWord::Unsupported);
                        };
                        match byte {
                            b'"' => {
                                self.index += 1;
                                break;
                            }
                            b'$' => {
                                supported = false;
                                if bytes.get(self.index + 1) == Some(&b'(') {
                                    self.halted = true;
                                    self.index = bytes.len();
                                    break;
                                }
                                self.index += 1;
                            }
                            b'`' => {
                                self.halted = true;
                                self.index = bytes.len();
                                supported = false;
                                break;
                            }
                            b'\\' => {
                                self.index += 1;
                                let Some(escaped) = bytes.get(self.index).copied() else {
                                    self.halted = true;
                                    return Some(ShellWord::Unsupported);
                                };
                                if matches!(escaped, b'$' | b'`' | b'"' | b'\\' | b'\n') {
                                    if escaped != b'\n' {
                                        word.push(escaped as char);
                                    }
                                } else {
                                    word.push('\\');
                                    word.push(escaped as char);
                                }
                                self.index += 1;
                            }
                            _ if byte.is_ascii() => {
                                word.push(byte as char);
                                self.index += 1;
                            }
                            _ => {
                                let character = self.input[self.index..].chars().next()?;
                                word.push(character);
                                self.index += character.len_utf8();
                            }
                        }
                    }
                }
                b'\\' => {
                    started = true;
                    self.index += 1;
                    let Some(escaped) = bytes.get(self.index).copied() else {
                        self.halted = true;
                        return Some(ShellWord::Unsupported);
                    };
                    if escaped != b'\n' {
                        let character = self.input[self.index..].chars().next()?;
                        word.push(character);
                        self.index += character.len_utf8();
                    } else {
                        self.index += 1;
                    }
                }
                b'$' => {
                    started = true;
                    supported = false;
                    if bytes.get(self.index + 1) == Some(&b'(') {
                        self.halted = true;
                        self.index = bytes.len();
                        break;
                    }
                    self.index += 1;
                }
                b'`' => {
                    started = true;
                    supported = false;
                    self.halted = true;
                    self.index = bytes.len();
                    break;
                }
                b'*' | b'?' | b'[' | b'~' | b'{' => {
                    started = true;
                    supported = false;
                    self.index += 1;
                }
                b';' | b'|' | b'&' | b'<' | b'>' | b'(' | b')' => break,
                _ if byte.is_ascii() => {
                    started = true;
                    word.push(byte as char);
                    self.index += 1;
                }
                _ => {
                    started = true;
                    let character = self.input[self.index..].chars().next()?;
                    word.push(character);
                    self.index += character.len_utf8();
                }
            }
        }
        started.then_some({
            if supported {
                ShellWord::Static(word)
            } else {
                ShellWord::Unsupported
            }
        })
    }

    fn skip_static_redirection(&mut self) -> bool {
        if self.halted {
            return false;
        }
        let bytes = self.input.as_bytes();
        while bytes
            .get(self.index)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            self.index += 1;
        }

        let start = self.index;
        while bytes
            .get(self.index)
            .is_some_and(|byte| byte.is_ascii_digit())
        {
            self.index += 1;
        }
        if bytes.get(self.index) == Some(&b'&') && bytes.get(self.index + 1) == Some(&b'>') {
            self.index += 2;
            if bytes.get(self.index) == Some(&b'>') {
                self.index += 1;
            }
        } else {
            let Some(operator) = bytes.get(self.index).copied() else {
                self.index = start;
                return false;
            };
            if !matches!(operator, b'<' | b'>') {
                self.index = start;
                return false;
            }
            self.index += 1;
            match (operator, bytes.get(self.index).copied()) {
                (b'<', Some(b'<')) => {
                    self.index += 1;
                    if bytes.get(self.index) == Some(&b'-') {
                        self.index += 1;
                    }
                }
                (b'<', Some(b'&' | b'>')) | (b'>', Some(b'&' | b'>' | b'|')) => {
                    self.index += 1;
                }
                _ => {}
            }
        }

        let _ = self.next_word();
        true
    }
}

fn rm_is_recursive(arguments: &[ShellWord]) -> bool {
    arguments
        .iter()
        .filter_map(ShellWord::static_value)
        .take_while(|argument| *argument != "--")
        .any(|argument| {
            argument == "--recursive"
                || (argument.starts_with('-')
                    && !argument.starts_with("--")
                    && argument[1..]
                        .chars()
                        .any(|option| matches!(option, 'r' | 'R')))
        })
}

fn rm_recursive_targets(arguments: &[ShellWord]) -> Vec<&str> {
    let mut options = true;
    let mut targets = Vec::new();
    for argument in arguments.iter().filter_map(ShellWord::static_value) {
        if options && argument == "--" {
            options = false;
        } else if !options || argument == "-" || !argument.starts_with('-') {
            targets.push(argument);
        }
    }
    targets
}

fn move_destroys_project_ancestor<F>(command: &LiteralShellCommand, target_is_ancestor: F) -> bool
where
    F: Fn(&str) -> bool + Copy,
{
    let Some(arguments) = static_arguments(&command.arguments) else {
        return command
            .arguments
            .iter()
            .filter_map(ShellWord::static_value)
            .any(|argument| {
                target_is_ancestor(argument)
                    || argument
                        .split_once('=')
                        .is_some_and(|(_, value)| target_is_ancestor(value))
            });
    };
    let Some(layout) = move_layout(&arguments) else {
        return arguments.iter().any(|argument| {
            target_is_ancestor(argument)
                || argument
                    .split_once('=')
                    .is_some_and(|(_, value)| target_is_ancestor(value))
        });
    };
    let (sources, destination) = if let Some(target_directory) = layout.target_directory {
        (layout.operands.as_slice(), Some(target_directory))
    } else {
        let Some((destination, sources)) = layout.operands.split_last() else {
            return false;
        };
        (sources, Some(*destination))
    };
    if sources.is_empty() {
        return false;
    }
    if sources.iter().any(|source| target_is_ancestor(source)) {
        return true;
    }
    layout.replaces_destination && destination.is_some_and(target_is_ancestor)
}

struct MoveLayout<'value> {
    operands: Vec<&'value str>,
    target_directory: Option<&'value str>,
    replaces_destination: bool,
}

fn move_layout<'value>(arguments: &[&'value str]) -> Option<MoveLayout<'value>> {
    let mut layout = MoveLayout {
        operands: Vec::new(),
        target_directory: None,
        replaces_destination: false,
    };
    let mut options = true;
    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index];
        if options && argument == "--" {
            options = false;
            index += 1;
            continue;
        }
        if options && argument.starts_with("--") {
            let (name, attached_value) = argument
                .split_once('=')
                .map_or((argument, None), |(name, value)| (name, Some(value)));
            match name {
                "--target-directory" | "--suffix" => {
                    let value = if let Some(value) = attached_value {
                        if value.is_empty() {
                            return None;
                        }
                        value
                    } else {
                        index += 1;
                        *arguments.get(index)?
                    };
                    if name == "--target-directory"
                        && layout.target_directory.replace(value).is_some()
                    {
                        return None;
                    }
                }
                "--no-target-directory" | "--exchange" => {
                    if attached_value.is_some() {
                        return None;
                    }
                    layout.replaces_destination = true;
                }
                "--backup"
                | "--debug"
                | "--force"
                | "--interactive"
                | "--no-clobber"
                | "--no-copy"
                | "--strip-trailing-slashes"
                | "--update"
                | "--verbose" => {
                    if attached_value.is_some() {
                        return None;
                    }
                }
                _ => return None,
            }
            index += 1;
            continue;
        }
        if options && argument.starts_with('-') && argument != "-" {
            let cluster = argument.strip_prefix('-')?;
            if !cluster.is_ascii() {
                return None;
            }
            let bytes = cluster.as_bytes();
            let mut option_index = 0;
            while option_index < bytes.len() {
                match bytes[option_index] as char {
                    'T' => layout.replaces_destination = true,
                    't' | 'S' => {
                        let attached = &cluster[option_index + 1..];
                        let value = if attached.is_empty() {
                            index += 1;
                            *arguments.get(index)?
                        } else {
                            attached
                        };
                        if bytes[option_index] as char == 't'
                            && layout.target_directory.replace(value).is_some()
                        {
                            return None;
                        }
                        option_index = bytes.len();
                        continue;
                    }
                    'b' | 'f' | 'i' | 'n' | 'u' | 'v' | 'Z' => {}
                    _ => return None,
                }
                option_index += 1;
            }
            index += 1;
            continue;
        }
        layout.operands.push(argument);
        index += 1;
    }
    if layout.target_directory.is_some() && layout.replaces_destination {
        return None;
    }
    Some(layout)
}

enum GitCleanScope {
    Exact(PathBuf),
    Unknown,
}

struct GitCleanInvocation {
    scope: GitCleanScope,
    exposes_private_state: bool,
    conservative_pathspecs: bool,
    pathspecs: Vec<String>,
}

struct GitCleanArguments {
    exposes_private_state: bool,
    pathspecs: Vec<String>,
}

fn git_clean_scope(
    command: &LiteralShellCommand,
    effective_cwd: Option<&Path>,
) -> Option<GitCleanScope> {
    let arguments = static_arguments(&command.arguments)?;
    let invocation = git_clean_invocation_from_arguments(
        &arguments,
        effective_cwd,
        command.conservative_git_pathspecs,
    )?;
    if !invocation.exposes_private_state {
        return None;
    }
    Some(invocation.scope)
}

fn git_clean_invocation_from_arguments(
    arguments: &[&str],
    effective_cwd: Option<&Path>,
    mut conservative_pathspecs: bool,
) -> Option<GitCleanInvocation> {
    let mut scope = effective_cwd.map(Path::to_path_buf);
    let mut worktree = None::<Option<PathBuf>>;
    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index];
        if argument == "clean" {
            break;
        }
        if argument == "-C" {
            index += 1;
            let path = arguments.get(index).copied()?;
            scope = resolve_unknown_scope(scope, path);
        } else if let Some(path) = argument.strip_prefix("-C") {
            if path.is_empty() {
                return None;
            }
            scope = resolve_unknown_scope(scope, path);
        } else if argument == "--work-tree" {
            index += 1;
            let path = arguments.get(index).copied()?;
            worktree = Some(resolve_unknown_scope(scope.clone(), path));
        } else if let Some(path) = argument.strip_prefix("--work-tree=") {
            if path.is_empty() {
                return None;
            }
            worktree = Some(resolve_unknown_scope(scope.clone(), path));
        } else if argument == "--icase-pathspecs" {
            conservative_pathspecs = true;
        }
        index += 1;
    }
    if arguments.get(index).copied() != Some("clean") {
        return None;
    }
    let analysis = git_clean_arguments(&arguments[index + 1..])?;
    let scope = match worktree.unwrap_or(scope) {
        Some(scope) => GitCleanScope::Exact(scope),
        None => GitCleanScope::Unknown,
    };
    Some(GitCleanInvocation {
        scope,
        exposes_private_state: analysis.exposes_private_state,
        conservative_pathspecs,
        pathspecs: analysis.pathspecs,
    })
}

fn git_clean_arguments(arguments: &[&str]) -> Option<GitCleanArguments> {
    let mut dry_run = false;
    let mut exposes_ignored = false;
    let mut pathspecs = Vec::new();
    let mut options = true;
    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index];
        if !options {
            pathspecs.push(argument.to_owned());
            index += 1;
            continue;
        }
        if argument == "--" {
            options = false;
            index += 1;
            continue;
        }
        if argument == "--dry-run" {
            dry_run = true;
            index += 1;
            continue;
        }
        if argument == "--exclude" {
            index += 1;
            let pattern = arguments.get(index).copied()?;
            exposes_ignored |= git_clean_exclude_reincludes(pattern);
            index += 1;
            continue;
        }
        if let Some(pattern) = argument.strip_prefix("--exclude=") {
            if pattern.is_empty() {
                return None;
            }
            exposes_ignored |= git_clean_exclude_reincludes(pattern);
            index += 1;
            continue;
        }
        if argument.starts_with("--") {
            index += 1;
            continue;
        }
        if argument == "-" || !argument.starts_with('-') {
            pathspecs.push(argument.to_owned());
            index += 1;
            continue;
        }

        let cluster = argument.strip_prefix('-')?;
        if !cluster.is_ascii() {
            return None;
        }
        let bytes = cluster.as_bytes();
        let mut option_index = 0;
        while option_index < bytes.len() {
            match bytes[option_index] as char {
                'n' => dry_run = true,
                'x' | 'X' => exposes_ignored = true,
                'e' => {
                    let attached = &cluster[option_index + 1..];
                    let pattern = if attached.is_empty() {
                        index += 1;
                        arguments.get(index).copied()?
                    } else {
                        attached
                    };
                    exposes_ignored |= git_clean_exclude_reincludes(pattern);
                    break;
                }
                _ => {}
            }
            option_index += 1;
        }
        index += 1;
    }
    Some(GitCleanArguments {
        exposes_private_state: !dry_run && exposes_ignored,
        pathspecs,
    })
}

fn git_clean_exclude_reincludes(pattern: &str) -> bool {
    pattern.starts_with('!')
}

fn git_clean_pathspec_mutation_targets(
    arguments: &[&str],
    effective_cwd: Option<&Path>,
    conservative_pathspecs: bool,
) -> Option<Vec<String>> {
    let invocation =
        git_clean_invocation_from_arguments(arguments, effective_cwd, conservative_pathspecs)?;
    if !invocation.exposes_private_state {
        return Some(Vec::new());
    }
    let GitCleanScope::Exact(scope) = invocation.scope else {
        return Some(invocation.pathspecs);
    };
    let worktree = git_worktree_root(&scope).unwrap_or_else(|| scope.clone());
    let mut targets = Vec::new();
    for pathspec in invocation.pathspecs {
        let Some(target) = git_clean_pathspec_target(
            &pathspec,
            &scope,
            &worktree,
            invocation.conservative_pathspecs,
        ) else {
            continue;
        };
        targets.push(target.to_string_lossy().into_owned());
        if fs::symlink_metadata(&target)
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        {
            targets.push(target.join("*").to_string_lossy().into_owned());
        }
    }
    Some(targets)
}

fn git_worktree_root(scope: &Path) -> Option<PathBuf> {
    scope
        .ancestors()
        .find(|candidate| {
            fs::symlink_metadata(candidate.join(".git")).is_ok_and(|metadata| {
                !metadata.file_type().is_symlink() && (metadata.is_dir() || metadata.is_file())
            })
        })
        .map(Path::to_path_buf)
}

fn git_clean_pathspec_target(
    pathspec: &str,
    scope: &Path,
    worktree: &Path,
    conservative_pathspecs: bool,
) -> Option<PathBuf> {
    let pathspec = git_clean_pathspec_parts(pathspec)?;
    let base = if pathspec.top { worktree } else { scope };
    if conservative_pathspecs || pathspec.conservative_magic {
        return Some(git_clean_conservative_scope(pathspec.body, base).join("*"));
    }
    Some(resolved_path(pathspec.body, Some(base), false))
}

fn git_clean_conservative_scope(pathspec_body: &str, base: &Path) -> PathBuf {
    let mut scope = base.to_path_buf();
    for component in Path::new(pathspec_body).components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                scope.pop();
            }
            std::path::Component::RootDir => scope = PathBuf::from("/"),
            std::path::Component::Prefix(_) | std::path::Component::Normal(_) => break,
        }
    }
    scope
}

struct GitCleanPathspec<'pathspec> {
    top: bool,
    body: &'pathspec str,
    conservative_magic: bool,
}

fn git_clean_pathspec_parts(pathspec: &str) -> Option<GitCleanPathspec<'_>> {
    if let Some(body) = pathspec.strip_prefix(":/") {
        return Some(GitCleanPathspec {
            top: true,
            body,
            conservative_magic: false,
        });
    }
    if pathspec.starts_with(":!") || pathspec.starts_with(":^") {
        return None;
    }
    let Some(magic) = pathspec.strip_prefix(":(") else {
        return Some(GitCleanPathspec {
            top: false,
            body: pathspec,
            conservative_magic: false,
        });
    };
    let closing = magic.find(')')?;
    let signature = &magic[..closing];
    if signature
        .split(',')
        .any(|item| matches!(item, "exclude" | "!" | "^"))
    {
        return None;
    }
    let top = signature.split(',').any(|item| matches!(item, "top" | "/"));
    let conservative_magic = signature
        .split(',')
        .any(|item| !matches!(item, "top" | "/" | "literal" | "glob"));
    Some(GitCleanPathspec {
        top,
        body: &magic[closing + 1..],
        conservative_magic,
    })
}

fn git_clean_destroys_project_ancestor(
    command: &LiteralShellCommand,
    effective_cwd: Option<&Path>,
    project_root: &Path,
) -> bool {
    git_clean_destroys_matching_scope(command, effective_cwd, |scope| {
        path_includes_project_root(scope, project_root)
    })
}

fn git_clean_may_destroy_project_from_unknown(
    command: &LiteralShellCommand,
    project_root: &Path,
) -> bool {
    match git_clean_scope(command, None) {
        Some(GitCleanScope::Exact(_)) => {
            git_clean_destroys_project_ancestor(command, None, project_root)
        }
        Some(GitCleanScope::Unknown) => true,
        None => false,
    }
}

fn git_clean_destroys_matching_scope<F>(
    command: &LiteralShellCommand,
    effective_cwd: Option<&Path>,
    scope_matches: F,
) -> bool
where
    F: Fn(&Path) -> bool,
{
    let Some(arguments) = static_arguments(&command.arguments) else {
        return false;
    };
    let Some(invocation) = git_clean_invocation_from_arguments(
        &arguments,
        effective_cwd,
        command.conservative_git_pathspecs,
    ) else {
        return false;
    };
    if !invocation.exposes_private_state {
        return false;
    }
    let GitCleanScope::Exact(scope) = &invocation.scope else {
        return false;
    };
    if invocation.pathspecs.is_empty() {
        return scope_matches(scope);
    }

    let worktree = git_worktree_root(scope).unwrap_or_else(|| scope.clone());
    invocation.pathspecs.iter().any(|pathspec| {
        git_clean_pathspec_target(
            pathspec,
            scope,
            &worktree,
            invocation.conservative_pathspecs,
        )
        .map_or_else(|| scope_matches(scope), |target| scope_matches(&target))
    })
}

fn resolve_unknown_scope(scope: Option<PathBuf>, target: &str) -> Option<PathBuf> {
    let decoded = decode_shell_token(target);
    let target = Path::new(&decoded);
    if target.is_absolute() {
        Some(resolved_path(&decoded, None, true))
    } else {
        scope.map(|scope| resolved_path(&decoded, Some(&scope), true))
    }
}

fn find_delete_destroys_project_ancestor<F>(
    command: &LiteralShellCommand,
    target_is_ancestor: F,
) -> bool
where
    F: Fn(&str) -> bool + Copy,
{
    if !command
        .arguments
        .iter()
        .filter_map(ShellWord::static_value)
        .any(|argument| argument == "-delete")
    {
        return false;
    }

    let mut index = 0;
    while let Some(argument) = command
        .arguments
        .get(index)
        .and_then(ShellWord::static_value)
    {
        match argument {
            "-H" | "-L" | "-P" => index += 1,
            "-D" => {
                if command.arguments.get(index + 1).is_none() {
                    return false;
                }
                index += 2;
            }
            argument if argument.starts_with("-D") || argument.starts_with("-O") => index += 1,
            _ => break,
        }
    }

    let mut starting_points = Vec::new();
    let mut dynamic_start = false;
    while let Some(argument) = command.arguments.get(index) {
        let Some(argument) = argument.static_value() else {
            dynamic_start = true;
            break;
        };
        if argument.starts_with('-') || matches!(argument, "!" | "(" | ")" | ",") {
            break;
        }
        starting_points.push(argument);
        index += 1;
    }
    if starting_points.is_empty() {
        !dynamic_start && target_is_ancestor(".")
    } else {
        starting_points
            .iter()
            .any(|target| target_is_ancestor(target))
    }
}

fn recursive_metadata_change_destroys_project_ancestor<F>(
    command: &LiteralShellCommand,
    target_is_ancestor: F,
) -> bool
where
    F: Fn(&str) -> bool + Copy,
{
    if let Some(arguments) = static_arguments(&command.arguments) {
        let Some(layout) = metadata_layout(&command.operation, &arguments) else {
            return false;
        };
        return layout.recursive
            && layout
                .targets
                .iter()
                .any(|target| target_is_ancestor(target));
    }

    mixed_metadata_targets(&command.operation, &command.arguments).is_some_and(
        |(recursive, targets)| recursive && targets.into_iter().flatten().any(target_is_ancestor),
    )
}

fn literal_command_destroys_matching_scope<F>(
    command: &LiteralShellCommand,
    effective_cwd: Option<&Path>,
    scope_matches: F,
) -> bool
where
    F: Fn(&Path) -> bool + Copy,
{
    let target_matches = |target: &str| {
        resolved_static_scope(target, effective_cwd).is_some_and(|scope| scope_matches(&scope))
    };
    match command.operation.as_str() {
        "rm" if rm_is_recursive(&command.arguments) => rm_recursive_targets(&command.arguments)
            .into_iter()
            .any(target_matches),
        "mv" => move_destroys_project_ancestor(command, target_matches),
        "git" => git_clean_destroys_matching_scope(command, effective_cwd, scope_matches),
        "find" => find_delete_destroys_project_ancestor(command, target_matches),
        "chmod" | "chown" | "chgrp" => {
            recursive_metadata_change_destroys_project_ancestor(command, target_matches)
        }
        _ => false,
    }
}

#[derive(Clone, Copy)]
enum DestructiveSymlinkPolicy {
    Skip,
    RejectRoot,
    RejectAny,
}

fn destructive_symlink_policy(command: &LiteralShellCommand) -> DestructiveSymlinkPolicy {
    match command.operation.as_str() {
        "find" => find_symlink_policy(&command.arguments),
        "chown" | "chgrp" => recursive_metadata_symlink_policy(&command.arguments),
        _ => DestructiveSymlinkPolicy::Skip,
    }
}

fn find_symlink_policy(arguments: &[ShellWord]) -> DestructiveSymlinkPolicy {
    if arguments
        .iter()
        .filter_map(ShellWord::static_value)
        .any(|argument| argument == "-follow")
    {
        return DestructiveSymlinkPolicy::RejectAny;
    }
    let mut policy = DestructiveSymlinkPolicy::Skip;
    let mut index = 0;
    while let Some(argument) = arguments.get(index).and_then(ShellWord::static_value) {
        match argument {
            "-H" => policy = DestructiveSymlinkPolicy::RejectRoot,
            "-L" => policy = DestructiveSymlinkPolicy::RejectAny,
            "-P" => policy = DestructiveSymlinkPolicy::Skip,
            "-D" => {
                index += 1;
                if arguments
                    .get(index)
                    .and_then(ShellWord::static_value)
                    .is_none()
                {
                    break;
                }
            }
            argument if argument.starts_with("-D") || argument.starts_with("-O") => {}
            _ => break,
        }
        index += 1;
    }
    policy
}

fn recursive_metadata_symlink_policy(arguments: &[ShellWord]) -> DestructiveSymlinkPolicy {
    let mut policy = DestructiveSymlinkPolicy::Skip;
    for argument in arguments.iter().filter_map(ShellWord::static_value) {
        if argument == "--" {
            break;
        }
        if argument == "-" || !argument.starts_with('-') {
            continue;
        }
        if argument.starts_with("--") {
            continue;
        }
        for option in argument[1..].chars() {
            match option {
                'H' => policy = DestructiveSymlinkPolicy::RejectRoot,
                'L' => policy = DestructiveSymlinkPolicy::RejectAny,
                'P' => policy = DestructiveSymlinkPolicy::Skip,
                _ => {}
            }
        }
    }
    policy
}

fn resolved_static_scope(target: &str, effective_cwd: Option<&Path>) -> Option<PathBuf> {
    let target = decode_shell_token(target);
    if target.is_empty()
        || target
            .chars()
            .any(|character| matches!(character, '*' | '?' | '[' | '{' | '$' | '`'))
    {
        return None;
    }
    Some(resolved_path(&target, effective_cwd, false))
}

fn mixed_metadata_targets<'value>(
    operation: &str,
    arguments: &'value [ShellWord],
) -> Option<(bool, Vec<Option<&'value str>>)> {
    let mut recursive = false;
    let mut uses_reference = false;
    let mut operands = Vec::new();
    let mut options = true;
    let mut index = 0;
    while index < arguments.len() {
        let Some(argument) = arguments[index].static_value() else {
            operands.push(None);
            index += 1;
            continue;
        };
        if options && argument == "--" {
            options = false;
            index += 1;
            continue;
        }
        if options && argument.starts_with("--") {
            let (name, attached_value) = argument
                .split_once('=')
                .map_or((argument, None), |(name, value)| (name, Some(value)));
            match name {
                "--recursive" if attached_value.is_none() => recursive = true,
                "--reference" => {
                    uses_reference = true;
                    if attached_value.is_none() {
                        index += 1;
                        arguments.get(index)?;
                    }
                }
                "--from" if operation == "chown" => {
                    if attached_value.is_none() {
                        index += 1;
                        arguments.get(index)?;
                    }
                }
                "--changes" | "--silent" | "--quiet" | "--verbose" | "--no-preserve-root"
                | "--preserve-root" | "--dereference" | "--no-dereference"
                    if attached_value.is_none() => {}
                _ => return None,
            }
            index += 1;
            continue;
        }
        if options && argument.starts_with('-') && argument != "-" {
            let cluster = argument.strip_prefix('-')?;
            let valid = cluster.chars().all(|option| match operation {
                "chmod" => matches!(option, 'c' | 'f' | 'v' | 'R'),
                "chown" | "chgrp" => {
                    matches!(option, 'c' | 'f' | 'v' | 'R' | 'h' | 'H' | 'L' | 'P')
                }
                _ => false,
            });
            if valid {
                recursive |= cluster.contains('R');
                index += 1;
                continue;
            }
            if operation != "chmod" {
                return None;
            }
        }
        operands.push(Some(argument));
        index += 1;
    }

    if uses_reference {
        (!operands.is_empty()).then_some((recursive, operands))
    } else {
        (operands.len() >= 2).then(|| (recursive, operands.into_iter().skip(1).collect()))
    }
}

struct MetadataLayout<'value> {
    recursive: bool,
    targets: Vec<&'value str>,
}

fn metadata_layout<'value>(
    operation: &str,
    arguments: &[&'value str],
) -> Option<MetadataLayout<'value>> {
    let mut recursive = false;
    let mut uses_reference = false;
    let mut operands = Vec::new();
    let mut options = true;
    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index];
        if options && argument == "--" {
            options = false;
            index += 1;
            continue;
        }
        if options && argument.starts_with("--") {
            let (name, attached_value) = argument
                .split_once('=')
                .map_or((argument, None), |(name, value)| (name, Some(value)));
            match name {
                "--recursive" => {
                    if attached_value.is_some() {
                        return None;
                    }
                    recursive = true;
                }
                "--reference" => {
                    consume_long_option_value(arguments, &mut index, attached_value)?;
                    uses_reference = true;
                }
                "--from" if operation == "chown" => {
                    consume_long_option_value(arguments, &mut index, attached_value)?;
                }
                "--changes" | "--silent" | "--quiet" | "--verbose" | "--no-preserve-root"
                | "--preserve-root" | "--dereference" | "--no-dereference" => {
                    if attached_value.is_some() {
                        return None;
                    }
                }
                _ => return None,
            }
            index += 1;
            continue;
        }
        if options && argument.starts_with('-') && argument != "-" {
            let cluster = argument.strip_prefix('-')?;
            let valid = cluster.chars().all(|option| match operation {
                "chmod" => matches!(option, 'c' | 'f' | 'v' | 'R'),
                "chown" | "chgrp" => {
                    matches!(option, 'c' | 'f' | 'v' | 'R' | 'h' | 'H' | 'L' | 'P')
                }
                _ => false,
            });
            if valid {
                recursive |= cluster.contains('R');
                index += 1;
                continue;
            }
            if operation != "chmod" {
                return None;
            }
        }
        operands.push(argument);
        index += 1;
    }

    let targets = if uses_reference {
        if operands.is_empty() {
            return None;
        }
        operands
    } else {
        if operands.len() < 2 {
            return None;
        }
        operands.into_iter().skip(1).collect()
    };
    Some(MetadataLayout { recursive, targets })
}

fn consume_long_option_value<'value>(
    arguments: &[&'value str],
    index: &mut usize,
    attached_value: Option<&'value str>,
) -> Option<&'value str> {
    match attached_value {
        Some(value) if !value.is_empty() => Some(value),
        Some(_) => None,
        None => {
            *index += 1;
            arguments.get(*index).copied()
        }
    }
}

fn structured_mcp_destroys_core_descendants(
    tool_name: &str,
    value: &Value,
    effective_cwd: Option<&Path>,
    project_roots: &[PathBuf],
) -> bool {
    let name = tool_name.to_ascii_lowercase();
    if !["delete", "remove", "move", "rename", "replace"]
        .iter()
        .any(|signal| name.contains(signal))
    {
        return false;
    }
    let Some(values) = value.as_object() else {
        return false;
    };
    let targets_destructive_scope = |keys: &[&str]| {
        keys.iter()
            .filter_map(|key| values.get(*key).and_then(Value::as_str))
            .filter_map(|target| resolved_static_scope(target, effective_cwd))
            .any(|scope| {
                project_roots
                    .iter()
                    .any(|project_root| path_includes_project_root(&scope, project_root))
                    || (scope.is_absolute()
                        && destructive_scope_contains_bound_project(
                            &scope,
                            DestructiveSymlinkPolicy::Skip,
                        ))
            })
    };
    if name.contains("delete") || name.contains("remove") || name.contains("replace") {
        return targets_destructive_scope(&[
            "path",
            "file_path",
            "target",
            "target_path",
            "source",
            "source_path",
            "destination",
            "destination_path",
            "old_path",
            "new_path",
        ]);
    }

    let source_is_ancestor =
        targets_destructive_scope(&["path", "file_path", "source", "source_path", "old_path"]);
    let replaces_destination = ["overwrite", "replace", "no_target_directory"]
        .iter()
        .any(|key| values.get(*key).and_then(Value::as_bool) == Some(true))
        || values.get("mode").and_then(Value::as_str) == Some("replace");
    source_is_ancestor
        || (replaces_destination
            && targets_destructive_scope(&[
                "target",
                "target_path",
                "destination",
                "destination_path",
                "new_path",
            ]))
}

fn path_is_project_ancestor(
    target: &str,
    effective_cwd: Option<&Path>,
    project_root: &Path,
) -> bool {
    let target = decode_shell_token(target);
    if target.is_empty()
        || target
            .chars()
            .any(|character| matches!(character, '*' | '?' | '[' | '{' | '$' | '`'))
    {
        return false;
    }
    path_includes_project_root(&resolved_path(&target, effective_cwd, false), project_root)
}

fn path_may_be_project_ancestor_from_unknown(target: &str, project_root: &Path) -> bool {
    let target = decode_shell_token(target);
    if target.is_empty()
        || target
            .chars()
            .any(|character| matches!(character, '*' | '?' | '[' | '{' | '$' | '`'))
    {
        return false;
    }
    let target = resolved_path(&target, None, false);
    !target.is_absolute() || path_includes_project_root(&target, project_root)
}

fn path_includes_project_root(scope: &Path, project_root: &Path) -> bool {
    project_root == scope || project_root.starts_with(scope)
}

fn normalized_path(target: &str, workdir: Option<&Path>) -> PathBuf {
    let workdir = workdir.map(|path| path.to_string_lossy());
    PathBuf::from(resolve_shell_target(target, workdir.as_deref()))
}

fn resolved_path(target: &str, workdir: Option<&Path>, follow_final_symlink: bool) -> PathBuf {
    let target = decode_shell_token(target);
    let lexical = normalized_path(&target, workdir);
    let raw = Path::new(&target);
    let raw = if raw.is_absolute() {
        raw.to_path_buf()
    } else if let Some(workdir) = workdir {
        workdir.join(raw)
    } else {
        return lexical;
    };
    if raw.is_absolute()
        && fs::symlink_metadata(&raw)
            .is_ok_and(|metadata| follow_final_symlink || !metadata.file_type().is_symlink())
    {
        if let Ok(physical) = fs::canonicalize(raw) {
            return physical;
        }
    }
    lexical
}

fn filesystem_cwd(cwd: Option<&Path>) -> Option<PathBuf> {
    cwd.map(|cwd| resolved_path(".", Some(cwd), true))
}

fn apply_filesystem_cwd_overrides(base: Option<&Path>, overrides: &[String]) -> Option<PathBuf> {
    let mut cwd = filesystem_cwd(base);
    for target in overrides {
        cwd = Some(resolved_path(target, cwd.as_deref(), true));
    }
    cwd
}

fn apply_cwd_overrides(base: Option<&Path>, overrides: &[String]) -> Option<PathBuf> {
    let mut cwd = base.map(Path::to_path_buf);
    for override_cwd in overrides {
        cwd = Some(normalized_path(override_cwd, cwd.as_deref()));
    }
    cwd
}

fn write_stdin_mutation_targets(value: &Value, cwd: Option<&Path>) -> Vec<String> {
    let Some(command) = value
        .as_object()
        .and_then(|values| values.get("chars"))
        .and_then(Value::as_str)
    else {
        return Vec::new();
    };
    let command = command.trim();
    if command.is_empty() {
        return Vec::new();
    }
    shell_command_mutation_targets(command, cwd)
}

fn shell_command_mutation_targets(command: &str, workdir: Option<&Path>) -> Vec<String> {
    let Some(plan) = shell_execution_plan(command, workdir.map(Path::to_path_buf), false) else {
        let mut targets = redirection_destinations(command)
            .into_iter()
            .map(|target| resolved_target_string(target, workdir))
            .collect::<Vec<_>>();
        targets.extend(conservative_shell_targets(command, workdir));
        return targets;
    };

    let mut targets = Vec::new();
    for execution in plan.executions {
        for segment in execution.commands {
            for sequential_cwd in &execution.cwds.known {
                targets.extend(shell_segment_mutation_targets(
                    segment,
                    sequential_cwd.as_deref(),
                ));
            }
            if execution.cwds.ambiguous {
                targets.extend(shell_segment_mutation_targets(segment, None));
            }
        }
    }
    targets
}

fn shell_segment_mutation_targets(segment: &str, workdir: Option<&Path>) -> Vec<String> {
    let mut targets = redirection_destinations(segment)
        .into_iter()
        .map(|target| resolved_target_string(target, workdir))
        .collect::<Vec<_>>();
    if has_active_shell_substitution(segment) {
        let cwd = filesystem_cwd(workdir);
        targets.extend(conservative_shell_targets(segment, cwd.as_deref()));
        return targets;
    }
    let command = match literal_shell_command(segment) {
        LiteralShellCommandParse::Exec(command) => command,
        LiteralShellCommandParse::NoExec => return targets,
        LiteralShellCommandParse::Opaque => {
            let cwd = filesystem_cwd(workdir);
            targets.extend(conservative_shell_targets(segment, cwd.as_deref()));
            return targets;
        }
    };
    let effective_cwd = apply_filesystem_cwd_overrides(workdir, &command.cwd_overrides);
    if let Some(payload) = static_shell_wrapper_payload(&command) {
        let wrapper_cwd = apply_filesystem_cwd_overrides(workdir, &payload.cwd_overrides);
        targets.extend(shell_command_mutation_targets(
            &payload.command,
            wrapper_cwd.as_deref(),
        ));
        return targets;
    }
    if shell_command_is_read_only(&command.operation) {
        return targets;
    }

    let Some(arguments) = static_arguments(&command.arguments) else {
        targets.extend(conservative_shell_targets(
            segment,
            effective_cwd.as_deref(),
        ));
        return targets;
    };
    if command.operation == "git" {
        if let Some(git_targets) = git_clean_pathspec_mutation_targets(
            &arguments,
            effective_cwd.as_deref(),
            command.conservative_git_pathspecs,
        ) {
            targets.extend(git_targets);
            return targets;
        }
    }
    let mutation = targeted_mutation_operands(&command.operation, &arguments);
    for target in &mutation.targets {
        targets.push(resolved_target_string(target, effective_cwd.as_deref()));
    }
    if mutation.exact {
        return targets;
    }

    targets.extend(conservative_shell_targets(
        segment,
        effective_cwd.as_deref(),
    ));
    targets
}

fn shell_segment_may_mutate_core_from_unknown(segment: &str, project_root: &Path) -> bool {
    if redirection_destinations(segment)
        .into_iter()
        .any(|target| mutation_target_may_reach_core_from_unknown(target, None, project_root))
    {
        return true;
    }
    if has_active_shell_substitution(segment) {
        return true;
    }
    let command = match literal_shell_command(segment) {
        LiteralShellCommandParse::Exec(command) => command,
        LiteralShellCommandParse::NoExec => return false,
        LiteralShellCommandParse::Opaque => return true,
    };
    if static_shell_wrapper_payload(&command).is_some()
        || shell_command_is_read_only(&command.operation)
    {
        return false;
    }
    let Some(arguments) = static_arguments(&command.arguments) else {
        return true;
    };
    let known_mutation = matches!(
        command.operation.as_str(),
        "cp" | "install"
            | "ln"
            | "mv"
            | "rm"
            | "rmdir"
            | "unlink"
            | "touch"
            | "mkdir"
            | "truncate"
            | "tee"
            | "chmod"
            | "chown"
            | "chgrp"
            | "dd"
            | "sqlite3"
    );
    if !known_mutation {
        return false;
    }
    let mutation = targeted_mutation_operands(&command.operation, &arguments);
    let targets =
        if matches!(command.operation.as_str(), "cp" | "install" | "ln") && !mutation.exact {
            shell_operands(&arguments)
        } else {
            mutation.targets
        };
    let exact_cwd = apply_filesystem_cwd_overrides(None, &command.cwd_overrides)
        .filter(|cwd| cwd.is_absolute());
    targets.into_iter().any(|target| {
        mutation_target_may_reach_core_from_unknown(target, exact_cwd.as_deref(), project_root)
    })
}

fn mutation_target_may_reach_core_from_unknown(
    target: &str,
    exact_cwd: Option<&Path>,
    project_root: &Path,
) -> bool {
    let target = resolved_path(&decode_shell_token(target), exact_cwd, true);
    if target.is_absolute() {
        candidate_may_reach_managed_scope(target.to_string_lossy().as_ref(), project_root)
    } else {
        true
    }
}

fn resolved_target_string(target: &str, workdir: Option<&Path>) -> String {
    resolved_path(target, workdir, true)
        .to_string_lossy()
        .into_owned()
}

fn conservative_shell_targets(command: &str, workdir: Option<&Path>) -> Vec<String> {
    let mut targets = vec![command.to_owned()];
    targets.extend(command.split_whitespace().map(|target| {
        let decoded = decode_shell_token(target);
        if decoded.chars().next().is_some_and(|character| {
            matches!(character, '$' | '*' | '?' | '[' | '{') || character == '\x60'
        }) {
            target.to_owned()
        } else {
            resolved_target_string(target, workdir)
        }
    }));
    if let Some(workdir) = workdir {
        targets.push(workdir.to_string_lossy().into_owned());
    }
    targets
}

fn redirection_destinations(command: &str) -> Vec<&str> {
    let bytes = command.as_bytes();
    let mut destinations = Vec::new();
    let mut index = 0;
    let mut quote = None;
    while index < bytes.len() {
        match (quote, bytes[index]) {
            (Some(current), character) if character == current => {
                quote = None;
                index += 1;
            }
            (Some(b'"'), b'\\') if index + 1 < bytes.len() => index += 2,
            (Some(_), _) => index += 1,
            (None, b'\'' | b'"') => {
                quote = Some(bytes[index]);
                index += 1;
            }
            (None, b'\\') if index + 1 < bytes.len() => index += 2,
            (None, b'>') => {
                index += 1;
                if bytes.get(index) == Some(&b'>') {
                    index += 1;
                }
                while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
                    index += 1;
                }
                let descriptor_redirection = bytes.get(index) == Some(&b'&');
                if descriptor_redirection {
                    index += 1;
                    while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
                        index += 1;
                    }
                }
                let quoted = bytes
                    .get(index)
                    .copied()
                    .filter(|value| matches!(value, b'\'' | b'"'));
                if let Some(delimiter) = quoted {
                    index += 1;
                    let start = index;
                    while index < bytes.len() && bytes[index] != delimiter {
                        if delimiter == b'"' && bytes[index] == b'\\' && index + 1 < bytes.len() {
                            index += 2;
                        } else {
                            index += 1;
                        }
                    }
                    if start < index {
                        let target = &command[start..index];
                        if !descriptor_redirection || !is_file_descriptor_target(target) {
                            destinations.push(target);
                        }
                    }
                    index = (index + 1).min(bytes.len());
                } else {
                    let start = index;
                    while index < bytes.len()
                        && !bytes[index].is_ascii_whitespace()
                        && !matches!(bytes[index], b';' | b'|' | b'&' | b'<' | b'>')
                    {
                        index += 1;
                    }
                    if start < index {
                        let target = &command[start..index];
                        if !descriptor_redirection || !is_file_descriptor_target(target) {
                            destinations.push(target);
                        }
                    }
                }
            }
            (None, _) => index += 1,
        }
    }
    destinations
}

fn is_file_descriptor_target(target: &str) -> bool {
    target == "-" || (!target.is_empty() && target.bytes().all(|byte| byte.is_ascii_digit()))
}

struct SimpleCommandSegment<'command> {
    command: &'command str,
    separator_after: ShellSeparator,
}

#[derive(Clone, Copy)]
enum ShellSeparator {
    And,
    Or,
    Sequence,
    Pipeline,
    Background,
    End,
}

fn simple_command_segments(command: &str) -> Option<Vec<SimpleCommandSegment<'_>>> {
    let bytes = command.as_bytes();
    let mut segments: Vec<SimpleCommandSegment<'_>> = Vec::new();
    let mut start = 0;
    let mut index = 0;
    let mut quote = None;
    while index < bytes.len() {
        match (quote, bytes[index]) {
            (Some(current), character) if character == current => {
                quote = None;
                index += 1;
            }
            (Some(b'\''), _) => index += 1,
            (Some(b'"'), b'\\') if index + 1 < bytes.len() => index += 2,
            (Some(_), _) => index += 1,
            (None, b'\'' | b'"') => {
                quote = Some(bytes[index]);
                index += 1;
            }
            (None, b'\\') if index + 1 < bytes.len() => index += 2,
            (None, b'&')
                if bytes.get(index + 1) == Some(&b'>')
                    || (index > 0 && bytes[index - 1] == b'>') =>
            {
                index += 1;
            }
            (None, b'|' | b';' | b'&' | b'\n' | b'\r') => {
                let separator_length = if (bytes[index] == b'|'
                    && bytes.get(index + 1) == Some(&b'|'))
                    || (bytes[index] == b'&' && bytes.get(index + 1) == Some(&b'&'))
                    || (bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n'))
                {
                    2
                } else {
                    1
                };
                let segment = command[start..index].trim();
                if segment.is_empty() {
                    if matches!(bytes[index], b'\n' | b'\r') {
                        start = index + separator_length;
                        index += separator_length;
                        continue;
                    }
                    return None;
                }
                let separator_after = match (bytes[index], bytes.get(index + 1)) {
                    (b'&', Some(b'&')) => ShellSeparator::And,
                    (b'&', _) => ShellSeparator::Background,
                    (b'|', Some(b'|')) => ShellSeparator::Or,
                    (b'|', _) => ShellSeparator::Pipeline,
                    (b';' | b'\n' | b'\r', _) => ShellSeparator::Sequence,
                    _ => return None,
                };
                segments.push(SimpleCommandSegment {
                    command: segment,
                    separator_after,
                });
                start = index + separator_length;
                index += separator_length;
            }
            (None, _) => index += 1,
        }
    }
    if quote.is_some() {
        return None;
    }
    let segment = command[start..].trim();
    if segment.is_empty() {
        if segments.last().is_some_and(|segment| {
            matches!(
                segment.separator_after,
                ShellSeparator::Sequence | ShellSeparator::Background
            )
        }) {
            return Some(segments);
        }
        return None;
    }
    segments.push(SimpleCommandSegment {
        command: segment,
        separator_after: ShellSeparator::End,
    });
    Some(segments)
}

struct SimpleCommandPipeline<'command> {
    commands: Vec<&'command str>,
    separator_after: ShellSeparator,
}

fn simple_command_pipelines(command: &str) -> Option<Vec<SimpleCommandPipeline<'_>>> {
    let segments = simple_command_segments(command)?;
    let mut pipelines = Vec::new();
    let mut commands = Vec::new();
    for segment in segments {
        commands.push(segment.command);
        if !matches!(segment.separator_after, ShellSeparator::Pipeline) {
            pipelines.push(SimpleCommandPipeline {
                commands,
                separator_after: segment.separator_after,
            });
            commands = Vec::new();
        }
    }
    if commands.is_empty() {
        Some(pipelines)
    } else {
        None
    }
}

fn literal_cd_target(segment: &str, cwd: Option<&Path>) -> Option<PathBuf> {
    let LiteralShellCommandParse::Exec(command) = literal_shell_command(segment) else {
        return None;
    };
    if command.operation != "cd" || !command.cwd_changing_shell_builtin {
        return None;
    }
    let arguments = static_arguments(&command.arguments)?;
    let mut index = 0;
    let mut physical = false;
    while let Some(argument) = arguments.get(index) {
        if *argument == "--" {
            index += 1;
            break;
        }
        let Some(options) = argument
            .strip_prefix('-')
            .filter(|options| !options.is_empty())
        else {
            break;
        };
        if !options.chars().all(|option| matches!(option, 'L' | 'P')) {
            return None;
        }
        for option in options.chars() {
            physical = option == 'P';
        }
        index += 1;
    }
    let [target] = arguments.get(index..)? else {
        return None;
    };
    let target = *target;
    if target.is_empty() || target == "-" || target.starts_with('~') {
        return None;
    }
    let effective_cwd = apply_cwd_overrides(cwd, &command.cwd_overrides);
    if !physical {
        return Some(normalized_path(target, effective_cwd.as_deref()));
    }
    let target = Path::new(target);
    let target = if target.is_absolute() {
        target.to_path_buf()
    } else {
        effective_cwd?.join(target)
    };
    fs::canonicalize(target)
        .ok()
        .filter(|target| target.is_dir())
}

const MAX_SEQUENTIAL_CWD_CANDIDATES: usize = 64;

#[derive(Clone)]
struct CwdCandidates {
    known: Vec<Option<PathBuf>>,
    ambiguous: bool,
}

impl CwdCandidates {
    fn empty() -> Self {
        Self {
            known: Vec::new(),
            ambiguous: false,
        }
    }

    fn new(cwd: Option<PathBuf>) -> Self {
        let mut result = Self::empty();
        push_unique_cwd(&mut result, cwd);
        result
    }

    fn ambiguous() -> Self {
        Self {
            known: Vec::new(),
            ambiguous: true,
        }
    }

    fn merge(&mut self, other: &Self) {
        if self.ambiguous {
            return;
        }
        if other.ambiguous {
            self.known.clear();
            self.ambiguous = true;
            return;
        }
        for cwd in &other.known {
            push_unique_cwd(self, cwd.clone());
        }
    }
}

#[derive(Clone)]
struct ShellBranchCwds {
    success: CwdCandidates,
    failure: CwdCandidates,
}

impl ShellBranchCwds {
    fn all(&self) -> CwdCandidates {
        let mut all = self.success.clone();
        all.merge(&self.failure);
        all
    }
}

struct ShellPipelineExecution<'command> {
    commands: Vec<&'command str>,
    cwds: CwdCandidates,
}

struct ShellExecutionPlan<'command> {
    executions: Vec<ShellPipelineExecution<'command>>,
    bound_roots: Vec<PathBuf>,
}

fn shell_execution_plan(
    command: &str,
    cwd: Option<PathBuf>,
    ambiguous_cwd: bool,
) -> Option<ShellExecutionPlan<'_>> {
    let pipelines = simple_command_pipelines(command)?;
    let initial = if ambiguous_cwd {
        CwdCandidates::ambiguous()
    } else {
        CwdCandidates::new(cwd)
    };
    let mut list_entry = initial.clone();
    let mut previous = None::<ShellBranchCwds>;
    let mut previous_separator = None::<ShellSeparator>;
    let mut executions = Vec::new();
    let mut bound_roots = Vec::new();
    record_candidate_bound_roots(&mut bound_roots, &initial);

    for pipeline in pipelines {
        let (inputs, bypass_success, bypass_failure) = match (previous.take(), previous_separator) {
            (None, None) => (
                initial.clone(),
                CwdCandidates::empty(),
                CwdCandidates::empty(),
            ),
            (Some(branches), Some(ShellSeparator::And)) => {
                (branches.success, CwdCandidates::empty(), branches.failure)
            }
            (Some(branches), Some(ShellSeparator::Or)) => {
                let inputs = branches.all();
                (inputs, branches.success, CwdCandidates::empty())
            }
            (Some(branches), Some(ShellSeparator::Sequence)) => {
                let inputs = branches.all();
                list_entry = inputs.clone();
                (inputs, CwdCandidates::empty(), CwdCandidates::empty())
            }
            (Some(_), Some(ShellSeparator::Background)) => (
                list_entry.clone(),
                CwdCandidates::empty(),
                CwdCandidates::empty(),
            ),
            (Some(_), Some(ShellSeparator::End | ShellSeparator::Pipeline))
            | (None, Some(_))
            | (Some(_), None) => return None,
        };
        record_candidate_bound_roots(&mut bound_roots, &inputs);
        let mut outcomes = pipeline_cwd_outcomes(&pipeline.commands, &inputs, &mut bound_roots);
        outcomes.success.merge(&bypass_success);
        outcomes.failure.merge(&bypass_failure);
        executions.push(ShellPipelineExecution {
            commands: pipeline.commands,
            cwds: inputs,
        });
        previous = Some(outcomes);
        previous_separator = Some(pipeline.separator_after);
    }

    Some(ShellExecutionPlan {
        executions,
        bound_roots,
    })
}

fn pipeline_cwd_outcomes(
    commands: &[&str],
    inputs: &CwdCandidates,
    bound_roots: &mut Vec<PathBuf>,
) -> ShellBranchCwds {
    if commands.len() != 1 {
        return ShellBranchCwds {
            success: inputs.clone(),
            failure: inputs.clone(),
        };
    }

    let command = commands[0];
    let mut success = CwdCandidates::empty();
    let mut failure = CwdCandidates::empty();
    for cwd in &inputs.known {
        push_unique_cwd(&mut failure, cwd.clone());
        if let Some(changed) = literal_cd_target(command, cwd.as_deref()) {
            push_bound_project_root(bound_roots, &changed);
            push_unique_cwd(&mut success, Some(changed));
        } else {
            push_unique_cwd(&mut success, cwd.clone());
        }
    }
    if inputs.ambiguous {
        failure = CwdCandidates::ambiguous();
        success = literal_cd_target(command, None).map_or_else(CwdCandidates::ambiguous, |cwd| {
            if cwd.is_absolute() {
                push_bound_project_root(bound_roots, &cwd);
                CwdCandidates::new(Some(cwd))
            } else {
                CwdCandidates::ambiguous()
            }
        });
    }
    ShellBranchCwds { success, failure }
}

fn record_candidate_bound_roots(roots: &mut Vec<PathBuf>, candidates: &CwdCandidates) {
    for cwd in &candidates.known {
        if let Some(cwd) = cwd.as_deref() {
            push_bound_project_root(roots, cwd);
        }
    }
}

fn push_unique_cwd(values: &mut CwdCandidates, value: Option<PathBuf>) {
    if values.ambiguous || values.known.contains(&value) {
        return;
    }
    if values.known.len() == MAX_SEQUENTIAL_CWD_CANDIDATES {
        values.known.clear();
        values.ambiguous = true;
    } else {
        values.known.push(value);
    }
}

fn has_active_shell_substitution(command: &str) -> bool {
    let bytes = command.as_bytes();
    let mut index = 0;
    let mut quote = None;
    while index < bytes.len() {
        match (quote, bytes[index]) {
            (Some(current), character) if character == current => {
                quote = None;
                index += 1;
            }
            (Some(b'\''), _) => index += 1,
            (Some(b'"'), b'\\') if index + 1 < bytes.len() => index += 2,
            (Some(_), b'`') | (None, b'`') => return true,
            (Some(b'"'), b'$') | (None, b'$') if bytes.get(index + 1) == Some(&b'(') => {
                return true;
            }
            (None, b'<' | b'>') if bytes.get(index + 1) == Some(&b'(') => return true,
            (Some(_), _) => index += 1,
            (None, b'\'' | b'"') => {
                quote = Some(bytes[index]);
                index += 1;
            }
            (None, b'\\') if index + 1 < bytes.len() => index += 2,
            (None, _) => index += 1,
        }
    }
    false
}

struct MutationOperandTargets<'value> {
    targets: Vec<&'value str>,
    exact: bool,
}

struct CopyLikeLayout<'value> {
    operands: Vec<&'value str>,
    target_directory: Option<&'value str>,
    install_directory_mode: bool,
    source_alias_mode: bool,
}

#[derive(Clone, Copy)]
enum CopyLikeOption {
    Flag,
    Value,
    TargetDirectory,
    InstallDirectory,
    SourceAlias,
}

fn targeted_mutation_operands<'value>(
    operation: &str,
    arguments: &[&'value str],
) -> MutationOperandTargets<'value> {
    let operands = shell_operands(arguments);
    match operation {
        "cp" | "install" => {
            let Some(layout) = copy_like_layout(operation, arguments) else {
                return MutationOperandTargets {
                    targets: Vec::new(),
                    exact: false,
                };
            };
            let source_alias_mode = layout.source_alias_mode;
            let targets = if layout.install_directory_mode {
                layout.operands
            } else if let Some(target_directory) = layout.target_directory {
                vec![target_directory]
            } else if layout.operands.len() >= 2 {
                layout.operands.last().copied().into_iter().collect()
            } else {
                return MutationOperandTargets {
                    targets: Vec::new(),
                    exact: false,
                };
            };
            MutationOperandTargets {
                targets,
                exact: !source_alias_mode,
            }
        }
        "ln" => {
            let targets = copy_like_layout(operation, arguments)
                .and_then(|layout| {
                    layout.target_directory.or_else(|| {
                        (layout.operands.len() >= 2)
                            .then(|| layout.operands.last().copied())
                            .flatten()
                    })
                })
                .into_iter()
                .collect();
            MutationOperandTargets {
                targets,
                // The source is also security-sensitive because a link aliases protected state.
                exact: false,
            }
        }
        "mv" | "rm" | "rmdir" | "unlink" | "touch" | "mkdir" | "truncate" | "tee" => {
            MutationOperandTargets {
                targets: operands.clone(),
                exact: false,
            }
        }
        "chmod" | "chown" | "chgrp" => {
            let Some(layout) = metadata_layout(operation, arguments) else {
                return MutationOperandTargets {
                    targets: Vec::new(),
                    exact: false,
                };
            };
            MutationOperandTargets {
                targets: layout.targets,
                exact: true,
            }
        }
        "dd" if arguments
            .iter()
            .all(|argument| !argument.starts_with('-') && argument.contains('=')) =>
        {
            MutationOperandTargets {
                targets: arguments
                    .iter()
                    .copied()
                    .filter_map(|operand| operand.strip_prefix("of="))
                    .collect(),
                exact: true,
            }
        }
        "sqlite3" => MutationOperandTargets {
            targets: operands.first().copied().into_iter().collect(),
            exact: false,
        },
        _ => MutationOperandTargets {
            targets: Vec::new(),
            exact: false,
        },
    }
}

fn copy_like_layout<'value>(
    operation: &str,
    arguments: &[&'value str],
) -> Option<CopyLikeLayout<'value>> {
    let mut layout = CopyLikeLayout {
        operands: Vec::new(),
        target_directory: None,
        install_directory_mode: false,
        source_alias_mode: false,
    };
    let mut options = true;
    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index];
        if options && argument == "--" {
            options = false;
            index += 1;
            continue;
        }
        if options && argument.starts_with("--") {
            let (name, attached_value) = argument
                .split_once('=')
                .map_or((argument, None), |(name, value)| (name, Some(value)));
            let option = copy_like_long_option(operation, name)?;
            let value = match option {
                CopyLikeOption::Flag
                | CopyLikeOption::InstallDirectory
                | CopyLikeOption::SourceAlias => {
                    if attached_value.is_some() {
                        return None;
                    }
                    None
                }
                CopyLikeOption::Value | CopyLikeOption::TargetDirectory => {
                    if let Some(value) = attached_value {
                        (!value.is_empty()).then_some(value)?
                    } else {
                        index += 1;
                        arguments.get(index).copied()?
                    }
                    .into()
                }
            };
            apply_copy_like_option(&mut layout, option, value)?;
            index += 1;
            continue;
        }
        if options && argument.starts_with('-') && argument != "-" {
            let cluster = argument.strip_prefix('-')?;
            if !cluster.is_ascii() {
                return None;
            }
            let bytes = cluster.as_bytes();
            let mut option_index = 0;
            while option_index < bytes.len() {
                let option = copy_like_short_option(operation, bytes[option_index] as char)?;
                let value = match option {
                    CopyLikeOption::Flag
                    | CopyLikeOption::InstallDirectory
                    | CopyLikeOption::SourceAlias => None,
                    CopyLikeOption::Value | CopyLikeOption::TargetDirectory => {
                        let attached = &cluster[option_index + 1..];
                        let value = if attached.is_empty() {
                            index += 1;
                            arguments.get(index).copied()?
                        } else {
                            attached
                        };
                        option_index = bytes.len();
                        Some(value)
                    }
                };
                apply_copy_like_option(&mut layout, option, value)?;
                option_index += 1;
            }
            index += 1;
            continue;
        }
        layout.operands.push(argument);
        index += 1;
    }

    if layout.install_directory_mode && layout.target_directory.is_some() {
        return None;
    }
    Some(layout)
}

fn apply_copy_like_option<'value>(
    layout: &mut CopyLikeLayout<'value>,
    option: CopyLikeOption,
    value: Option<&'value str>,
) -> Option<()> {
    match option {
        CopyLikeOption::Flag | CopyLikeOption::Value => {}
        CopyLikeOption::InstallDirectory => layout.install_directory_mode = true,
        CopyLikeOption::SourceAlias => layout.source_alias_mode = true,
        CopyLikeOption::TargetDirectory => {
            if layout.target_directory.replace(value?).is_some() {
                return None;
            }
        }
    }
    Some(())
}

fn copy_like_short_option(operation: &str, option: char) -> Option<CopyLikeOption> {
    let kind = match (operation, option) {
        (_, 't') => CopyLikeOption::TargetDirectory,
        ("cp" | "ln", 'S') | ("install", 'g' | 'm' | 'o' | 'S') => CopyLikeOption::Value,
        ("install", 'd') => CopyLikeOption::InstallDirectory,
        ("cp", 'l' | 's') => CopyLikeOption::SourceAlias,
        (
            "cp",
            'a' | 'b' | 'd' | 'f' | 'H' | 'i' | 'L' | 'n' | 'P' | 'p' | 'R' | 'r' | 'T' | 'u' | 'v'
            | 'x' | 'Z',
        )
        | ("install", 'b' | 'c' | 'C' | 'D' | 'p' | 's' | 'T' | 'v' | 'Z')
        | ("ln", 'b' | 'd' | 'F' | 'f' | 'i' | 'L' | 'n' | 'P' | 'r' | 's' | 'T' | 'v') => {
            CopyLikeOption::Flag
        }
        _ => return None,
    };
    Some(kind)
}

fn copy_like_long_option(operation: &str, option: &str) -> Option<CopyLikeOption> {
    let kind = match (operation, option) {
        (_, "--target-directory") => CopyLikeOption::TargetDirectory,
        ("cp" | "ln", "--suffix")
        | ("install", "--group" | "--mode" | "--owner" | "--strip-program" | "--suffix") => {
            CopyLikeOption::Value
        }
        ("install", "--directory") => CopyLikeOption::InstallDirectory,
        ("cp", "--link" | "--symbolic-link") => CopyLikeOption::SourceAlias,
        (
            "cp",
            "--archive"
            | "--attributes-only"
            | "--backup"
            | "--copy-contents"
            | "--dereference"
            | "--force"
            | "--interactive"
            | "--no-clobber"
            | "--no-dereference"
            | "--no-target-directory"
            | "--one-file-system"
            | "--parents"
            | "--preserve"
            | "--recursive"
            | "--remove-destination"
            | "--strip-trailing-slashes"
            | "--update"
            | "--verbose",
        )
        | (
            "install",
            "--backup"
            | "--compare"
            | "--no-target-directory"
            | "--preserve-context"
            | "--preserve-timestamps"
            | "--strip"
            | "--verbose",
        )
        | (
            "ln",
            "--backup"
            | "--directory"
            | "--force"
            | "--interactive"
            | "--logical"
            | "--no-dereference"
            | "--no-target-directory"
            | "--physical"
            | "--relative"
            | "--symbolic"
            | "--verbose",
        ) => CopyLikeOption::Flag,
        _ => return None,
    };
    Some(kind)
}

fn shell_command_is_read_only(operation: &str) -> bool {
    matches!(
        operation,
        "cat"
            | "cd"
            | "cmp"
            | "du"
            | "echo"
            | "file"
            | "grep"
            | "head"
            | "ls"
            | "md5sum"
            | "printf"
            | "pwd"
            | "readlink"
            | "realpath"
            | "rg"
            | "sha256sum"
            | "shasum"
            | "stat"
            | "tail"
            | "wc"
    )
}

struct CommandLocation {
    index: usize,
    cwd_overrides: Vec<String>,
    preserves_shell_builtin_context: bool,
}

enum CommandResolution {
    Exec(CommandLocation),
    NoExec,
    Incomplete,
    Opaque,
}

enum PrefixResolution {
    Exec {
        index: usize,
        cwd_overrides: Vec<String>,
    },
    NoExec,
    Incomplete,
    Opaque,
}

fn shell_command_location(words: &[&str]) -> CommandResolution {
    let mut index = 0;
    while index < words.len() && is_shell_assignment(words[index]) {
        index += 1;
    }
    let mut cwd_overrides = Vec::new();
    let mut preserves_shell_builtin_context = true;
    while index < words.len() {
        let prefix_preserves_shell_builtin_context = words[index] == "command";
        let command = words[index]
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(words[index]);
        let resolution = match command {
            "env" => env_command_resolution(words, index + 1),
            "sudo" => sudo_command_resolution(words, index + 1),
            "command" => command_builtin_resolution(words, index + 1),
            "nohup" => nohup_command_resolution(words, index + 1),
            _ => {
                return CommandResolution::Exec(CommandLocation {
                    index,
                    cwd_overrides,
                    preserves_shell_builtin_context,
                });
            }
        };
        match resolution {
            PrefixResolution::Exec {
                index: child_index,
                cwd_overrides: child_overrides,
            } => {
                index = child_index;
                cwd_overrides.extend(child_overrides);
                preserves_shell_builtin_context &= prefix_preserves_shell_builtin_context;
            }
            PrefixResolution::NoExec => return CommandResolution::NoExec,
            PrefixResolution::Incomplete => return CommandResolution::Incomplete,
            PrefixResolution::Opaque => return CommandResolution::Opaque,
        }
    }
    CommandResolution::Opaque
}

fn env_command_resolution(words: &[&str], mut index: usize) -> PrefixResolution {
    let mut cwd_overrides = Vec::new();
    while let Some(argument) = words.get(index).copied() {
        if argument == "--" {
            index += 1;
            break;
        }
        if argument == "-" {
            index += 1;
            continue;
        }
        if !argument.starts_with('-') {
            break;
        }
        if argument.starts_with("--") {
            let (name, attached_value) = argument
                .split_once('=')
                .map_or((argument, None), |(name, value)| (name, Some(value)));
            match name {
                "--unset" => {
                    if consume_long_option_value(words, &mut index, attached_value).is_none() {
                        return PrefixResolution::Opaque;
                    }
                }
                "--chdir" => {
                    let Some(value) = consume_long_option_value(words, &mut index, attached_value)
                    else {
                        return PrefixResolution::Opaque;
                    };
                    cwd_overrides.push(value.to_owned());
                }
                "--ignore-environment" | "--null" | "--debug" => {
                    if attached_value.is_some() {
                        return PrefixResolution::Opaque;
                    }
                }
                "--split-string" => return PrefixResolution::Opaque,
                _ => return PrefixResolution::Opaque,
            }
            index += 1;
            continue;
        }

        let Some(cluster) = argument.strip_prefix('-') else {
            return PrefixResolution::Opaque;
        };
        let mut option_index = 0;
        while option_index < cluster.len() {
            match cluster.as_bytes()[option_index] as char {
                'i' | '0' | 'v' => option_index += 1,
                option @ ('u' | 'C') => {
                    let value = if option_index + 1 == cluster.len() {
                        index += 1;
                        let Some(value) = words.get(index).copied() else {
                            return PrefixResolution::Opaque;
                        };
                        value
                    } else {
                        &cluster[option_index + 1..]
                    };
                    if option == 'C' {
                        cwd_overrides.push(value.to_owned());
                    }
                    option_index = cluster.len();
                }
                'S' => return PrefixResolution::Opaque,
                _ => return PrefixResolution::Opaque,
            }
        }
        index += 1;
    }
    while words.get(index).is_some_and(|word| env_assignment(word)) {
        index += 1;
    }
    if index < words.len() {
        PrefixResolution::Exec {
            index,
            cwd_overrides,
        }
    } else {
        PrefixResolution::Incomplete
    }
}

fn sudo_command_resolution(words: &[&str], mut index: usize) -> PrefixResolution {
    let mut cwd_overrides = Vec::new();
    while let Some(argument) = words.get(index).copied() {
        if argument == "--" {
            index += 1;
            break;
        }
        if is_shell_assignment(argument) {
            index += 1;
            continue;
        }
        if !argument.starts_with('-') || argument == "-" {
            break;
        }
        if argument.starts_with("--") {
            let (name, attached_value) = argument
                .split_once('=')
                .map_or((argument, None), |(name, value)| (name, Some(value)));
            match name {
                "--chroot" => return PrefixResolution::Opaque,
                "--chdir" => {
                    let Some(value) = consume_long_option_value(words, &mut index, attached_value)
                    else {
                        return PrefixResolution::Opaque;
                    };
                    cwd_overrides.push(value.to_owned());
                }
                "--close-from" | "--group" | "--host" | "--prompt" | "--role" | "--type"
                | "--command-timeout" | "--user" => {
                    if consume_long_option_value(words, &mut index, attached_value).is_none() {
                        return PrefixResolution::Opaque;
                    }
                }
                "--edit" | "--help" | "--list" | "--other-user" | "--remove-timestamp"
                | "--validate" | "--version" => return PrefixResolution::NoExec,
                "--login" | "--shell" => return PrefixResolution::Opaque,
                "--askpass" | "--background" | "--bell" | "--preserve-env"
                | "--preserve-groups" | "--set-home" | "--stdin" | "--non-interactive"
                | "--reset-timestamp" => {
                    if attached_value.is_some()
                        && !(name == "--preserve-env"
                            && attached_value.is_some_and(|value| !value.is_empty()))
                    {
                        return PrefixResolution::Opaque;
                    }
                }
                _ => return PrefixResolution::Opaque,
            }
            index += 1;
            continue;
        }

        let Some(cluster) = argument.strip_prefix('-') else {
            return PrefixResolution::Opaque;
        };
        let mut option_index = 0;
        while option_index < cluster.len() {
            match cluster.as_bytes()[option_index] as char {
                'A' | 'b' | 'B' | 'E' | 'H' | 'k' | 'n' | 'P' | 'S' => option_index += 1,
                'e' | 'K' | 'l' | 'U' | 'V' | 'v' => return PrefixResolution::NoExec,
                'i' | 's' | 'R' => return PrefixResolution::Opaque,
                option @ ('C' | 'D' | 'g' | 'h' | 'p' | 'r' | 't' | 'T' | 'u') => {
                    let value = if option_index + 1 == cluster.len() {
                        index += 1;
                        let Some(value) = words.get(index).copied() else {
                            return PrefixResolution::Opaque;
                        };
                        value
                    } else {
                        &cluster[option_index + 1..]
                    };
                    if option == 'D' {
                        cwd_overrides.push(value.to_owned());
                    }
                    option_index = cluster.len();
                }
                _ => return PrefixResolution::Opaque,
            }
        }
        index += 1;
    }
    if index < words.len() {
        PrefixResolution::Exec {
            index,
            cwd_overrides,
        }
    } else {
        PrefixResolution::Incomplete
    }
}

fn env_assignment(word: &str) -> bool {
    word.split_once('=')
        .is_some_and(|(name, _)| !name.is_empty())
}

fn is_shell_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn command_builtin_resolution(words: &[&str], mut index: usize) -> PrefixResolution {
    while let Some(argument) = words.get(index).copied() {
        match argument {
            "--" => {
                index += 1;
                break;
            }
            "-p" => index += 1,
            "-v" | "-V" => return PrefixResolution::NoExec,
            argument if argument.starts_with('-') => return PrefixResolution::Opaque,
            _ => break,
        }
    }
    if index < words.len() {
        PrefixResolution::Exec {
            index,
            cwd_overrides: Vec::new(),
        }
    } else {
        PrefixResolution::Incomplete
    }
}

fn nohup_command_resolution(words: &[&str], mut index: usize) -> PrefixResolution {
    if words.get(index) == Some(&"--") {
        index += 1;
    } else if words
        .get(index)
        .is_some_and(|argument| argument.starts_with('-'))
    {
        return PrefixResolution::Opaque;
    }
    if index < words.len() {
        PrefixResolution::Exec {
            index,
            cwd_overrides: Vec::new(),
        }
    } else {
        PrefixResolution::Incomplete
    }
}

fn shell_operands<'value>(words: &[&'value str]) -> Vec<&'value str> {
    let mut options = true;
    words
        .iter()
        .filter_map(|word| {
            if options && *word == "--" {
                options = false;
                None
            } else if options && word.starts_with('-') {
                None
            } else {
                Some(*word)
            }
        })
        .collect()
}

fn trim_shell_token(value: &str) -> &str {
    value.trim_matches(|character| matches!(character, '\'' | '"' | '`'))
}

fn resolve_shell_target(target: &str, workdir: Option<&str>) -> String {
    let target = trim_shell_token(target);
    let target_path = Path::new(target);
    let path = match workdir.filter(|_| !target_path.is_absolute()) {
        Some(workdir) => Path::new(workdir).join(target_path),
        None => target_path.to_path_buf(),
    };
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => match resolved.components().next_back() {
                Some(std::path::Component::Normal(_)) => {
                    resolved.pop();
                }
                Some(std::path::Component::ParentDir) | None => resolved.push(".."),
                Some(
                    std::path::Component::Prefix(_)
                    | std::path::Component::RootDir
                    | std::path::Component::CurDir,
                ) => {}
            },
            component => resolved.push(component.as_os_str()),
        }
    }
    resolved.to_string_lossy().into_owned()
}

const CORE_STATE_PATHS: [&str; 8] = [
    ".mobius",
    ".mobius/.gitignore",
    ".mobius/mobius.sqlite3",
    ".mobius/mobius.sqlite3-wal",
    ".mobius/mobius.sqlite3-shm",
    ".mobius/artifacts",
    ".mobius/artifacts/blobs",
    ".mobius/artifacts/staging",
];

fn targets_core_state(strings: &[String], project_root: Option<&Path>) -> bool {
    strings
        .iter()
        .any(|value| target_may_reach_core_state(value, project_root))
}

fn target_may_reach_core_state(value: &str, project_root: Option<&Path>) -> bool {
    let decoded = decode_shell_token(value);
    let candidates = std::iter::once(decoded.as_str())
        .chain(decoded.split_once('=').map(|(_, candidate)| candidate));
    candidates.into_iter().any(|candidate| match project_root {
        Some(project_root) => candidate_may_reach_managed_scope(candidate, project_root),
        None => {
            let normalized = resolve_shell_target(candidate, None);
            is_literal_core_state_target(&normalized)
                || static_pattern_prefix_may_reach_core_state(&normalized)
        }
    })
}

fn candidate_may_reach_managed_scope(candidate: &str, project_root: &Path) -> bool {
    let candidate = candidate.trim_matches(|character| {
        matches!(
            character,
            '\'' | '"' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';'
        )
    });
    if let Some(pattern_index) = candidate.char_indices().find_map(|(index, character)| {
        (matches!(character, '*' | '?' | '[' | '{' | '$') || character == '\x60').then_some(index)
    }) {
        if pattern_index == 0 {
            return false;
        }
        let prefix = normalized_path(&candidate[..pattern_index], Some(project_root));
        return static_path_prefix_intersects_managed_scope(&prefix, project_root);
    }

    let target = normalized_path(candidate, Some(project_root));
    path_is_managed_scope(&target, project_root)
}

fn path_is_managed_scope(target: &Path, project_root: &Path) -> bool {
    let mobius = project_root.join(".mobius");
    let artifacts = mobius.join("artifacts");
    let blobs = artifacts.join("blobs");
    let staging = artifacts.join("staging");
    target == mobius
        || target == artifacts
        || target == mobius.join(".gitignore")
        || target == mobius.join("mobius.sqlite3")
        || target == mobius.join("mobius.sqlite3-wal")
        || target == mobius.join("mobius.sqlite3-shm")
        || target.starts_with(blobs)
        || target.starts_with(staging)
}

fn static_path_prefix_intersects_managed_scope(prefix: &Path, project_root: &Path) -> bool {
    let mobius = project_root.join(".mobius");
    let artifacts = mobius.join("artifacts");
    let blobs = artifacts.join("blobs");
    let staging = artifacts.join("staging");
    let protected = [
        mobius.clone(),
        artifacts,
        mobius.join(".gitignore"),
        mobius.join("mobius.sqlite3"),
        mobius.join("mobius.sqlite3-wal"),
        mobius.join("mobius.sqlite3-shm"),
        blobs.clone(),
        staging.clone(),
    ];
    let prefix_string = prefix.to_string_lossy();
    protected
        .iter()
        .any(|target| target.to_string_lossy().starts_with(prefix_string.as_ref()))
        || prefix.starts_with(blobs)
        || prefix.starts_with(staging)
}

fn is_literal_core_state_target(value: &str) -> bool {
    (contains_file_target(value, ".mobius/.gitignore")
        || contains_file_target(value, ".mobius/mobius.sqlite3")
        || contains_file_target(value, ".mobius/mobius.sqlite3-wal")
        || contains_file_target(value, ".mobius/mobius.sqlite3-shm")
        || contains_directory_target(value, ".mobius/artifacts/blobs")
        || contains_directory_target(value, ".mobius/artifacts/staging"))
        || mentions_managed_ancestor(value)
}

fn decode_shell_token(value: &str) -> String {
    let mut decoded = String::with_capacity(value.len());
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\'' | '"' => {}
            '$' if characters
                .peek()
                .is_some_and(|next| matches!(next, '\'' | '"')) => {}
            '\\' => {
                if let Some(escaped) = characters.next() {
                    decoded.push(escaped);
                }
            }
            character => decoded.push(character),
        }
    }
    decoded
}

fn static_pattern_prefix_may_reach_core_state(value: &str) -> bool {
    let Some(pattern_index) = value.char_indices().find_map(|(index, character)| {
        matches!(character, '*' | '?' | '[' | '{' | '$' | '`').then_some(index)
    }) else {
        return false;
    };
    let static_prefix = &value[..pattern_index];
    static_prefix.char_indices().any(|(index, character)| {
        if character != '.' {
            return false;
        }
        let boundary = index == 0
            || static_prefix[..index]
                .chars()
                .next_back()
                .is_some_and(|character| {
                    matches!(character, '/' | '\\') || is_path_delimiter(character)
                });
        boundary
            && CORE_STATE_PATHS
                .iter()
                .any(|protected| protected.starts_with(&static_prefix[index..]))
    })
}

fn contains_file_target(value: &str, target: &str) -> bool {
    value.match_indices(target).any(|(index, _)| {
        let prefix = &value[..index];
        let suffix = &value[index + target.len()..];
        prefix
            .chars()
            .next_back()
            .is_none_or(|character| matches!(character, '/' | '\\') || is_path_delimiter(character))
            && suffix.chars().next().is_none_or(is_path_delimiter)
    })
}

fn contains_directory_target(value: &str, target: &str) -> bool {
    value.match_indices(target).any(|(index, _)| {
        let prefix = &value[..index];
        let suffix = &value[index + target.len()..];
        prefix
            .chars()
            .next_back()
            .is_none_or(|character| matches!(character, '/' | '\\') || is_path_delimiter(character))
            && suffix.chars().next().is_none_or(|character| {
                matches!(character, '/' | '\\') || is_path_delimiter(character)
            })
    })
}

fn is_path_delimiter(character: char) -> bool {
    character.is_whitespace()
        || matches!(
            character,
            '\'' | '"' | '`' | ')' | ']' | '}' | ',' | ';' | ':' | '=' | '>' | '<' | '|'
        )
}

fn mentions_managed_ancestor(value: &str) -> bool {
    value.split_whitespace().any(|token| {
        let token = token.trim_matches(|character: char| {
            matches!(
                character,
                '\'' | '"' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';'
            )
        });
        let token = token.trim_end_matches(['/', '\\']);
        [".mobius", ".mobius/artifacts"].iter().any(|ancestor| {
            token == *ancestor
                || token.ends_with(&format!("/{ancestor}"))
                || token == format!("{ancestor}/*")
                || token.ends_with(&format!("/{ancestor}/*"))
        })
    })
}

fn stop_output(input: &StopInput) -> Option<Value> {
    stop_output_with(input, |objective_id| {
        objective_is_achieved(&input.cwd, objective_id)
    })
}

fn stop_output_with<F>(input: &StopInput, verify: F) -> Option<Value>
where
    F: FnOnce(&ObjectiveId) -> Result<bool, String>,
{
    if input.stop_hook_active {
        return None;
    }

    let claim = match completion_claim(input.last_assistant_message.as_deref().unwrap_or_default())
    {
        CompletionClaim::Absent => return None,
        CompletionClaim::Invalid(reason) => return Some(stop_block(reason)),
        CompletionClaim::Objective(objective_id) => objective_id,
    };

    match verify(&claim) {
        Ok(true) => None,
        Ok(false) => Some(stop_block(format!(
            "Mobius Objective `{}` is not in Achieved state; continue without the completion marker",
            claim.as_str()
        ))),
        Err(reason) => Some(stop_block(format!(
            "Mobius Objective `{}` completion could not be verified: {reason}",
            claim.as_str()
        ))),
    }
}

enum CompletionClaim {
    Absent,
    Invalid(String),
    Objective(ObjectiveId),
}

fn completion_claim(message: &str) -> CompletionClaim {
    let Some(line) = message.lines().rev().find(|line| !line.trim().is_empty()) else {
        return CompletionClaim::Absent;
    };
    let Some(objective_id) = line.strip_prefix(COMPLETION_MARKER) else {
        return CompletionClaim::Absent;
    };
    if objective_id.is_empty() || objective_id.trim() != objective_id {
        return CompletionClaim::Invalid(
            "the Mobius completion marker must end with one exact Objective identity".to_owned(),
        );
    }
    CompletionClaim::Objective(ObjectiveId::new(objective_id))
}

fn stop_block(reason: impl Into<String>) -> Value {
    json!({
        "decision": "block",
        "reason": reason.into()
    })
}

fn objective_is_achieved(cwd: &Path, objective_id: &ObjectiveId) -> Result<bool, String> {
    let project_root = find_bound_project_root(cwd)
        .ok_or_else(|| "no bound project was found from the current directory".to_owned())?;
    let allowed_roots = vec![project_root.clone()];
    let admitted = admit_project_root(&project_root, &allowed_roots)
        .map_err(|error| format!("project admission failed: {error}"))?;
    let storage_binding = SqliteStore::inspect_binding(&admitted)
        .map_err(|error| format!("project binding inspection failed: {error}"))?;
    let canonical_root = admitted.canonical_root().to_path_buf();
    let service = CoreService::new(vec![canonical_root.clone()]);
    let response = service
        .read(ReadRequest {
            binding: ServiceProjectBinding {
                project_root: canonical_root,
                project_id: storage_binding.project_id,
            },
            query: ReadQuery::Status {
                objective_id: Some(objective_id.clone()),
            },
        })
        .map_err(|error| error.to_string())?;

    let ReadResult::Status(status) = response.result else {
        return Err("Core returned a non-status response".to_owned());
    };
    Ok(matches!(
        status.objective_state,
        Some(ObjectiveState::Achieved { ref objective, .. }) if objective == objective_id
    ))
}

enum BoundDescendantScan {
    Clear,
    Bound,
    Indeterminate,
}

fn destructive_scope_contains_bound_project(
    scope: &Path,
    symlink_policy: DestructiveSymlinkPolicy,
) -> bool {
    !matches!(
        scan_bound_project_descendant(scope, symlink_policy),
        BoundDescendantScan::Clear
    )
}

fn scan_bound_project_descendant(
    scope: &Path,
    symlink_policy: DestructiveSymlinkPolicy,
) -> BoundDescendantScan {
    let metadata = match fs::symlink_metadata(scope) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return BoundDescendantScan::Clear;
        }
        Err(_) => return BoundDescendantScan::Indeterminate,
    };
    if metadata.file_type().is_symlink() {
        return match symlink_policy {
            DestructiveSymlinkPolicy::Skip => BoundDescendantScan::Clear,
            DestructiveSymlinkPolicy::RejectRoot | DestructiveSymlinkPolicy::RejectAny => {
                BoundDescendantScan::Indeterminate
            }
        };
    }
    if !metadata.is_dir() {
        return BoundDescendantScan::Clear;
    }

    let mut pending = vec![scope.to_path_buf()];
    while let Some(directory) = pending.pop() {
        match binding_marker_at(&directory) {
            BoundDescendantScan::Bound => return BoundDescendantScan::Bound,
            BoundDescendantScan::Indeterminate => return BoundDescendantScan::Indeterminate,
            BoundDescendantScan::Clear => {}
        }
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => return BoundDescendantScan::Indeterminate,
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => return BoundDescendantScan::Indeterminate,
            };
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => return BoundDescendantScan::Indeterminate,
            };
            if file_type.is_symlink() {
                if matches!(symlink_policy, DestructiveSymlinkPolicy::RejectAny) {
                    return BoundDescendantScan::Indeterminate;
                }
                continue;
            }
            if file_type.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    BoundDescendantScan::Clear
}

fn binding_marker_at(project_root: &Path) -> BoundDescendantScan {
    let state_directory = project_root.join(".mobius");
    let state_metadata = match fs::symlink_metadata(&state_directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return BoundDescendantScan::Clear;
        }
        Err(_) => return BoundDescendantScan::Indeterminate,
    };
    if state_metadata.file_type().is_symlink() || !state_metadata.is_dir() {
        return BoundDescendantScan::Clear;
    }
    match fs::symlink_metadata(state_directory.join("mobius.sqlite3")) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            BoundDescendantScan::Bound
        }
        Ok(_) => BoundDescendantScan::Clear,
        Err(error) if error.kind() == io::ErrorKind::NotFound => BoundDescendantScan::Clear,
        Err(_) => BoundDescendantScan::Indeterminate,
    }
}

fn find_bound_project_root(cwd: &Path) -> Option<PathBuf> {
    cwd.ancestors()
        .find(|candidate| matches!(binding_marker_at(candidate), BoundDescendantScan::Bound))
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_input(tool_name: &str, value: Value) -> PreToolUseInput {
        tool_input_at(tool_name, value, "/project")
    }

    fn tool_input_at(tool_name: &str, value: Value, cwd: &str) -> PreToolUseInput {
        PreToolUseInput {
            cwd: Some(PathBuf::from(cwd)),
            tool_name: tool_name.to_owned(),
            tool_input: value,
        }
    }

    fn pre_tool_use_output_bound(input: &PreToolUseInput, project_root: &str) -> Option<Value> {
        let targets = mutation_targets(input);
        pre_tool_use_decision(input, &targets, &[PathBuf::from(project_root)])
    }

    fn stop_input(message: &str) -> StopInput {
        StopInput {
            cwd: PathBuf::from("/unused"),
            last_assistant_message: Some(message.to_owned()),
            stop_hook_active: false,
        }
    }

    #[test]
    fn pre_tool_use_denies_direct_mutation_of_authoritative_state() {
        let denied = pre_tool_use_output(&tool_input(
            "exec_command",
            json!({"cmd": "rm -f .mobius/mobius.sqlite3-wal"}),
        ))
        .expect("authoritative state mutation must be denied");
        assert_eq!(denied["hookSpecificOutput"]["permissionDecision"], "deny");
        assert_eq!(denied["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert!(denied.get("continue").is_none());

        assert!(
            pre_tool_use_output(&tool_input(
                "apply_patch",
                json!({"command": "*** Update File: .mobius/artifacts/blobs/a"}),
            ))
            .is_some()
        );
        assert!(
            pre_tool_use_output(&tool_input(
                "mcp__filesystem__write_file",
                json!({"path": "/project/.mobius/artifacts/staging/a"}),
            ))
            .is_some()
        );
        assert!(
            pre_tool_use_output(&tool_input(
                "exec_command",
                json!({"cmd": "rm -f .mobius/.gitignore"}),
            ))
            .is_some(),
            "the self-ignore policy is Core-owned and must not be removable"
        );
    }

    #[test]
    fn apply_patch_checks_file_headers_instead_of_document_content() {
        let documentation_patch = concat!(
            "*** Begin Patch\n",
            "*** Update File: docs/storage.md\n",
            "@@\n",
            "+Core uses .mobius/mobius.sqlite3 and .mobius/artifacts/blobs.\n",
            "+*** Update File: .mobius/mobius.sqlite3\n",
            "*** End Patch",
        );
        assert!(
            pre_tool_use_output(&tool_input(
                "apply_patch",
                json!({"command": documentation_patch}),
            ))
            .is_none()
        );

        let mixed_patch = concat!(
            "*** Begin Patch\n",
            "*** Update File: docs/storage.md\n",
            "@@\n",
            "+ordinary documentation\n",
            "*** Update File: .mobius/mobius.sqlite3\n",
            "@@\n",
            "+tamper\n",
            "*** End Patch",
        );
        assert!(
            pre_tool_use_output(&tool_input("apply_patch", json!({"command": mixed_patch}),))
                .is_some()
        );

        assert!(
            pre_tool_use_output(&tool_input(
                "apply_patch",
                json!({"command": "*** Update File: .mobius/./mobius.sqlite3"}),
            ))
            .is_some()
        );
    }

    #[test]
    fn structured_file_tools_check_targets_instead_of_content() {
        assert!(
            pre_tool_use_output(&tool_input(
                "Write",
                json!({
                    "file_path": "docs/storage.md",
                    "content": "The database is .mobius/mobius.sqlite3",
                }),
            ))
            .is_none()
        );
        assert!(
            pre_tool_use_output(&tool_input(
                "Write",
                json!({
                    "file_path": "debug/not.mobius/mobius.sqlite3",
                    "content": "ordinary fixture",
                }),
            ))
            .is_none()
        );
        assert!(
            pre_tool_use_output(&tool_input(
                "Write",
                json!({
                    "file_path": ".mobius/mobius.sqlite3",
                    "content": "tamper",
                }),
            ))
            .is_some()
        );

        assert!(
            pre_tool_use_output(&tool_input(
                "mcp__filesystem__copy_file",
                json!({
                    "source": ".mobius/mobius.sqlite3",
                    "destination": "debug/mobius.sqlite3",
                }),
            ))
            .is_none()
        );
        assert!(
            pre_tool_use_output(&tool_input(
                "mcp__filesystem__copy_file",
                json!({
                    "source": "fixtures/mobius.sqlite3",
                    "destination": ".mobius/mobius.sqlite3",
                }),
            ))
            .is_some()
        );
        assert!(
            pre_tool_use_output(&tool_input(
                "mcp__filesystem__move_file",
                json!({
                    "source_path": ".mobius/artifacts/blobs/digest",
                    "destination_path": "debug/digest",
                }),
            ))
            .is_some()
        );
    }

    #[test]
    fn shell_continuations_and_execute_named_mcp_tools_use_shell_target_analysis() {
        for input in [
            tool_input(
                "write_stdin",
                json!({
                    "session_id": 17,
                    "chars": "rm -f .mobius/mobius.sqlite3\n",
                }),
            ),
            tool_input(
                "mcp__terminal__execute",
                json!({"cmd": "cp fixture.sqlite3 .mobius/mobius.sqlite3"}),
            ),
            tool_input(
                "mcp__terminal__execute_command",
                json!({"command": "rm -rf .mobius/artifacts/blobs"}),
            ),
            tool_input(
                "mcp__terminal__execute_command",
                json!({
                    "command": "rm -f mobius.sqlite3",
                    "cwd": "/project/.mobius",
                }),
            ),
        ] {
            assert!(
                pre_tool_use_output(&input).is_some(),
                "protected shell mutation must be denied for {}",
                input.tool_name
            );
        }

        for input in [
            tool_input(
                "write_stdin",
                json!({"session_id": 17, "chars": "cp .mobius/mobius.sqlite3 debug/copy.sqlite3\n"}),
            ),
            tool_input(
                "mcp__terminal__execute",
                json!({"cmd": "cp .mobius/mobius.sqlite3 debug/copy.sqlite3"}),
            ),
            tool_input(
                "mcp__terminal__execute_command",
                json!({
                    "command": "rm -f mobius.sqlite3",
                    "cwd": "/project/debug",
                }),
            ),
        ] {
            assert!(
                pre_tool_use_output(&input).is_none(),
                "protected read with an ordinary write target must be allowed for {}",
                input.tool_name
            );
        }
    }

    #[test]
    fn copy_install_link_and_dd_extract_complete_write_targets() {
        let cases = [
            (
                "cp short target directory",
                "cp -t .mobius/artifacts/blobs fixture",
                true,
            ),
            (
                "cp attached short target directory",
                "cp -t.mobius/artifacts/blobs fixture",
                true,
            ),
            (
                "cp concatenated quoted target directory",
                "cp -t.mobius/artifacts/'blobs' fixture",
                true,
            ),
            (
                "cp combined short target directory",
                "cp -vt .mobius/artifacts/blobs fixture",
                true,
            ),
            (
                "cp attached combined short target directory",
                "cp -vt.mobius/artifacts/blobs fixture",
                true,
            ),
            (
                "cp long target directory",
                "cp --target-directory .mobius/artifacts/blobs fixture",
                true,
            ),
            (
                "cp attached long target directory",
                "cp --target-directory=.mobius/artifacts/blobs fixture",
                true,
            ),
            (
                "install target directory",
                "install -v -t .mobius/artifacts/blobs fixture",
                true,
            ),
            (
                "install concatenated quoted target directory",
                "install -t.mobius/artifacts/'blobs' fixture",
                true,
            ),
            (
                "link target directory",
                "ln -svt.mobius/artifacts/blobs fixture",
                true,
            ),
            (
                "link concatenated quoted target directory",
                "ln -t.mobius/artifacts/'blobs' fixture",
                true,
            ),
            (
                "multi-source cp from protected state",
                "cp .mobius/mobius.sqlite3 .mobius/mobius.sqlite3-wal debug/",
                false,
            ),
            (
                "combined-option cp from protected state",
                "cp -avt debug/ .mobius/mobius.sqlite3",
                false,
            ),
            (
                "hard-link cp keeps protected source denial",
                "cp -l .mobius/mobius.sqlite3 debug/database.sqlite3",
                true,
            ),
            (
                "long hard-link cp keeps protected source denial",
                "cp --link .mobius/mobius.sqlite3 debug/database.sqlite3",
                true,
            ),
            (
                "symbolic-link cp keeps protected source denial",
                "cp -s .mobius/mobius.sqlite3 debug/database.sqlite3",
                true,
            ),
            (
                "long symbolic-link cp keeps protected source denial",
                "cp --symbolic-link .mobius/mobius.sqlite3 debug/database.sqlite3",
                true,
            ),
            (
                "byte-copy cp still permits protected source reads",
                "cp -v .mobius/mobius.sqlite3 debug/database.sqlite3",
                false,
            ),
            (
                "quoted byte-copy cp permits protected source reads",
                "cp '.mobius/mobius.sqlite3' 'debug/database.sqlite3'",
                false,
            ),
            (
                "dd reads protected state",
                "dd if=.mobius/mobius.sqlite3 of=debug/database.sqlite3",
                false,
            ),
            (
                "quoted dd reads protected state",
                "dd 'if=.mobius/mobius.sqlite3' 'of=debug/database.sqlite3'",
                false,
            ),
            (
                "dd writes protected state",
                "dd if=fixture.sqlite3 of=.mobius/mobius.sqlite3",
                true,
            ),
            (
                "install copies protected source out",
                "install .mobius/mobius.sqlite3 debug/database.sqlite3",
                false,
            ),
            (
                "quoted install copies protected source out",
                "install '.mobius/mobius.sqlite3' 'debug/database.sqlite3'",
                false,
            ),
            (
                "install with mode copies protected source out",
                "install -m 0644 .mobius/mobius.sqlite3 debug/database.sqlite3",
                false,
            ),
            (
                "install target directory copies protected source out",
                "install -vt debug/ .mobius/mobius.sqlite3",
                false,
            ),
            (
                "link keeps protected source denial",
                "ln .mobius/mobius.sqlite3 debug/database.sqlite3",
                true,
            ),
            (
                "link target directory keeps protected source denial",
                "ln -st debug/ .mobius/mobius.sqlite3",
                true,
            ),
        ];

        for (name, command, denied) in cases {
            assert_eq!(
                pre_tool_use_output_bound(
                    &tool_input("exec_command", json!({"cmd": command})),
                    "/project",
                )
                .is_some(),
                denied,
                "unexpected shell hook decision for {name}"
            );
        }
    }

    #[test]
    fn shell_tools_check_mutation_operands_redirections_and_workdir_table() {
        let cases = vec![
            (
                "read protected source",
                json!({"cmd": "cat .mobius/mobius.sqlite3"}),
                false,
            ),
            (
                "remove protected database",
                json!({"cmd": "rm -f .mobius/mobius.sqlite3"}),
                true,
            ),
            (
                "dynamic chmod mode before a literal managed target",
                json!({"cmd": "chmod -R \"$MODE\" .mobius/artifacts/blobs"}),
                true,
            ),
            (
                "dynamic chown owner before a literal managed target",
                json!({"cmd": "chown -R \"$OWNER\" .mobius/artifacts/blobs"}),
                true,
            ),
            (
                "dynamic metadata word after a literal ordinary target",
                json!({"cmd": "chmod -R 000 /tmp/ordinary \"$LATER\" .mobius/artifacts/blobs"}),
                true,
            ),
            (
                "remove protected database family through star glob",
                json!({"cmd": "rm -f .mobius/mobius.sqlite3*"}),
                true,
            ),
            (
                "remove protected database through question glob",
                json!({"cmd": "rm -f .mobius/mobius.sqlite?"}),
                true,
            ),
            (
                "remove protected database through bracket glob",
                json!({"cmd": "rm -f .mobius/mobius.sqlite[3]"}),
                true,
            ),
            (
                "remove protected database family through brace expansion",
                json!({"cmd": "rm -f .mobius/mobius.sqlite3{,-wal,-shm}"}),
                true,
            ),
            (
                "remove protected database through quote concatenation",
                json!({"cmd": "rm -f .mobius/mobius.sqlite''3"}),
                true,
            ),
            (
                "remove protected database through dollar quote",
                json!({"cmd": "rm -f $'.mobius/mobius.sqlite3'"}),
                true,
            ),
            (
                "remove protected database through command substitution",
                json!({"cmd": "rm -f .mobius/$(printf mobius.sqlite3)"}),
                true,
            ),
            (
                "remove protected database through dot component",
                json!({"cmd": "rm -f .mobius/./mobius.sqlite3"}),
                true,
            ),
            (
                "remove protected database through repeated separator",
                json!({"cmd": "rm -f .mobius//mobius.sqlite3"}),
                true,
            ),
            (
                "redirect without whitespace",
                json!({"cmd": "echo corrupt>.mobius/mobius.sqlite3"}),
                true,
            ),
            (
                "relative remove under protected workdir",
                json!({
                    "cmd": "rm -f mobius.sqlite3",
                    "workdir": "/project/.mobius",
                }),
                true,
            ),
            (
                "copy protected source out",
                json!({"cmd": "cp .mobius/mobius.sqlite3 debug-copy.sqlite3"}),
                false,
            ),
            (
                "verbose copy protected source out",
                json!({"cmd": "cp -v .mobius/mobius.sqlite3 debug-copy.sqlite3"}),
                false,
            ),
            (
                "copy target directory option into protected state",
                json!({"cmd": "cp --target-directory=.mobius/artifacts/blobs source"}),
                true,
            ),
            (
                "echo protected path as prose",
                json!({"cmd": "echo .mobius/mobius.sqlite3"}),
                false,
            ),
            (
                "copy into protected destination",
                json!({"cmd": "cp fixture.sqlite3 .mobius/mobius.sqlite3"}),
                true,
            ),
            (
                "move protected source out",
                json!({"cmd": "mv .mobius/mobius.sqlite3 debug-move.sqlite3"}),
                true,
            ),
            (
                "find delete under managed root",
                json!({"cmd": "find .mobius -type f -delete"}),
                true,
            ),
            (
                "rsync delete into artifact blobs",
                json!({"cmd": "rsync --delete empty/ .mobius/artifacts/blobs/"}),
                true,
            ),
            (
                "rsync normalized backup directory into artifact blobs",
                json!({
                    "cmd": "rsync --backup-dir=.mobius/./artifacts/blobs source destination",
                }),
                true,
            ),
            (
                "shred protected database",
                json!({"cmd": "shred -u .mobius/mobius.sqlite3"}),
                true,
            ),
            (
                "truncate protected database family through star glob",
                json!({"cmd": "truncate -s 0 .mobius/mobius.sqlite3*"}),
                true,
            ),
            (
                "remove artifact blobs through question glob",
                json!({"cmd": "rm -rf .mobius/artifacts/blob?"}),
                true,
            ),
            (
                "diff output option into protected state",
                json!({"cmd": "diff --output=.mobius/mobius.sqlite3 before after"}),
                true,
            ),
            (
                "read protected database through checksum pipeline",
                json!({"cmd": "cat .mobius/mobius.sqlite3 | sha256sum"}),
                false,
            ),
            (
                "mention protected database through counting pipeline",
                json!({"cmd": "echo .mobius/mobius.sqlite3 | wc -c"}),
                false,
            ),
            (
                "read protected database through command list",
                json!({
                    "cmd": "cat .mobius/mobius.sqlite3; rg needle .mobius/mobius.sqlite3",
                }),
                false,
            ),
            (
                "mutation in command list after protected read",
                json!({
                    "cmd": "cat .mobius/mobius.sqlite3; rm .mobius/mobius.sqlite3",
                }),
                true,
            ),
            (
                "ordinary mutation after protected read",
                json!({
                    "cmd": "cat .mobius/mobius.sqlite3; rm docs/draft.md",
                }),
                false,
            ),
            (
                "read-only command chain",
                json!({
                    "cmd": "cat .mobius/mobius.sqlite3 && rg needle .mobius/mobius.sqlite3",
                }),
                false,
            ),
            (
                "read-only fallback chain",
                json!({
                    "cmd": "cat .mobius/mobius.sqlite3 || rg needle .mobius/mobius.sqlite3",
                }),
                false,
            ),
            (
                "redirection in otherwise read-only command list",
                json!({
                    "cmd": "cat .mobius/mobius.sqlite3; printf snapshot > debug.txt",
                }),
                false,
            ),
            (
                "protected redirection in otherwise read-only command list",
                json!({
                    "cmd": "cat .mobius/mobius.sqlite3; printf snapshot > .mobius/mobius.sqlite3",
                }),
                true,
            ),
            (
                "ordinary combined redirection after protected read",
                json!({
                    "cmd": "cat .mobius/mobius.sqlite3; printf snapshot >& debug.txt",
                }),
                false,
            ),
            (
                "ordinary ampersand redirection after protected read",
                json!({
                    "cmd": "cat .mobius/mobius.sqlite3; printf snapshot &> debug.txt",
                }),
                false,
            ),
            (
                "protected combined redirection",
                json!({"cmd": "printf snapshot >& .mobius/mobius.sqlite3"}),
                true,
            ),
            (
                "protected ampersand redirection",
                json!({"cmd": "printf snapshot &> .mobius/mobius.sqlite3"}),
                true,
            ),
            (
                "protected ampersand append redirection",
                json!({"cmd": "printf snapshot &>> .mobius/mobius.sqlite3"}),
                true,
            ),
            (
                "file descriptor duplication after protected read",
                json!({"cmd": "cat .mobius/mobius.sqlite3; printf snapshot 2>&1"}),
                false,
            ),
            (
                "file descriptor close after protected read",
                json!({"cmd": "cat .mobius/mobius.sqlite3; printf snapshot >&-"}),
                false,
            ),
            (
                "substitution in otherwise read-only command list",
                json!({
                    "cmd": "cat .mobius/mobius.sqlite3; rg needle $(printf .mobius/mobius.sqlite3)",
                }),
                true,
            ),
            (
                "ordinary substitution output after protected read",
                json!({
                    "cmd": "cat .mobius/mobius.sqlite3; printf \"$(date)\" > docs/debug.txt",
                }),
                false,
            ),
            (
                "substitution contains protected nested mutation",
                json!({
                    "cmd": "cat docs/readme.md; printf \"$(rm .mobius/mobius.sqlite3)\" > docs/debug.txt",
                }),
                true,
            ),
            (
                "substituted output redirects to protected state",
                json!({
                    "cmd": "cat docs/readme.md; printf \"$(date)\" > .mobius/mobius.sqlite3",
                }),
                true,
            ),
            (
                "unknown command may touch protected state",
                json!({
                    "cmd": "cat docs/draft.md; mystery-command .mobius/mobius.sqlite3",
                }),
                true,
            ),
            (
                "pipeline mutation of protected database",
                json!({"cmd": "cat fixture.sqlite3 | tee .mobius/mobius.sqlite3"}),
                true,
            ),
            (
                "ordinary mutation with protected prose field",
                json!({
                    "cmd": "rm -f docs/draft.md",
                    "justification": "Database lives at .mobius/mobius.sqlite3",
                }),
                false,
            ),
            (
                "remove managed ancestor contents",
                json!({"cmd": "rm -rf .mobius/*"}),
                true,
            ),
            (
                "remove derived view",
                json!({"cmd": "rm -f .mobius/views/current"}),
                false,
            ),
        ];

        for (name, value, denied) in cases {
            assert_eq!(
                pre_tool_use_output(&tool_input("exec_command", value)).is_some(),
                denied,
                "unexpected shell hook decision for {name}"
            );
        }
    }

    #[test]
    fn destructive_ancestor_scopes_are_denied_without_spelling_managed_paths() {
        let shell_cases = [
            ("recursive project-root deletion", "rm -rf /project"),
            (
                "leading static redirection before recursive project deletion",
                ">debug.log rm -rf /project",
            ),
            (
                "interspersed static redirection before a recursive target",
                "rm -rf >debug.log /project",
            ),
            ("recursive ancestor deletion", "rm --recursive --force /"),
            ("relative project-root deletion", "rm -R ."),
            (
                "literal sh wrapper deleting the project root",
                "sh -c 'rm -rf /project'",
            ),
            (
                "literal bash wrapper deleting the project root",
                "bash -lc 'rm -rf /project'",
            ),
            (
                "bash wrapper options and delimiter before a destructive payload",
                "bash --noprofile -lc -- 'rm -rf /project'",
            ),
            (
                "nested literal shell wrappers deleting the project root",
                "sh -c \"bash -lc 'rm -rf /project'\"",
            ),
            (
                "nested wrapper with a named shell option deleting the project root",
                "sh -c \"bash -o posix -c 'rm -rf /project'\"",
            ),
            (
                "wrapper with a disabled named shell option deleting the project root",
                "bash +o posix -c 'rm -rf /project'",
            ),
            (
                "wrapper with a named shopt option deleting the project root",
                "bash -O extglob -c 'rm -rf /project'",
            ),
            (
                "wrapper with a disabled named shopt option deleting the project root",
                "bash +O extglob -c 'rm -rf /project'",
            ),
            (
                "wrapper with a combined named-option cluster deleting the project root",
                "bash -lo posix -c 'rm -rf /project'",
            ),
            (
                "wrapper with an rcfile option deleting the project root",
                "bash --rcfile /dev/null -c 'rm -rf /project'",
            ),
            (
                "wrapper with an init-file option deleting the project root",
                "bash --init-file /dev/null -c 'rm -rf /project'",
            ),
            (
                "sh wrapper with an rcfile option deleting the project root",
                "sh --rcfile /dev/null -c 'rm -rf /project'",
            ),
            (
                "sh wrapper with an init-file option deleting the project root",
                "sh --init-file /dev/null -c 'rm -rf /project'",
            ),
            (
                "env unset prefix before recursive project deletion",
                "env -u FOO rm -rf /project",
            ),
            (
                "env long unset prefix before a destructive wrapper",
                "env --unset FOO sh -c 'rm -rf /project'",
            ),
            (
                "env chdir prefix before recursive project deletion",
                "env -C /tmp rm -rf /project",
            ),
            (
                "env long chdir prefix before recursive project deletion",
                "env --chdir /tmp rm -rf /project",
            ),
            (
                "env ignore-environment dash before recursive project deletion",
                "env - rm -rf /project",
            ),
            (
                "sudo user prefix before recursive project deletion",
                "sudo -u root rm -rf /project",
            ),
            (
                "sudo assignment before recursive project deletion",
                "sudo -u root FOO=x rm -rf /project",
            ),
            (
                "sudo execution option values before recursive project deletion",
                "sudo -D /tmp -g root -r role -t type -T 10 -u root rm -rf /project",
            ),
            (
                "sudo long execution option values before recursive project deletion",
                "sudo --chdir /tmp --group root --role role --type type --command-timeout 10 --user root rm -rf /project",
            ),
            ("moving the project root", "mv /project /archive/project"),
            (
                "moving the project root with a target directory",
                "mv -t /archive /project",
            ),
            (
                "moving the project root with a long target directory",
                "mv --target-directory=/archive /project",
            ),
            (
                "moving the project root with an unmodeled backup value",
                "mv --backup=numbered /project /archive/project",
            ),
            (
                "moving the project root with an unmodeled update value",
                "mv --update=none /project /archive/project",
            ),
            (
                "replacing the project root",
                "mv --no-target-directory /replacement /project",
            ),
            ("ignored-file clean at project scope", "git clean -fdx"),
            (
                "interspersed static redirection in ignored-file clean",
                "git >debug.log clean -fdx",
            ),
            ("ignored-only clean at project scope", "git clean -fdX"),
            (
                "negative short exclude clean at project scope",
                "git clean -fd -e '!*'",
            ),
            (
                "negative long exclude clean at project scope",
                "git clean -fd --exclude='!*'",
            ),
            (
                "find delete rooted at the project",
                "find /project -type f -delete",
            ),
            ("find delete rooted above the project", "find / -delete"),
            ("find delete with the default project root", "find -delete"),
            ("recursive chmod of the project", "chmod -R u+rwX /project"),
            (
                "recursive numeric chmod of the project",
                "chmod -R 000 /project",
            ),
            (
                "recursive chown of the project",
                "chown --recursive user:group /project",
            ),
            ("recursive chgrp of the project", "chgrp -vR group /project"),
        ];
        for (name, command) in shell_cases {
            assert!(
                pre_tool_use_output_bound(
                    &tool_input("exec_command", json!({"cmd": command})),
                    "/project",
                )
                .is_some(),
                "ancestor operation was not denied: {name}"
            );
        }

        assert!(
            pre_tool_use_output_bound(
                &tool_input(
                    "write_stdin",
                    json!({"session_id": 17, "chars": "rm -rf /project\n"}),
                ),
                "/project",
            )
            .is_some(),
            "interactive recursive deletion must use the top-level hook cwd"
        );

        let structured_cases = [
            (
                "recursive directory delete",
                "mcp__filesystem__delete_directory",
                json!({"path": "/project", "recursive": true}),
            ),
            (
                "remove",
                "mcp__filesystem__remove",
                json!({"target_path": "/project"}),
            ),
            (
                "move source",
                "mcp__filesystem__move_directory",
                json!({"source_path": "/project", "destination_path": "/archive/project"}),
            ),
            (
                "move with explicit destination replacement",
                "mcp__filesystem__move_directory",
                json!({
                    "source_path": "/replacement",
                    "destination_path": "/project",
                    "overwrite": true,
                }),
            ),
            (
                "replace",
                "mcp__filesystem__replace_directory",
                json!({"path": "/project"}),
            ),
        ];
        for (name, tool_name, value) in structured_cases {
            assert!(
                pre_tool_use_output_bound(&tool_input(tool_name, value), "/project").is_some(),
                "structured ancestor operation was not denied: {name}"
            );
        }
    }

    #[test]
    fn wrapper_chdir_context_is_sequential_and_project_scoped() {
        let ordinary = std::env::temp_dir().join(format!(
            "mobius-hook-wrapper-ordinary-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&ordinary).unwrap();
        let ordinary_text = ordinary.to_str().unwrap();
        let allowed = [
            format!("env -C '{ordinary_text}' sh -c 'rm -rf .'"),
            format!("sudo -D '{ordinary_text}' sh -c 'rm -rf .'"),
            "sudo -R /tmp rm -rf /project".to_owned(),
            "sudo --chroot=/tmp rm -rf /project".to_owned(),
            "sudo -l rm -f .mobius/mobius.sqlite3".to_owned(),
            "sudo -l \"$QUERY\" .mobius/mobius.sqlite3".to_owned(),
            "sudo --validate rm -f .mobius/mobius.sqlite3".to_owned(),
            "sudo --version rm -f .mobius/mobius.sqlite3".to_owned(),
        ];
        for command in allowed {
            assert!(
                pre_tool_use_output_bound(
                    &tool_input("exec_command", json!({"cmd": command})),
                    "/project",
                )
                .is_none(),
                "wrapper context overblocked an unrelated or non-executing command: {command}"
            );
        }

        let denied = [
            "env -C /tmp rm -f .mobius/mobius.sqlite3",
            "env --chdir=/tmp rm -f .mobius/mobius.sqlite3",
            "sudo -D /tmp rm -f .mobius/mobius.sqlite3",
            "sudo --chdir=/tmp rm -f .mobius/mobius.sqlite3",
            "env -C /tmp sh -c 'rm -f .mobius/mobius.sqlite3'",
            "sudo -D /tmp bash -c 'rm -f .mobius/mobius.sqlite3'",
            "env -C /tmp env -C nested rm -f .mobius/mobius.sqlite3",
            "env -C /project rm -f .mobius/mobius.sqlite3",
            "env --chdir=/project rm -f .mobius/mobius.sqlite3",
            "sudo -D /project rm -f .mobius/mobius.sqlite3",
            "sudo --chdir=/project rm -f .mobius/mobius.sqlite3",
            "env -C /project sh -c 'rm -f .mobius/mobius.sqlite3'",
            "sudo -D /project bash -c 'rm -f .mobius/mobius.sqlite3'",
            "env -C / env -C project rm -f .mobius/mobius.sqlite3",
            "env X-Y=z rm -f .mobius/mobius.sqlite3",
            "env X=\"$VALUE\" rm -f .mobius/mobius.sqlite3",
            "env -C /project sh -c 'rm -rf .'",
            "sudo -D /project sh -c 'rm -rf .'",
        ];
        for command in denied {
            assert!(
                pre_tool_use_output_bound(
                    &tool_input("exec_command", json!({"cmd": command})),
                    "/project",
                )
                .is_some(),
                "wrapper context failed to protect project state: {command}"
            );
        }

        assert!(
            pre_tool_use_output_bound(
                &tool_input(
                    "Write",
                    json!({"file_path": "/tmp/.mobius/artifacts/staging/new"}),
                ),
                "/project",
            )
            .is_some(),
            "a bound project must not disable binding-independent explicit state protection"
        );
        assert!(
            pre_tool_use_output_bound(
                &tool_input("Write", json!({"file_path": "/tmp/.mobius/views/current"}),),
                "/project",
            )
            .is_none(),
            "derived views remain outside authoritative-state protection"
        );

        for command in [
            "cd /project && rm -f .mobius/mobius.sqlite3",
            "cd -- /project; rm -f .mobius/mobius.sqlite3",
            "bash -c 'cd /project && rm -f .mobius/mobius.sqlite3'",
        ] {
            assert!(
                pre_tool_use_output_bound(
                    &tool_input("exec_command", json!({"cmd": command, "workdir": "/tmp"}),),
                    "/project",
                )
                .is_some(),
                "literal cd failed to carry its cwd into protected mutation: {command}"
            );
        }

        assert!(
            pre_tool_use_output_bound(
                &tool_input(
                    "exec_command",
                    json!({
                        "cmd": "cd /project | rm -rf .",
                        "workdir": ordinary_text,
                    }),
                ),
                "/project",
            )
            .is_none(),
            "a pipeline-local cd must not change the following command's cwd"
        );
        assert!(
            pre_tool_use_output_bound(
                &tool_input(
                    "exec_command",
                    json!({"cmd": "cd /project && file .mobius/mobius.sqlite3", "workdir": "/tmp"}),
                ),
                "/project",
            )
            .is_none(),
            "the file inspector is read-only even after a literal cd"
        );

        for command in [
            "cd /does-not-exist; rm -rf .",
            "env cd /tmp; rm -rf .",
            "/usr/bin/cd /tmp; rm -rf .",
            "cd /tmp || rm -rf .",
            "cd /; rm -rf project",
            "sh -c 'cd /; bash -c \"rm -rf project\"'",
            "sh -c 'cd /tmp'; rm -rf .",
        ] {
            assert!(
                pre_tool_use_output_bound(
                    &tool_input("exec_command", json!({"cmd": command})),
                    "/project",
                )
                .is_some(),
                "cd success/failure semantics hid a destructive project ancestor: {command}"
            );
        }
        assert!(
            pre_tool_use_output_bound(
                &tool_input(
                    "exec_command",
                    json!({"cmd": format!("cd '{ordinary_text}' && rm -rf .")}),
                ),
                "/project",
            )
            .is_none(),
            "an && command after a successful literal cd must use only the changed cwd"
        );
        fs::remove_dir_all(ordinary).unwrap();
    }

    #[test]
    fn ancestor_guard_requires_an_existing_bound_project() {
        let root =
            std::env::temp_dir().join(format!("mobius-hook-binding-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let root_text = root.to_str().unwrap();
        let root_delete = json!({"cmd": format!("rm -rf '{}'", root.display())});
        assert!(
            pre_tool_use_output(&tool_input_at(
                "exec_command",
                root_delete.clone(),
                root_text
            ))
            .is_none(),
            "an unbound ordinary cwd must not acquire imaginary project-root protection"
        );
        assert!(
            pre_tool_use_output(&tool_input_at(
                "exec_command",
                json!({"cmd": format!("find '{}' -type f -delete", root.display())}),
                root_text,
            ))
            .is_none(),
            "an unbound ordinary cwd must remain available to find-delete"
        );
        assert!(
            pre_tool_use_output(&tool_input_at(
                "exec_command",
                json!({"cmd": format!("chmod -R u+rwX '{}'", root.display())}),
                root_text,
            ))
            .is_none(),
            "an unbound ordinary cwd must remain available to recursive metadata changes"
        );
        let structured_root_delete = json!({"path": root_text, "recursive": true});
        assert!(
            pre_tool_use_output(&tool_input_at(
                "mcp__filesystem__delete_directory",
                structured_root_delete.clone(),
                root_text,
            ))
            .is_none(),
            "an unbound ordinary cwd must not acquire structured ancestor protection"
        );
        assert!(
            pre_tool_use_output(&tool_input_at(
                "exec_command",
                json!({"cmd": "rm -f .mobius/mobius.sqlite3"}),
                root_text,
            ))
            .is_some(),
            "an explicit relative Mobius-state mutation remains protected before binding"
        );
        let external_state = root.parent().unwrap().join(format!(
            "other-{}/.mobius/mobius.sqlite3",
            uuid::Uuid::new_v4()
        ));
        assert!(
            pre_tool_use_output(&tool_input_at(
                "exec_command",
                json!({"cmd": format!("rm -f '{}'", external_state.display())}),
                root_text,
            ))
            .is_some(),
            "the unbound generic matcher must protect an explicit absolute Mobius-state target"
        );
        assert!(
            pre_tool_use_output(&tool_input_at(
                "exec_command",
                json!({"cmd": "rm -f .mobius/views/current.csv"}),
                root_text,
            ))
            .is_none(),
            "derived views remain outside authoritative-state protection"
        );

        let mobius = root.join(".mobius");
        fs::create_dir(&mobius).unwrap();
        fs::write(mobius.join("mobius.sqlite3"), b"binding marker").unwrap();
        assert!(
            pre_tool_use_output(&tool_input_at("exec_command", root_delete, root_text)).is_some(),
            "an existing bound project must protect destructive ancestor scope"
        );
        assert!(
            pre_tool_use_output(&tool_input_at(
                "mcp__filesystem__delete_directory",
                structured_root_delete,
                root_text,
            ))
            .is_some(),
            "an existing bound project must protect structured destructive ancestor scope"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn git_clean_pathspecs_respect_worktree_and_project_scope() {
        let root =
            std::env::temp_dir().join(format!("mobius-hook-git-clean-{}", uuid::Uuid::new_v4()));
        let subdirectory = root.join("subdirectory");
        fs::create_dir_all(root.join(".mobius")).unwrap();
        fs::create_dir_all(subdirectory.join("build")).unwrap();
        fs::write(root.join(".mobius/mobius.sqlite3"), b"binding marker").unwrap();
        fs::write(root.join("README.md"), b"ordinary").unwrap();
        fs::write(subdirectory.join("build/output.txt"), b"ordinary").unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(&root)
                .output()
                .unwrap()
                .status
                .success()
        );

        let selected_private_state = std::process::Command::new("git")
            .args(["clean", "-ndx", "--", ":(top).mobius"])
            .current_dir(&subdirectory)
            .output()
            .unwrap();
        assert!(selected_private_state.status.success());
        assert!(
            String::from_utf8(selected_private_state.stdout)
                .unwrap()
                .contains("Would remove ../.mobius/"),
            "the regression command must actually select root private state"
        );

        let subdirectory_text = subdirectory.to_str().unwrap();
        for command in [
            "git clean -fdx -- ':(top).mobius'",
            "git clean -fdx -- ':(top,icase).MOBIUS'",
            "git clean -fdx -- ':(top,icase).MOBIUS/mobius.sqlite3'",
            "git --icase-pathspecs clean -fdx -- ':(top).MOBIUS'",
            "git --icase-pathspecs clean -fdx -- ':(top).MOBIUS/mobius.sqlite3'",
            "GIT_ICASE_PATHSPECS=1 git clean -fdx -- ':(top).MOBIUS'",
            "GIT_ICASE_PATHSPECS=1 git clean -fdx -- ':(top).MOBIUS/mobius.sqlite3'",
            "git clean -fdx -- ../.mobius",
        ] {
            assert!(
                pre_tool_use_output(&tool_input_at(
                    "exec_command",
                    json!({"cmd": command}),
                    subdirectory_text,
                ))
                .is_some(),
                "a static clean pathspec reached root private state: {command}"
            );
        }
        for command in [
            "git clean -fdx -- ':(top)README.md'",
            "git clean -fdx -- build/",
            "git clean -ndx -- ':(top).mobius'",
            "git clean -ndx -- .mobius",
            "git clean -ndx -- ../.mobius",
        ] {
            assert!(
                pre_tool_use_output(&tool_input_at(
                    "exec_command",
                    json!({"cmd": command}),
                    subdirectory_text,
                ))
                .is_none(),
                "an unrelated or non-mutating subdirectory clean was overblocked: {command}"
            );
        }
        assert!(
            pre_tool_use_output(&tool_input_at(
                "exec_command",
                json!({"cmd": "git clean -fdx -- README.md"}),
                root.to_str().unwrap(),
            ))
            .is_none(),
            "a bound-root clean narrowed to an ordinary file was overblocked"
        );
        assert!(
            pre_tool_use_output(&tool_input_at(
                "exec_command",
                json!({"cmd": "git clean -fdx -- .mobius"}),
                root.to_str().unwrap(),
            ))
            .is_some(),
            "a bound-root clean selecting private state was not denied"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn binding_is_resolved_per_effective_cwd_and_cross_project_target() {
        let workspace = std::env::temp_dir().join(format!(
            "mobius-hook-multi-binding-{}",
            uuid::Uuid::new_v4()
        ));
        let project_a = workspace.join("project-a");
        let project_b = workspace.join("project-b");
        let unbound = workspace.join("unbound");
        for root in [&project_a, &project_b] {
            fs::create_dir_all(root.join(".mobius")).unwrap();
            fs::write(root.join(".mobius/mobius.sqlite3"), b"binding marker").unwrap();
        }
        fs::create_dir_all(&unbound).unwrap();
        let a = project_a.to_str().unwrap();
        let b = project_b.to_str().unwrap();
        let outside = unbound.to_str().unwrap();

        for command in [
            format!("cd '{b}' && rm -rf ."),
            format!("cd '{b}' &&\nfind . -delete"),
            format!("cd '{b}' && rm -rf .;"),
            format!("cd '{b}' && rm -rf . &"),
            format!("cd '{b}' && find . -delete;"),
            format!("cd '{b}' && find . -delete &"),
            format!("cd -L '{b}' && rm -rf ."),
            format!("cd -P -- '{b}' && rm -rf ."),
            format!("cd -L -P '{b}' && rm -rf ."),
            format!("cd -P -L -- '{b}' && rm -rf ."),
            format!("cd -LP '{b}' && rm -rf ."),
            format!("sh -c \"cd '{b}' && rm -rf .\""),
            format!("sh -c \"cd '{b}' &&\nfind . -delete\""),
            format!("sh -c \"cd '{b}' && rm -rf .;\""),
            format!("sh -c \"cd '{b}' && rm -rf . &\""),
            format!("command cd '{b}' && rm -rf ."),
            format!("command -p -- cd '{b}' && rm -rf ."),
            format!("command command cd -L '{b}' && rm -rf ."),
            format!("command -- cd -P -- '{b}' && rm -rf ."),
            format!("command cd '{b}'; rm -rf ."),
            format!("command cd '{b}' || rm -rf ."),
            format!("sh -c \"command cd '{b}' && rm -rf .\""),
            format!("env -C '{b}' rm -rf ."),
            format!("sudo -D '{b}' rm -rf ."),
            format!("git -C{b} clean -fdx"),
            format!("git --git-dir='{b}/.git' --work-tree='{b}' clean -fdX"),
            format!("cd '{b}' && true; find . -delete"),
            format!("cd '{b}' && true | find . -delete"),
        ] {
            assert!(
                pre_tool_use_output(&tool_input_at(
                    "exec_command",
                    json!({"cmd": command}),
                    outside,
                ))
                .is_some(),
                "an unbound launch failed to discover the command-local binding: {command}"
            );
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let child = project_b.join("child");
            let link = unbound.join("link-to-project-b-child");
            fs::create_dir(&child).unwrap();
            symlink(&child, &link).unwrap();
            let physical = format!("cd -L -P '{}/..' && rm -rf .", link.display());
            assert!(
                pre_tool_use_output(&tool_input_at(
                    "exec_command",
                    json!({"cmd": physical}),
                    outside,
                ))
                .is_some(),
                "the final -P option must resolve symlink/.. to project B"
            );
            let logical = format!("cd -P -L '{}/..' && rm -rf .", link.display());
            assert!(
                pre_tool_use_output(&tool_input_at(
                    "exec_command",
                    json!({"cmd": logical}),
                    outside,
                ))
                .is_none(),
                "the final -L option must retain lexical symlink/.. resolution"
            );
        }

        assert!(
            pre_tool_use_output(&tool_input_at(
                "exec_command",
                json!({"cmd": "rm -rf .", "workdir": b}),
                a,
            ))
            .is_some(),
            "tool workdir B must not be shadowed by hook cwd A"
        );
        for command in [
            "rm -rf .;",
            "rm -rf .;\n",
            "true &&\nfind . -delete",
            "false ||\nfind . -delete",
            "printf value |\nfind . -delete",
            "rm -rf . &",
            "rm -rf . &\n",
            "find . -delete;",
            "find . -delete &",
            "sh -c 'rm -rf .;'",
            "sh -c 'rm -rf .;\n'",
            "sh -c 'find . -delete &'",
        ] {
            assert!(
                pre_tool_use_output(&tool_input_at(
                    "exec_command",
                    json!({"cmd": command, "workdir": b}),
                    a,
                ))
                .is_some(),
                "a valid trailing shell terminator bypassed project B protection: {command}"
            );
        }
        assert!(
            pre_tool_use_output(&tool_input_at(
                "exec_command",
                json!({"cmd": format!("rm -rf '{b}'")}),
                a,
            ))
            .is_some(),
            "an absolute destructive target must discover project B"
        );
        assert!(
            pre_tool_use_output(&tool_input_at(
                "exec_command",
                json!({"cmd": format!("cd '{b}' || cd '{outside}' && find . -delete")}),
                outside,
            ))
            .is_some(),
            "a successful short-circuit branch must retain project B's cwd"
        );
        assert!(
            pre_tool_use_output(&tool_input_at(
                "exec_command",
                json!({"cmd": format!("cd '{outside}' && true & rm -rf ."), "workdir": b}),
                outside,
            ))
            .is_some(),
            "a background AND-OR list must restore its entry cwd"
        );
        assert!(
            pre_tool_use_output(&tool_input_at(
                "exec_command",
                json!({"cmd": "rm -rf .", "workdir": outside, "cwd": b}),
                outside,
            ))
            .is_some(),
            "differing workdir and cwd values must both be analyzed"
        );
        assert!(
            pre_tool_use_output(&tool_input_at(
                "mcp__filesystem__write_file",
                json!({"path": project_b.join(".mobius/mobius.sqlite3")}),
                a,
            ))
            .is_some(),
            "an absolute structured target must discover project B"
        );
        assert!(
            pre_tool_use_output(&tool_input_at(
                "mcp__filesystem__delete_file",
                json!({"target": project_b.join(".mobius/mobius.sqlite3")}),
                outside,
            ))
            .is_some(),
            "structured target keys must receive binding-independent state protection"
        );
        for command in [
            format!("git -C{b} clean -fdx"),
            format!("git --git-dir='{b}/.git' --work-tree='{b}' clean -fdX"),
        ] {
            assert!(
                pre_tool_use_output(&tool_input_at("exec_command", json!({"cmd": command}), a,))
                    .is_some(),
                "project A hid project B's explicit Git clean scope"
            );
        }

        let mut branches = String::new();
        for index in 0..20 {
            branches.push_str("cd branch-");
            branches.push_str(&index.to_string());
            branches.push_str("; ");
        }
        for command in [
            format!("cd '{b}'; {branches}rm -rf ."),
            format!("{branches}cd '{b}' && rm -rf ."),
            format!("{branches}cd '{b}'; rm -rf ."),
            format!("sh -c \"{branches}cd '{b}'; rm -rf .\""),
            format!("{branches}command cd '{b}' && rm -rf ."),
            format!("{branches}command cd '{b}'; rm -rf ."),
            format!("{branches}command cd '{b}' || rm -rf ."),
            format!("sh -c \"{branches}command cd '{b}' && rm -rf .\""),
        ] {
            assert!(
                pre_tool_use_output(&tool_input_at(
                    "exec_command",
                    json!({"cmd": command}),
                    outside,
                ))
                .is_some(),
                "candidate saturation lost a discovered or recovered binding"
            );
        }

        for value in [
            json!({"cmd": "rm -rf ."}),
            json!({"cmd": format!("cd -Q '{b}' && rm -rf .")}),
            json!({"cmd": format!("cd '{b}' & rm -rf .")}),
            json!({"cmd": format!("cd '{b}' & find . -delete")}),
            json!({"cmd": format!("command cd '{outside}' && rm -rf .")}),
            json!({"cmd": "command -v cd && rm -rf ."}),
            json!({"cmd": "command -V cd && rm -rf ."}),
            json!({"cmd": format!("/usr/bin/command cd '{b}' && rm -rf .")}),
            json!({"cmd": format!("nohup command cd '{b}' && rm -rf .")}),
            json!({"cmd": format!("env cd '{b}' && rm -rf .")}),
            json!({"cmd": format!("file '{b}/.mobius/mobius.sqlite3'")}),
            json!({"cmd": format!("git -C'{outside}' clean -fdx")}),
            json!({"cmd": format!("git --work-tree='{outside}' clean -fdX")}),
            json!({"cmd": format!("true | cd '{b}' && find . -delete")}),
            json!({"cmd": format!("{branches}cd '{outside}'; rm -rf .")}),
            json!({"cmd": "rm -rf .", "workdir": outside}),
        ] {
            assert!(
                pre_tool_use_output(&tool_input_at("exec_command", value, outside)).is_none(),
                "an unbound or read-only control was overblocked"
            );
        }

        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn destructive_ancestor_discovers_nested_binding_without_following_marker_symlinks() {
        let workspace = std::env::temp_dir().join(format!(
            "mobius-hook-descendant-binding-{}",
            uuid::Uuid::new_v4()
        ));
        let destructive_scope = workspace.join("destructive-scope");
        let nested_project = destructive_scope.join("nested/project");
        let ordinary = workspace.join("ordinary");
        fs::create_dir_all(nested_project.join(".mobius")).unwrap();
        fs::write(
            nested_project.join(".mobius/mobius.sqlite3"),
            b"binding marker",
        )
        .unwrap();
        fs::create_dir_all(&ordinary).unwrap();
        let scope = destructive_scope.to_str().unwrap();
        let outside = ordinary.to_str().unwrap();

        for (tool_name, input) in [
            ("exec_command", json!({"cmd": format!("rm -rf '{scope}'")})),
            (
                "exec_command",
                json!({"cmd": format!("find '{scope}' -delete")}),
            ),
            (
                "exec_command",
                json!({"cmd": format!("git -C'{scope}' clean -fdx")}),
            ),
            (
                "exec_command",
                json!({"cmd": format!("mv '{scope}' '{outside}/archive'")}),
            ),
            (
                "exec_command",
                json!({"cmd": format!("chmod -R 000 '{scope}'")}),
            ),
            (
                "mcp__filesystem__delete_directory",
                json!({"path": destructive_scope}),
            ),
            (
                "mcp__filesystem__move_directory",
                json!({"source_path": destructive_scope, "destination_path": ordinary.join("archive")}),
            ),
        ] {
            assert!(
                pre_tool_use_output(&tool_input_at(tool_name, input, outside)).is_some(),
                "a destructive ancestor scope failed to discover its nested binding: {tool_name}"
            );
        }

        let dash_named_project = workspace.join("-bound-scope");
        fs::create_dir_all(dash_named_project.join(".mobius")).unwrap();
        fs::write(
            dash_named_project.join(".mobius/mobius.sqlite3"),
            b"binding marker",
        )
        .unwrap();
        assert!(
            pre_tool_use_output(&tool_input_at(
                "exec_command",
                json!({"cmd": "rm -rf -- -bound-scope", "workdir": workspace}),
                outside,
            ))
            .is_some(),
            "rm operands beginning with a dash must be analyzed after --"
        );
        assert!(
            pre_tool_use_output(&tool_input_at(
                "exec_command",
                json!({"cmd": "rm -rf -bound-scope", "workdir": workspace}),
                outside,
            ))
            .is_none(),
            "an option-shaped rm word before -- must not become a destructive scope"
        );

        let directory_marker_scope = workspace.join("directory-marker-scope");
        fs::create_dir_all(directory_marker_scope.join("candidate/.mobius/mobius.sqlite3"))
            .unwrap();
        assert!(
            pre_tool_use_output(&tool_input_at(
                "exec_command",
                json!({"cmd": format!("rm -rf '{}'", directory_marker_scope.display())}),
                outside,
            ))
            .is_none(),
            "a directory at the database path must not create a binding"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let symlink_marker_scope = workspace.join("symlink-marker-scope");
            let marker_parent = symlink_marker_scope.join("candidate/.mobius");
            let external_marker = workspace.join("external-marker");
            fs::create_dir_all(&marker_parent).unwrap();
            fs::write(&external_marker, b"binding marker").unwrap();
            symlink(&external_marker, marker_parent.join("mobius.sqlite3")).unwrap();
            assert!(
                pre_tool_use_output(&tool_input_at(
                    "exec_command",
                    json!({"cmd": format!("rm -rf '{}'", symlink_marker_scope.display())}),
                    outside,
                ))
                .is_none(),
                "a symlink at the database path must not create a binding"
            );

            let external_project = workspace.join("external-project");
            fs::create_dir_all(external_project.join(".mobius")).unwrap();
            fs::create_dir(external_project.join(".mobius/child")).unwrap();
            fs::create_dir(external_project.join("child")).unwrap();
            fs::write(
                external_project.join(".mobius/mobius.sqlite3"),
                b"binding marker",
            )
            .unwrap();
            let symlink_subtree_scope = workspace.join("symlink-subtree-scope");
            fs::create_dir(&symlink_subtree_scope).unwrap();
            let linked_project = symlink_subtree_scope.join("linked-project");
            symlink(&external_project, &linked_project).unwrap();
            let linked_child = symlink_subtree_scope.join("linked-child");
            symlink(external_project.join("child"), &linked_child).unwrap();
            assert!(
                pre_tool_use_output(&tool_input_at(
                    "exec_command",
                    json!({"cmd": format!("rm -rf '{}'", symlink_subtree_scope.display())}),
                    outside,
                ))
                .is_none(),
                "descendant binding discovery must not follow a symlinked subtree"
            );
            assert!(
                pre_tool_use_output(&tool_input_at(
                    "exec_command",
                    json!({"cmd": format!("rm -rf '{}'", linked_project.display())}),
                    outside,
                ))
                .is_none(),
                "a no-follow rm scope must remove only the project alias"
            );
            assert!(
                pre_tool_use_output(&tool_input_at(
                    "exec_command",
                    json!({"cmd": "rm -rf .", "workdir": linked_project}),
                    outside,
                ))
                .is_some(),
                "a symlinked tool workdir must bind its project referent"
            );
            assert!(
                pre_tool_use_output(&tool_input_at(
                    "exec_command",
                    json!({"cmd": format!("cd -L '{}' && find . -delete", linked_project.display())}),
                    outside,
                ))
                .is_some(),
                "a logical cd through a symlink must bind its project referent"
            );
            for input in [
                json!({"cmd": format!("cd -L '{}' && find .. -delete", linked_child.display())}),
                json!({"cmd": format!("cd -L '{}' && rm -rf ..", linked_child.display())}),
                json!({"cmd": "find .. -delete", "workdir": linked_child}),
                json!({"cmd": "rm -rf ..", "workdir": linked_child}),
                json!({"cmd": "find . -delete", "workdir": linked_child.join("..")} ),
                json!({"cmd": format!("env -C '{}/..' find . -delete", linked_child.display())}),
                json!({"cmd": format!("sudo -D '{}/..' find . -delete", linked_child.display())}),
                json!({"cmd": format!("git -C'{}/..' clean -fdx", linked_child.display())}),
                json!({"cmd": format!("find '{}/..' -delete", linked_child.display())}),
                json!({"cmd": format!("chmod -R 000 '{}/..'", linked_child.display())}),
                json!({"cmd": format!("chown -RP user:group '{}/..'", linked_child.display())}),
            ] {
                assert!(
                    pre_tool_use_output(&tool_input_at("exec_command", input, outside)).is_some(),
                    "physical symlink/.. resolution must retain the bound project guard"
                );
            }
            let linked_state_child = symlink_subtree_scope.join("linked-state-child");
            symlink(external_project.join(".mobius/child"), &linked_state_child).unwrap();
            let physical_state_cwd = linked_state_child.join("..");
            for (tool_name, input) in [
                (
                    "exec_command",
                    json!({"cmd": "rm -f mobius.sqlite3", "workdir": physical_state_cwd}),
                ),
                (
                    "exec_command",
                    json!({
                        "cmd": "rm -f mobius.sqlite3",
                        "workdir": ordinary,
                        "cwd": physical_state_cwd,
                    }),
                ),
                (
                    "mcp__filesystem__delete_file",
                    json!({"target": "mobius.sqlite3", "cwd": physical_state_cwd}),
                ),
            ] {
                assert!(
                    pre_tool_use_output(&tool_input_at(tool_name, input, outside)).is_some(),
                    "effective filesystem cwd must protect a bare Core-state basename: {tool_name}"
                );
            }
            assert!(
                pre_tool_use_output(&tool_input_at(
                    "exec_command",
                    json!({"cmd": format!("find -P '{}' -delete", symlink_subtree_scope.display())}),
                    outside,
                ))
                .is_none(),
                "find -P must skip a symlinked project subtree"
            );
            for command in [
                format!("find -H '{}' -delete", linked_project.display()),
                format!("find -L '{}' -delete", symlink_subtree_scope.display()),
                format!("find '{}' -follow -delete", symlink_subtree_scope.display()),
                format!("chown -RL user:group '{}'", symlink_subtree_scope.display()),
                format!("chown user:group '{}' -RL", symlink_subtree_scope.display()),
            ] {
                assert!(
                    pre_tool_use_output(&tool_input_at(
                        "exec_command",
                        json!({"cmd": command}),
                        outside,
                    ))
                    .is_some(),
                    "an explicit symlink-following destructive scope must fail closed: {command}"
                );
            }
        }

        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn sequential_cd_candidate_growth_is_bounded_and_fail_closed() {
        let ordinary = std::env::temp_dir().join(format!(
            "mobius-hook-saturation-ordinary-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&ordinary).unwrap();
        let mut branches = String::new();
        for index in 0..20 {
            branches.push_str("cd branch-");
            branches.push_str(&index.to_string());
            branches.push_str("; ");
        }
        for command in [
            format!("{branches}rm -rf ."),
            format!("{branches}rm -f .mobius/mobius.sqlite3"),
            format!("{branches}bash -c 'rm -rf .'"),
            format!("cd ..; {branches}rm -rf project"),
            format!("cd ..; {branches}rm -f project/.mobius/mobius.sqlite3"),
            format!("cd ..; {branches}bash -c 'rm -rf project'"),
            format!("cd .mobius; {branches}rm -f mobius.sqlite3"),
            format!("cd .mobius; {branches}bash -c 'rm -f mobius.sqlite3'"),
        ] {
            assert!(
                pre_tool_use_output_bound(
                    &tool_input("exec_command", json!({"cmd": command})),
                    "/project",
                )
                .is_some(),
                "ambiguous cwd candidates failed open: {command}"
            );
        }

        let recovered = format!("{branches}cd '{}' && rm -rf .", ordinary.display());
        assert!(
            pre_tool_use_output_bound(
                &tool_input("exec_command", json!({"cmd": recovered})),
                "/project",
            )
            .is_none(),
            "a direct absolute cd after candidate saturation must recover an exact cwd"
        );
        let absolute_outside = format!("{branches}rm -rf /tmp/unrelated");
        assert!(
            pre_tool_use_output_bound(
                &tool_input("exec_command", json!({"cmd": absolute_outside})),
                "/project",
            )
            .is_none(),
            "an absolute destructive target outside the project must remain exact after saturation"
        );
        for command in [
            format!("{branches}rm -f /tmp/unrelated"),
            format!("{branches}file mobius.sqlite3"),
        ] {
            assert!(
                pre_tool_use_output_bound(
                    &tool_input("exec_command", json!({"cmd": command})),
                    "/project",
                )
                .is_none(),
                "candidate saturation overblocked an exact or read-only operation: {command}"
            );
        }
        fs::remove_dir_all(ordinary).unwrap();
    }

    #[test]
    fn literal_ancestor_targets_survive_later_dynamic_words() {
        let root = "/project with spaces";
        let denied = [
            "rm -rf '/project with spaces' \"$LATER\"",
            "find '/project with spaces' \"$PRED\" -delete",
            "chmod -R 000 '/project with spaces' \"$LATER\"",
            "chown -R user:group '/project with spaces' \"$LATER\"",
            "chgrp -R group '/project with spaces' \"$LATER\"",
        ];
        for command in denied {
            assert!(
                pre_tool_use_output_bound(
                    &tool_input_at("exec_command", json!({"cmd": command}), root),
                    root,
                )
                .is_some(),
                "later dynamic word hid a literal destructive ancestor: {command}"
            );
        }
    }

    #[test]
    fn ancestor_scope_guard_preserves_unrelated_and_nondestructive_operations() {
        let shell_cases = [
            (
                "recursive delete below the project root",
                json!({"cmd": "rm -rf /project/tmp/build"}),
            ),
            (
                "unrelated recursive delete",
                json!({"cmd": "rm -rf /tmp/unrelated"}),
            ),
            (
                "nonrecursive root removal cannot remove descendants",
                json!({"cmd": "rm -f /project"}),
            ),
            (
                "ordinary move into the project directory",
                json!({"cmd": "mv /tmp/note.txt /project"}),
            ),
            (
                "ordinary target-directory move into the project",
                json!({"cmd": "mv -t /project /tmp/note.txt"}),
            ),
            (
                "unmodeled move shape with unrelated paths",
                json!({"cmd": "mv --future-mode /tmp/note.txt /archive"}),
            ),
            (
                "clean without ignored files",
                json!({"cmd": "git clean -fd"}),
            ),
            (
                "repeated clean protected by the self-ignoring policy",
                json!({"cmd": "git clean -fd && git clean -fd"}),
            ),
            (
                "positive clean exclude preserves more files",
                json!({"cmd": "git clean -fd -e build/"}),
            ),
            (
                "ignored clean dry run cannot mutate state",
                json!({"cmd": "git clean -nfdx"}),
            ),
            (
                "negative exclude dry run cannot mutate state",
                json!({"cmd": "git clean -nd -e '!*'"}),
            ),
            (
                "ignored-file clean from a project subdirectory",
                json!({"cmd": "git clean -fdx", "workdir": "/project/subdir"}),
            ),
            (
                "ignored-file clean from unrelated scope",
                json!({"cmd": "git clean -fdX", "cwd": "/tmp/unrelated"}),
            ),
            (
                "ordinary literal shell wrapper",
                json!({"cmd": "sh -c 'printf safe'"}),
            ),
            (
                "literal shell wrapper deleting only a project subtree",
                json!({"cmd": "bash --noprofile -lc -- 'rm -rf /project/tmp/build'"}),
            ),
            (
                "dynamic shell wrapper payload remains outside the lexical guard",
                json!({"cmd": "sh -c \"$payload\""}),
            ),
            (
                "option delimiter before c prevents command-string mode",
                json!({"cmd": "sh -- -c 'rm -rf /project'"}),
            ),
            (
                "ANSI-C quoted wrapper payload remains outside the POSIX literal reader",
                json!({"cmd": "bash -c $'rm -rf /project'"}),
            ),
            (
                "find delete confined below the project root",
                json!({"cmd": "find /project/tmp/build -type f -delete"}),
            ),
            (
                "find delete in an unrelated tree",
                json!({"cmd": "find /tmp/unrelated -delete"}),
            ),
            (
                "find without a delete action",
                json!({"cmd": "find /project -type f -print"}),
            ),
            (
                "recursive chmod confined below the project root",
                json!({"cmd": "chmod -R u+rwX /project/tmp/build"}),
            ),
            (
                "recursive chown in an unrelated tree",
                json!({"cmd": "chown -R user:group /tmp/unrelated"}),
            ),
            (
                "recursive chgrp confined below the project root",
                json!({"cmd": "chgrp -R group /project/tmp/build"}),
            ),
            (
                "nonrecursive chmod of the project directory",
                json!({"cmd": "chmod u+rwX /project"}),
            ),
            (
                "nonrecursive chown of the project directory",
                json!({"cmd": "chown user:group /project"}),
            ),
            (
                "chmod reading the project as a reference for an unrelated target",
                json!({"cmd": "chmod -R --reference /project /tmp/unrelated"}),
            ),
            (
                "chown reading the project as a reference for an unrelated target",
                json!({"cmd": "chown -R --reference=/project /tmp/unrelated"}),
            ),
            (
                "chgrp reading the project as a reference for an unrelated target",
                json!({"cmd": "chgrp --recursive --reference /project /tmp/unrelated"}),
            ),
            (
                "env option value named rm before an ordinary command",
                json!({"cmd": "env -u rm printf safe"}),
            ),
            (
                "sudo option value named rm before an ordinary command",
                json!({"cmd": "sudo -u rm printf safe"}),
            ),
            (
                "sudo edit mode does not treat a file as a command",
                json!({"cmd": "sudo -e /project"}),
            ),
            (
                "sudo list mode does not execute command-shaped arguments",
                json!({"cmd": "sudo -l rm -rf /project"}),
            ),
            (
                "rcfile option without a command-mode flag",
                json!({"cmd": "bash --rcfile -c 'rm -rf /project'"}),
            ),
        ];
        for (name, value) in shell_cases {
            assert!(
                pre_tool_use_output(&tool_input("exec_command", value)).is_none(),
                "ordinary operation was overblocked: {name}"
            );
        }

        assert!(
            pre_tool_use_output(&tool_input(
                "mcp__filesystem__delete_directory",
                json!({"path": "/project/tmp/build", "recursive": true}),
            ))
            .is_none(),
            "structured deletion below the managed root must remain available"
        );
        assert!(
            pre_tool_use_output(&tool_input(
                "mcp__filesystem__copy_directory",
                json!({"source_path": "/project", "destination_path": "/archive/project"}),
            ))
            .is_none(),
            "copying the project directory is not destructive"
        );
        assert!(
            pre_tool_use_output(&tool_input(
                "mcp__filesystem__move_directory",
                json!({
                    "source_path": "/tmp/note.txt",
                    "destination_path": "/project",
                }),
            ))
            .is_none(),
            "an ordinary structured move into the project directory is not replacement"
        );
    }

    #[test]
    fn ancestor_scope_guard_decodes_posix_literal_paths_with_spaces() {
        let root = "/project with spaces";
        let denied = [
            "rm -rf '/project with spaces'",
            "find '/project with spaces' -type f -delete",
            "chmod -R 000 '/project with spaces'",
            "chown --recursive user:group '/project with spaces'",
            "chgrp -vR group '/project with spaces'",
        ];
        for command in denied {
            assert!(
                pre_tool_use_output_bound(
                    &tool_input_at("exec_command", json!({"cmd": command}), root),
                    root,
                )
                .is_some(),
                "POSIX literal ancestor path was not denied: {command}"
            );
        }

        let allowed = [
            "find '/project with spaces/subtree' -delete",
            "chmod -R --reference '/project with spaces' /tmp/unrelated",
            "chown -R --reference='/project with spaces' /tmp/unrelated",
            "chgrp --recursive --reference '/project with spaces' /tmp/unrelated",
            "chmod -R --reference '/project with spaces/.mobius/mobius.sqlite3' /tmp/unrelated",
        ];
        for command in allowed {
            assert!(
                pre_tool_use_output(&tool_input_at(
                    "exec_command",
                    json!({"cmd": command}),
                    root,
                ))
                .is_none(),
                "read-only reference or unrelated target was overblocked: {command}"
            );
        }

        for command in [
            "chmod -R --reference /tmp/reference '/project with spaces'",
            "chown -R --reference=/tmp/reference '/project with spaces'",
            "chgrp --recursive --reference /tmp/reference '/project with spaces'",
            "chmod -R --reference /tmp/reference '/project with spaces/.mobius/mobius.sqlite3'",
        ] {
            assert!(
                pre_tool_use_output_bound(
                    &tool_input_at("exec_command", json!({"cmd": command}), root),
                    root,
                )
                .is_some(),
                "recursive metadata target was not denied: {command}"
            );
        }
    }

    #[test]
    fn redirection_lexer_distinguishes_path_targets_from_file_descriptors() {
        let protected = ".mobius/mobius.sqlite3";
        for command in [
            "printf x >&.mobius/mobius.sqlite3",
            "printf x >& .mobius/mobius.sqlite3",
            "printf x &>.mobius/mobius.sqlite3",
            "printf x &> .mobius/mobius.sqlite3",
            "printf x &>>.mobius/mobius.sqlite3",
            "printf x &>> .mobius/mobius.sqlite3",
        ] {
            assert_eq!(
                redirection_destinations(command),
                vec![protected],
                "path redirection was not recognized: {command}"
            );
        }

        for command in [
            "printf x 2>&1",
            "printf x >&1",
            "printf x 2>&-",
            "printf x >&-",
        ] {
            assert!(
                redirection_destinations(command).is_empty(),
                "fd duplication or close was treated as a path: {command}"
            );
        }
        assert_eq!(
            redirection_destinations("printf x >& debug.txt"),
            vec!["debug.txt"]
        );
        assert_eq!(
            redirection_destinations("printf x &> debug.txt"),
            vec!["debug.txt"]
        );
    }

    #[test]
    fn pre_tool_use_leaves_views_and_ordinary_files_alone() {
        assert!(
            pre_tool_use_output(&tool_input(
                "apply_patch",
                json!({"command": "*** Update File: src/main.rs"}),
            ))
            .is_none()
        );
        assert!(
            pre_tool_use_output(&tool_input(
                "apply_patch",
                json!({"command": "*** Update File: .mobius/views/current"}),
            ))
            .is_none()
        );
    }

    #[test]
    fn official_codex_wire_accepts_apply_patch_command_and_nullable_stop_message() {
        let pre_tool_use: PreToolUseInput = serde_json::from_value(json!({
            "session_id": "session-1",
            "transcript_path": "/tmp/transcript.jsonl",
            "cwd": "/project",
            "permission_mode": "default",
            "hook_event_name": "PreToolUse",
            "tool_name": "apply_patch",
            "tool_input": {
                "command": concat!(
                    "*** Begin Patch\n",
                    "*** Update File: .mobius/mobius.sqlite3\n",
                    "@@\n",
                    "+tamper\n",
                    "*** End Patch",
                ),
            },
            "tool_use_id": "tool-1",
        }))
        .expect("official PreToolUse wire input must deserialize");
        assert_eq!(pre_tool_use.cwd.as_deref(), Some(Path::new("/project")));
        assert!(
            pre_tool_use_output(&pre_tool_use).is_some(),
            "the official apply_patch command field must be inspected"
        );

        let stop: StopInput = serde_json::from_value(json!({
            "session_id": "session-1",
            "transcript_path": "/tmp/transcript.jsonl",
            "cwd": "/project",
            "permission_mode": "default",
            "hook_event_name": "Stop",
            "stop_hook_active": false,
            "last_assistant_message": null,
        }))
        .expect("official Stop wire input with a null message must deserialize");
        assert!(
            stop_output_with(&stop, |_| panic!("a null message has no completion claim")).is_none()
        );
    }

    #[test]
    fn stop_only_gates_an_exact_final_completion_marker() {
        let no_claim = stop_input("Objective objective-1 looks complete.");
        assert!(
            stop_output_with(&no_claim, |_| panic!(
                "verification must not run without a marker"
            ))
            .is_none()
        );

        let claim = stop_input("Summary.\n\nMOBIUS_OBJECTIVE_ACHIEVED: objective-1\n");
        assert!(stop_output_with(&claim, |_| Ok(true)).is_none());
        let blocked = stop_output_with(&claim, |_| Ok(false)).expect("false claim must block");
        assert_eq!(blocked["decision"], "block");
    }

    #[test]
    fn stop_avoids_recursive_blocking_and_blocks_unverifiable_claims() {
        let mut input = stop_input("MOBIUS_OBJECTIVE_ACHIEVED: objective-1");
        input.stop_hook_active = true;
        assert!(
            stop_output_with(&input, |_| panic!("active stop hook must not verify again"))
                .is_none()
        );

        input.stop_hook_active = false;
        let blocked = stop_output_with(&input, |_| Err("Core unavailable".to_owned()))
            .expect("unverifiable claim must block");
        assert!(
            blocked["reason"]
                .as_str()
                .unwrap()
                .contains("Core unavailable")
        );
    }
}
