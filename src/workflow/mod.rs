use crate::args::{Cli, ParsedInvocation};
use crate::error::{RosWireError, RosWireResult};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const SCRIPT_PUT_PLAN_SCHEMA_VERSION: &str = "roswire.workflow.script.put.plan.v1";
const MAX_SCRIPT_SOURCE_BYTES: u64 = 256 * 1024;

#[derive(Debug, Default)]
pub struct WorkflowModule;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowResult {
    Payload(String),
    Invocation(ParsedInvocation),
}

#[derive(Debug, Serialize)]
struct ScriptPutPlan {
    schema_version: &'static str,
    operation: &'static str,
    dry_run: bool,
    script_name: String,
    source_path: String,
    source_bytes: u64,
    routeros_command: &'static str,
    routeros_rest_path: &'static str,
    side_effects: Vec<&'static str>,
    routeros_file_created: bool,
    content_redacted: bool,
}

pub fn handle(tokens: &[String], cli: &Cli) -> Option<RosWireResult<WorkflowResult>> {
    match tokens {
        [script, put, name] if script == "script" && put == "put" => {
            Some(handle_script_put(name, cli))
        }
        [script, put, ..] if script == "script" && put == "put" => Some(Err(Box::new(
            RosWireError::usage(
                "script put requires exactly one script name: roswire script put <name> --source @<local.rsc>",
            ),
        ))),
        _ => None,
    }
}

fn handle_script_put(name: &str, cli: &Cli) -> RosWireResult<WorkflowResult> {
    if name.trim().is_empty() {
        return Err(Box::new(RosWireError::usage(
            "script put requires a non-empty script name",
        )));
    }

    let source_path = source_path_from_cli(cli)?;
    if cli.dry_run {
        return script_put_plan(name, &source_path).map(WorkflowResult::Payload);
    }

    let source = read_script_source(&source_path)?;
    Ok(WorkflowResult::Invocation(ParsedInvocation {
        path: vec!["system".to_owned(), "script".to_owned()],
        action: "add".to_owned(),
        resolved_args: BTreeMap::from([
            ("name".to_owned(), name.to_owned()),
            ("source".to_owned(), source),
        ]),
    }))
}

fn script_put_plan(name: &str, source_path: &Path) -> RosWireResult<String> {
    let source_bytes = validated_script_file_len(source_path)?;
    render_json(&ScriptPutPlan {
        schema_version: SCRIPT_PUT_PLAN_SCHEMA_VERSION,
        operation: "script.put",
        dry_run: true,
        script_name: name.to_owned(),
        source_path: redact_local_path(source_path),
        source_bytes,
        routeros_command: "/system/script/add",
        routeros_rest_path: "/rest/system/script",
        side_effects: vec!["creates-routeros-script"],
        routeros_file_created: false,
        content_redacted: true,
    })
}

fn source_path_from_cli(cli: &Cli) -> RosWireResult<PathBuf> {
    let source = cli.source.as_deref().ok_or_else(|| {
        Box::new(RosWireError::usage(
            "missing script source: use --source @<local.rsc>",
        ))
    })?;

    let path = source.strip_prefix('@').ok_or_else(|| {
        Box::new(RosWireError::usage(
            "script source must reference a local file with @<local.rsc>",
        ))
    })?;

    if path.trim().is_empty() {
        return Err(Box::new(RosWireError::usage(
            "script source path cannot be empty",
        )));
    }

    Ok(PathBuf::from(path))
}

fn read_script_source(path: &Path) -> RosWireResult<String> {
    validated_script_file_len(path)?;
    let bytes = fs::read(path).map_err(|error| {
        Box::new(RosWireError::usage(format!(
            "failed to read script source {}: {error}",
            redact_local_path(path),
        )))
    })?;

    if bytes.len() as u64 > MAX_SCRIPT_SOURCE_BYTES {
        return Err(Box::new(RosWireError::file_too_large(format!(
            "script source {} exceeds {} bytes",
            redact_local_path(path),
            MAX_SCRIPT_SOURCE_BYTES,
        ))));
    }

    String::from_utf8(bytes).map_err(|error| {
        Box::new(
            RosWireError::usage(format!(
                "script source {} must be UTF-8 text: {error}",
                redact_local_path(path),
            ))
            .with_hint("save the .rsc file as UTF-8 text"),
        )
    })
}

fn validated_script_file_len(path: &Path) -> RosWireResult<u64> {
    let metadata = fs::metadata(path).map_err(|error| {
        Box::new(RosWireError::usage(format!(
            "failed to inspect script source {}: {error}",
            redact_local_path(path),
        )))
    })?;

    if !metadata.is_file() {
        return Err(Box::new(RosWireError::usage(format!(
            "script source must be a file: {}",
            redact_local_path(path),
        ))));
    }

    let len = metadata.len();
    if len > MAX_SCRIPT_SOURCE_BYTES {
        return Err(Box::new(RosWireError::file_too_large(format!(
            "script source {} exceeds {} bytes",
            redact_local_path(path),
            MAX_SCRIPT_SOURCE_BYTES,
        ))));
    }

    Ok(len)
}

