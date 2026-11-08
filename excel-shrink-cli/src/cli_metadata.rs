//! Machine-readable options derived from the same Clap definition as cleanup.

use clap::{ArgAction, CommandFactory};
use serde_json::{json, Value};
use std::any::TypeId;

/// Describe supported scalar, enum and repeated options without parsing help text.
/// Configuration forms supply positional input and output paths separately.
pub(crate) fn describe() -> Value {
    let mut command = crate::Cli::command();
    command.build();
    let mut options = Vec::new();
    for argument in command.get_arguments() {
        let Some(long) = argument.get_long() else {
            continue;
        };
        if argument.is_hide_set() || matches!(long, "help" | "version") {
            continue;
        }
        let boolean = matches!(
            argument.get_action(),
            ArgAction::SetTrue | ArgAction::SetFalse
        );
        let parser = argument.get_value_parser();
        let id = parser.type_id();
        let integer_types = [
            TypeId::of::<usize>(),
            TypeId::of::<u64>(),
            TypeId::of::<u32>(),
            TypeId::of::<i32>(),
            TypeId::of::<i64>(),
        ];
        let integer = integer_types.iter().any(|candidate| id == *candidate);
        let choices: Vec<_> = parser
            .possible_values()
            .into_iter()
            .flatten()
            .filter(|value| !value.is_hide_set())
            .map(|value| value.get_name().to_owned())
            .collect();
        let repeated = matches!(argument.get_action(), ArgAction::Append);
        let kind = if boolean {
            "boolean"
        } else if repeated {
            "list"
        } else if !choices.is_empty() {
            "enum"
        } else if integer {
            "integer"
        } else {
            "string"
        };
        let defaults: Vec<_> = argument
            .get_default_values()
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect();
        options.push(json!({
            "name":long,
            "help":argument.get_help().map(ToString::to_string),
            "kind":kind,
            "choices":choices,
            "defaults":defaults,
        }));
    }
    json!({
        "schema": 1,
        "program": "excel-shrink-cli",
        "version": env!("CARGO_PKG_VERSION"),
        "options": options,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_preserves_types_and_defaults_from_the_cleanup_cli() {
        let schema = describe();
        let options = schema["options"].as_array().unwrap();
        let option = |name: &str| {
            options
                .iter()
                .find(|option| option["name"] == name)
                .unwrap()
        };
        assert_eq!(option("rayon-threads")["kind"], "integer");
        assert_eq!(option("force-overwrite")["kind"], "boolean");
        assert_eq!(option("sheets")["kind"], "list");
        assert_eq!(option("compression-level")["defaults"], json!(["2"]));
        assert_eq!(option("console-log-level")["kind"], "enum");
    }
}
