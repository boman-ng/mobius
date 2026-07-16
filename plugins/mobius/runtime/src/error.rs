use std::fmt::{self, Display, Formatter};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MobiusError {
    code: &'static str,
    message: String,
    exit_code: u8,
}

impl MobiusError {
    pub(crate) fn invalid_invocation(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_invocation",
            message: message.into(),
            exit_code: 2,
        }
    }

    pub(crate) fn operation(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            exit_code: 1,
        }
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::operation("internal_error", message)
    }

    pub(crate) fn exit_code(&self) -> u8 {
        self.exit_code
    }

    pub(crate) fn to_json(&self) -> String {
        format!(
            "{{\"schema\":\"mobius.error.v1\",\"code\":\"{}\",\"message\":\"{}\"}}",
            escape_json(self.code),
            escape_json(&self.message)
        )
    }
}

impl Display for MobiusError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for MobiusError {}

fn escape_json(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                use std::fmt::Write as _;
                write!(escaped, "\\u{:04x}", character as u32)
                    .expect("writing to a String cannot fail");
            }
            character => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_machine_readable_without_losing_control_characters() {
        let error = MobiusError::invalid_invocation("bad\n\"input\"");
        assert_eq!(
            error.to_json(),
            "{\"schema\":\"mobius.error.v1\",\"code\":\"invalid_invocation\",\"message\":\"bad\\n\\\"input\\\"\"}"
        );
    }
}