fn redact_local_path(path: &Path) -> String {
    if path.is_absolute() {
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("source.rsc");
        format!("***REDACTED***/{file_name}")
    } else {
        path.display().to_string()
    }
}

fn render_json<T: Serialize>(value: &T) -> RosWireResult<String> {
    serde_json::to_string(value).map_err(|error| {
        Box::new(RosWireError::internal(format!(
            "failed to serialize workflow payload: {error}",
        )))
    })
}

#[cfg(test)]
mod tests {
    use super::{handle, WorkflowResult, MAX_SCRIPT_SOURCE_BYTES};
    use crate::args::Cli;
    use crate::error::ErrorCode;
    use clap::Parser;
    use std::fs;

    #[test]
    fn script_put_dry_run_plan_redacts_path_and_content() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let source = temp.path().join("bootstrap.rsc");
        let script = ":put \"VERY_SECRET_SCRIPT\"";
        fs::write(&source, script).expect("source should be written");
        let source_arg = format!("@{}", source.display());
        let cli = Cli::try_parse_from([
            "roswire",
            "script",
            "put",
            "bootstrap",
            "--source",
            &source_arg,
            "--dry-run",
            "--json",
        ])
        .expect("cli should parse");

        let result = handle(&cli.tokens, &cli)
            .expect("workflow should match")
            .expect("dry-run should succeed");

        let WorkflowResult::Payload(payload) = result else {
            panic!("dry-run should return payload");
        };
        assert!(payload.contains("roswire.workflow.script.put.plan.v1"));
        assert!(payload.contains("\"script_name\":\"bootstrap\""));
        assert!(payload.contains("***REDACTED***/bootstrap.rsc"));
        assert!(payload.contains("\"routeros_file_created\":false"));
        assert!(!payload.contains(temp.path().to_string_lossy().as_ref()));
        assert!(!payload.contains(script));
    }

    #[test]
    fn script_put_actual_invocation_reads_utf8_source() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let source = temp.path().join("bootstrap.rsc");
        let script = ":put \"hello\"\n";
        fs::write(&source, script).expect("source should be written");
        let source_arg = format!("@{}", source.display());
        let cli = Cli::try_parse_from([
            "roswire",
            "script",
            "put",
            "bootstrap",
            "--source",
            &source_arg,
            "--json",
        ])
        .expect("cli should parse");

        let result = handle(&cli.tokens, &cli)
            .expect("workflow should match")
            .expect("script put should succeed");

        let WorkflowResult::Invocation(invocation) = result else {
            panic!("actual script put should return invocation");
        };
        assert_eq!(invocation.path, vec!["system", "script"]);
        assert_eq!(invocation.action, "add");
        assert_eq!(
            invocation.resolved_args.get("name").map(String::as_str),
            Some("bootstrap"),
        );
        assert_eq!(
            invocation.resolved_args.get("source").map(String::as_str),
            Some(script),
        );
    }

    #[test]
    fn script_put_requires_at_source_path() {
        let cli = Cli::try_parse_from([
            "roswire",
            "script",
            "put",
            "bootstrap",
            "--source",
            "setup.rsc",
            "--json",
        ])
        .expect("cli should parse");

        let error = handle(&cli.tokens, &cli)
            .expect("workflow should match")
            .expect_err("source should require @ prefix");

        assert_eq!(error.error_code, ErrorCode::UsageError);
        assert!(error.message.contains("@<local.rsc>"));
    }

    #[test]
    fn script_put_reports_too_large_without_reading_content() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let source = temp.path().join("large.rsc");
        fs::write(&source, vec![b'a'; MAX_SCRIPT_SOURCE_BYTES as usize + 1])
            .expect("large source should be written");
        let source_arg = format!("@{}", source.display());
        let cli = Cli::try_parse_from([
            "roswire",
            "script",
            "put",
            "bootstrap",
            "--source",
            &source_arg,
            "--json",
        ])
        .expect("cli should parse");

        let error = handle(&cli.tokens, &cli)
            .expect("workflow should match")
            .expect_err("large source should fail");

        assert_eq!(error.error_code, ErrorCode::FileTooLarge);
        assert!(!error
            .message
            .contains(temp.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn script_put_rejects_non_utf8_source() {
        let temp = tempfile::tempdir().expect("temp dir should be created");
        let source = temp.path().join("binary.rsc");
        fs::write(&source, [0xff, 0xfe]).expect("binary source should be written");
        let source_arg = format!("@{}", source.display());
        let cli = Cli::try_parse_from([
            "roswire",
            "script",
            "put",
            "bootstrap",
            "--source",
            &source_arg,
            "--json",
        ])
        .expect("cli should parse");

        let error = handle(&cli.tokens, &cli)
            .expect("workflow should match")
            .expect_err("non-utf8 source should fail");

        assert_eq!(error.error_code, ErrorCode::UsageError);
        assert!(error.message.contains("UTF-8"));
        assert!(!error
            .message
            .contains(temp.path().to_string_lossy().as_ref()));
    }
}
