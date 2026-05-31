use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TransferPolicy {
    pub(super) if_exists: TransferIfExists,
    pub(super) timeouts: TransferTimeouts,
    pub(super) retry: TransferRetryPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TransferTimeouts {
    pub(super) connect_seconds: u64,
    pub(super) wait_remote_file_seconds: u64,
    pub(super) transfer_seconds: u64,
    pub(super) cleanup_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TransferRetryPolicy {
    pub(super) max_retries: u8,
    pub(super) delay_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TransferRetryPlan {
    pub(super) max_retries: u8,
    pub(super) delay_seconds: u64,
    pub(super) retryable_error_codes: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TransferPolicyPlan {
    pub(super) if_exists: String,
    pub(super) timeouts: TransferTimeouts,
    pub(super) retry: TransferRetryPlan,
}

impl TransferPolicy {
    pub(super) fn plan(&self) -> TransferPolicyPlan {
        TransferPolicyPlan {
            if_exists: self.if_exists.as_str().to_owned(),
            timeouts: self.timeouts.clone(),
            retry: TransferRetryPlan {
                max_retries: self.retry.max_retries,
                delay_seconds: self.retry.delay_seconds,
                retryable_error_codes: vec![
                    "NETWORK_ERROR",
                    "FILE_TRANSFER_FAILED",
                    "ROS_API_FAILURE",
                ],
            },
        }
    }

    pub(super) fn connect_timeout(&self) -> Duration {
        Duration::from_secs(self.timeouts.connect_seconds)
    }

    pub(super) fn transfer_timeout(&self) -> Duration {
        Duration::from_secs(self.timeouts.transfer_seconds)
    }

    pub(super) fn wait_timeout(&self) -> Duration {
        Duration::from_secs(self.timeouts.wait_remote_file_seconds)
    }

    pub(super) fn retry_delay(&self) -> Duration {
        Duration::from_secs(self.retry.delay_seconds)
    }
}

pub(super) fn resolve_allow_from(
    cli: &Cli,
    profile: Option<&config::ProfileConfig>,
) -> RosWireResult<Vec<String>> {
    let values = if !cli.allow_from.is_empty() {
        cli.allow_from.clone()
    } else {
        profile
            .map(|profile| profile.allow_from.clone())
            .unwrap_or_default()
    };

    let mut cidrs = Vec::new();
    for value in values {
        for cidr in value
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            validate_safe_cidr(cidr)?;
            cidrs.push(cidr.to_owned());
        }
    }

    Ok(cidrs)
}

pub(super) fn resolve_allow_from_for_runtime(
    cli: &Cli,
    profile: Option<&config::ProfileConfig>,
    context: &ErrorContext,
) -> RosWireResult<Vec<String>> {
    if !cli.ensure_ssh {
        return Ok(Vec::new());
    }

    let allow_from = resolve_allow_from(cli, profile)
        .map_err(|error| Box::new((*error).clone().with_context(context.clone())))?;
    if allow_from.is_empty() {
        return Err(Box::new(
            RosWireError::ssh_whitelist_required(
                "--ensure-ssh requires at least one allow-from CIDR from --allow-from or profile allow_from to merge into /ip service ssh address",
            )
            .with_context(context.clone()),
        ));
    }
    Ok(allow_from)
}

pub(super) fn validate_safe_cidr(cidr: &str) -> RosWireResult<()> {
    let (addr, prefix) = cidr.split_once('/').ok_or_else(|| {
        Box::new(RosWireError::usage(format!(
            "allow-from must be CIDR notation: {cidr}",
        )))
    })?;
    let address = addr.parse::<IpAddr>().map_err(|error| {
        Box::new(RosWireError::usage(format!(
            "invalid allow-from address `{addr}`: {error}",
        )))
    })?;
    let prefix = prefix.parse::<u8>().map_err(|error| {
        Box::new(RosWireError::usage(format!(
            "invalid allow-from prefix `{prefix}`: {error}",
        )))
    })?;

    match address {
        IpAddr::V4(_) if prefix > 32 => Err(Box::new(RosWireError::usage(format!(
            "invalid IPv4 allow-from prefix: {prefix}",
        )))),
        IpAddr::V4(_) if prefix < 24 => Err(Box::new(RosWireError::ssh_whitelist_unsafe(
            "SSH allow-from IPv4 CIDR is too broad",
        ))),
        IpAddr::V6(_) if prefix > 128 => Err(Box::new(RosWireError::usage(format!(
            "invalid IPv6 allow-from prefix: {prefix}",
        )))),
        IpAddr::V6(_) if prefix < 64 => Err(Box::new(RosWireError::ssh_whitelist_unsafe(
            "SSH allow-from IPv6 CIDR is too broad",
        ))),
        _ => Ok(()),
    }
}

pub(super) fn transfer_policy(cli: &Cli) -> RosWireResult<TransferPolicy> {
    if cli.retries > MAX_TRANSFER_RETRIES {
        return Err(Box::new(RosWireError::usage(format!(
            "--retries must be <= {MAX_TRANSFER_RETRIES}"
        ))));
    }

    let mut policy = default_transfer_policy();
    policy.if_exists = cli.if_exists;
    policy.timeouts.connect_seconds = cli
        .connect_timeout_seconds
        .unwrap_or(DEFAULT_CONNECT_TIMEOUT_SECONDS);
    policy.timeouts.wait_remote_file_seconds = cli
        .wait_timeout_seconds
        .unwrap_or(DEFAULT_WAIT_TIMEOUT_SECONDS);
    policy.timeouts.transfer_seconds = cli
        .transfer_timeout_seconds
        .unwrap_or(DEFAULT_TRANSFER_TIMEOUT_SECONDS);
    policy.timeouts.cleanup_seconds = cli
        .cleanup_timeout_seconds
        .unwrap_or(DEFAULT_CLEANUP_TIMEOUT_SECONDS);
    policy.retry.max_retries = cli.retries;
    policy.retry.delay_seconds = cli.retry_delay_seconds;

    Ok(policy)
}

pub(super) fn default_transfer_policy() -> TransferPolicy {
    TransferPolicy {
        if_exists: TransferIfExists::Overwrite,
        timeouts: TransferTimeouts {
            connect_seconds: DEFAULT_CONNECT_TIMEOUT_SECONDS,
            wait_remote_file_seconds: DEFAULT_WAIT_TIMEOUT_SECONDS,
            transfer_seconds: DEFAULT_TRANSFER_TIMEOUT_SECONDS,
            cleanup_seconds: DEFAULT_CLEANUP_TIMEOUT_SECONDS,
        },
        retry: TransferRetryPolicy {
            max_retries: 0,
            delay_seconds: 0,
        },
    }
}

pub(super) fn retryable_transfer_error(error: &RosWireError) -> bool {
    matches!(
        error.error_code,
        error::ErrorCode::NetworkError
            | error::ErrorCode::FileTransferFailed
            | error::ErrorCode::RosApiFailure
    )
}

pub(super) fn retry_transfer_step<T, F>(
    policy: &TransferPolicy,
    mut operation: F,
) -> RosWireResult<T>
where
    F: FnMut() -> RosWireResult<T>,
{
    let mut retries_used = 0;
    loop {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error)
                if retries_used < policy.retry.max_retries && retryable_transfer_error(&error) =>
            {
                retries_used += 1;
                let delay = policy.retry_delay();
                if !delay.is_zero() {
                    thread::sleep(delay);
                }
            }
            Err(error) => return Err(error),
        }
    }
}
