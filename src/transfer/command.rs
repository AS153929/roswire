use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TransferCommand {
    FileUpload { local: String, remote: String },
    FileDownload { remote: String, local: String },
    Import { local: String },
    BackupDownload { local: String },
    ExportDownload { local: String },
}

impl TransferCommand {
    pub(super) fn operation(&self) -> &'static str {
        match self {
            Self::FileUpload { .. } => "file.upload",
            Self::FileDownload { .. } => "file.download",
            Self::Import { .. } => "import.plan",
            Self::BackupDownload { .. } => "backup.download",
            Self::ExportDownload { .. } => "export.download",
        }
    }

    pub(super) fn command_name(&self) -> &'static str {
        match self {
            Self::FileUpload { .. } => "file/upload",
            Self::FileDownload { .. } => "file/download",
            Self::Import { .. } => "import",
            Self::BackupDownload { .. } => "backup/download",
            Self::ExportDownload { .. } => "export/download",
        }
    }

    pub(super) fn context_args(&self) -> BTreeMap<String, String> {
        match self {
            Self::FileUpload { local, remote } => BTreeMap::from([
                ("local_path".to_owned(), redact_local_path(local)),
                ("remote_path".to_owned(), redact_remote_path(remote)),
            ]),
            Self::FileDownload { remote, local } => BTreeMap::from([
                ("remote_path".to_owned(), redact_remote_path(remote)),
                ("local_path".to_owned(), redact_local_path(local)),
            ]),
            Self::Import { local }
            | Self::BackupDownload { local }
            | Self::ExportDownload { local } => {
                BTreeMap::from([("local_path".to_owned(), redact_local_path(local))])
            }
        }
    }
}

pub(super) fn parse_transfer_command(tokens: &[String]) -> Option<RosWireResult<TransferCommand>> {
    match tokens {
        [file, action, local, remote] if file == "file" && action == "upload" => {
            Some(Ok(TransferCommand::FileUpload {
                local: local.clone(),
                remote: remote.clone(),
            }))
        }
        [file, action, remote, local] if file == "file" && action == "download" => {
            Some(Ok(TransferCommand::FileDownload {
                remote: remote.clone(),
                local: local.clone(),
            }))
        }
        [command, local] if command == "import" => Some(Ok(TransferCommand::Import {
            local: local.clone(),
        })),
        [command, action, local] if command == "backup" && action == "download" => {
            Some(Ok(TransferCommand::BackupDownload {
                local: local.clone(),
            }))
        }
        [command, action, local] if command == "export" && action == "download" => {
            Some(Ok(TransferCommand::ExportDownload {
                local: local.clone(),
            }))
        }
        [command, ..] if matches!(command.as_str(), "file" | "import" | "backup" | "export") => {
            Some(Err(Box::new(RosWireError::usage(
                "transfer commands require one of: file upload <local> <remote>, file download <remote> <local>, import <local>, backup download <local>, export download <local>",
            ))))
        }
        _ => None,
    }
}

#[cfg(test)]
pub(super) fn parse_port(value: &str) -> RosWireResult<u16> {
    value.parse::<u16>().map_err(|error| {
        Box::new(RosWireError::usage(format!(
            "invalid SSH port value `{value}`: {error}",
        )))
    })
}

pub(super) fn transfer_description(command: &TransferCommand, cli: &Cli) -> String {
    match command {
        TransferCommand::FileUpload { .. } => {
            "Upload local file to temporary remote path, then move into final remote path"
                .to_owned()
        }
        TransferCommand::FileDownload { .. } => {
            "Download remote file to a temporary local path, then move into final local path"
                .to_owned()
        }
        TransferCommand::Import { .. } => {
            "Upload local .rsc to a temporary remote path, then execute /import file-name=<temp>"
                .to_owned()
        }
        TransferCommand::BackupDownload { .. } => {
            "Execute /system/backup/save name=<name>, wait for .backup, then download".to_owned()
        }
        TransferCommand::ExportDownload { .. } if cli.compact => {
            "Execute compact /export file=<name>, wait for .rsc, then download".to_owned()
        }
        TransferCommand::ExportDownload { .. } => {
            "Execute /export file=<name>, wait for .rsc, then download".to_owned()
        }
    }
}
