use std::ffi::OsString;

use crate::error::MobiusError;

pub(crate) mod cli;
pub(crate) mod hooks;
pub(crate) mod mcp;

const USAGE: &str = "Mobius v1.2.0\n\nUsage:\n  mobius mcp\n  mobius audit ...\n  mobius doctor ...\n  mobius report ...\n  mobius hook pre-tool-use\n  mobius hook stop\n";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Mcp,
    Audit,
    Doctor,
    Report,
    Hook,
}

pub(crate) fn run(arguments: impl Iterator<Item = OsString>) -> Result<(), MobiusError> {
    let arguments = arguments.collect::<Vec<_>>();
    let Some(first) = arguments.first() else {
        return Err(MobiusError::invalid_invocation(
            "missing mode; run `mobius --help` for the supported adapters",
        ));
    };
    let first = first
        .to_str()
        .ok_or_else(|| MobiusError::invalid_invocation("mode must be valid UTF-8"))?;

    if matches!(first, "--help" | "-h" | "help") {
        print!("{USAGE}");
        return Ok(());
    }

    let mode = parse_mode(first)?;
    let tail = &arguments[1..];
    match mode {
        Mode::Mcp => mcp::run(tail),
        Mode::Audit | Mode::Doctor | Mode::Report => cli::run(first, tail),
        Mode::Hook => hooks::run(tail),
    }
}

fn parse_mode(value: &str) -> Result<Mode, MobiusError> {
    match value {
        "mcp" => Ok(Mode::Mcp),
        "audit" => Ok(Mode::Audit),
        "doctor" => Ok(Mode::Doctor),
        "report" => Ok(Mode::Report),
        "hook" => Ok(Mode::Hook),
        value => Err(MobiusError::invalid_invocation(format!(
            "unknown mode `{value}`; run `mobius --help`"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_dispatch_is_closed_over_the_blueprint_surface() {
        let accepted = ["mcp", "audit", "doctor", "report", "hook"];
        assert!(accepted.into_iter().all(|mode| parse_mode(mode).is_ok()));
        assert!(parse_mode("read").is_err());
        assert!(parse_mode("mutate").is_err());
        assert!(parse_mode("mark-complete").is_err());
    }
}
