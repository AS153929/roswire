use crate::args::Cli;
use crate::config::{self, ProfileConfig};
use crate::error::{ErrorCode, ErrorContext, RosWireError, RosWireResult};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::io::AsRawFd;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(windows)]
use std::os::windows::io::AsRawSocket;

const DEFAULT_JUMP_PORT: u16 = 22;
const ORIGINATOR_HOST: &str = "0.0.0.0";
const ORIGINATOR_PORT: u16 = 0;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JumpHopIdentity {
    pub host: String,
    pub port: u16,
    pub user: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DialStep {
    pub kind: DialKind,
    pub dest_host: String,
    pub dest_port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DialKind {
    TcpConnect,
    DirectTcpIp,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedJumpHop {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Option<String>,
    pub key_path: Option<String>,
    pub key_passphrase: Option<String>,
    pub expected_host_key: String,
}

impl std::fmt::Debug for ResolvedJumpHop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedJumpHop")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field(
                "password",
                &self.password.as_ref().map(|_| "***REDACTED***"),
            )
            .field(
                "key_path",
                &self.key_path.as_ref().map(|_| "***REDACTED***"),
            )
            .field(
                "key_passphrase",
                &self.key_passphrase.as_ref().map(|_| "***REDACTED***"),
            )
            .field("expected_host_key", &self.expected_host_key)
            .finish()
    }
}

impl ResolvedJumpHop {
    pub fn identity(&self) -> JumpHopIdentity {
        JumpHopIdentity {
            host: self.host.clone(),
            port: self.port,
            user: self.user.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct JumpViaPlan {
    pub kind: &'static str,
    pub hops: Vec<JumpHopIdentity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_port: Option<u16>,
    pub teardown: &'static str,
    pub local_bind: bool,
}

impl JumpViaPlan {
    pub fn from_hops(
        hops: Vec<JumpHopIdentity>,
        target_host: Option<String>,
        target_port: Option<u16>,
    ) -> Option<Self> {
        if hops.is_empty() {
            return None;
        }
        Some(Self {
            kind: "jump",
            hops,
            target_host,
            target_port,
            teardown: "process-exit",
            local_bind: false,
        })
    }
}

pub fn plan_dials(hops: &[JumpHopIdentity], target_host: &str, target_port: u16) -> Vec<DialStep> {
    if hops.is_empty() {
        return Vec::new();
    }

    let mut steps = Vec::with_capacity(hops.len() + 1);
    let first = &hops[0];
    steps.push(DialStep {
        kind: DialKind::TcpConnect,
        dest_host: first.host.clone(),
        dest_port: first.port,
    });
    for hop in hops.iter().skip(1) {
        steps.push(DialStep {
            kind: DialKind::DirectTcpIp,
            dest_host: hop.host.clone(),
            dest_port: hop.port,
        });
    }
    steps.push(DialStep {
        kind: DialKind::DirectTcpIp,
        dest_host: target_host.to_owned(),
        dest_port: target_port,
    });
    steps
}

pub fn identities_from_hops(hops: &[ResolvedJumpHop]) -> Vec<JumpHopIdentity> {
    hops.iter().map(ResolvedJumpHop::identity).collect()
}

pub fn resolve_jump_identities(
    cli: &Cli,
    profile: Option<&ProfileConfig>,
) -> RosWireResult<Vec<JumpHopIdentity>> {
    if let Some(host) = cli.jump_host.as_deref() {
        config::validate_remote_host(host)?;
        let user = cli.jump_user.clone().ok_or_else(|| {
            Box::new(RosWireError::config(
                "missing jump user; set --jump-user when --jump-host is set",
            ))
        })?;
        return Ok(vec![JumpHopIdentity {
            host: host.to_owned(),
            port: cli.jump_port.unwrap_or(DEFAULT_JUMP_PORT),
            user,
        }]);
    }

    let Some(profile) = profile else {
        return Ok(Vec::new());
    };
    let mut hops = Vec::new();
    for hop in &profile.jump {
        let Some(host) = hop.host.as_deref() else {
            continue;
        };
        config::validate_remote_host(host)?;
        let Some(user) = hop.user.clone() else {
            return Err(Box::new(RosWireError::config(
                "jump hop is missing user; set jump user in profile",
            )));
        };
        hops.push(JumpHopIdentity {
            host: host.to_owned(),
            port: hop.port.unwrap_or(DEFAULT_JUMP_PORT),
            user,
        });
    }
    Ok(hops)
}

pub fn resolve_jump_hops(
    cli: &Cli,
    env: &BTreeMap<String, String>,
    profile: Option<&ProfileConfig>,
) -> RosWireResult<Vec<ResolvedJumpHop>> {
    if cli.jump_host.is_some() {
        return Ok(vec![resolve_cli_hop(cli, env, profile)?]);
    }

    let Some(profile) = profile else {
        return Ok(Vec::new());
    };
    if profile.jump.is_empty() {
        return Ok(Vec::new());
    }

    let mut hops = Vec::with_capacity(profile.jump.len());
    for (index, hop) in profile.jump.iter().enumerate() {
        hops.push(resolve_profile_hop(profile, env, hop, index)?);
    }
    Ok(hops)
}

fn resolve_cli_hop(
    cli: &Cli,
    env: &BTreeMap<String, String>,
    profile: Option<&ProfileConfig>,
) -> RosWireResult<ResolvedJumpHop> {
    let host = cli.jump_host.clone().expect("jump_host is checked");
    config::validate_remote_host(&host)?;
    let user = cli.jump_user.clone().ok_or_else(|| {
        Box::new(RosWireError::config(
            "missing jump user; set --jump-user when --jump-host is set",
        ))
    })?;
    let expected_host_key = cli
        .jump_host_key
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            Box::new(RosWireError::jump_host_key_required(
                "SSH jump requires an expected jump host key fingerprint",
            ))
        })?;
    let key_path = cli
        .jump_key
        .clone()
        .filter(|value| !value.trim().is_empty());
    let (password, key_passphrase) = resolve_hop_secrets(cli, env, profile, 0, key_path.is_some())?;

    Ok(ResolvedJumpHop {
        host,
        port: cli.jump_port.unwrap_or(DEFAULT_JUMP_PORT),
        user,
        password,
        key_path,
        key_passphrase,
        expected_host_key,
    })
}

fn resolve_profile_hop(
    profile: &ProfileConfig,
    env: &BTreeMap<String, String>,
    hop: &config::JumpHopConfig,
    index: usize,
) -> RosWireResult<ResolvedJumpHop> {
    let host = hop.host.clone().ok_or_else(|| {
        Box::new(RosWireError::config(
            "jump hop is missing host; set jump host in profile",
        ))
    })?;
    config::validate_remote_host(&host)?;
    let user = hop.user.clone().ok_or_else(|| {
        Box::new(RosWireError::config(
            "jump hop is missing user; set jump user in profile",
        ))
    })?;
    let expected_host_key = hop
        .host_key
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            Box::new(RosWireError::jump_host_key_required(
                "SSH jump requires an expected jump host key fingerprint",
            ))
        })?;
    let key_path = hop.key.clone().filter(|value| !value.trim().is_empty());
    let (password, key_passphrase) =
        resolve_hop_secrets_from_profile(env, profile, index, key_path.is_some())?;

    Ok(ResolvedJumpHop {
        host,
        port: hop.port.unwrap_or(DEFAULT_JUMP_PORT),
        user,
        password,
        key_path,
        key_passphrase,
        expected_host_key,
    })
}

fn resolve_hop_secrets(
    cli: &Cli,
    env: &BTreeMap<String, String>,
    profile: Option<&ProfileConfig>,
    index: usize,
    has_key: bool,
) -> RosWireResult<(Option<String>, Option<String>)> {
    if has_key {
        let passphrase = profile
            .map(|profile| resolve_hop_secret_value(profile, env, index, "key_passphrase"))
            .transpose()?
            .flatten();
        return Ok((None, passphrase));
    }

    if let Some(password) = cli.jump_password.clone() {
        return Ok((Some(password), None));
    }

    let password = profile
        .map(|profile| resolve_hop_secret_value(profile, env, index, "password"))
        .transpose()?
        .flatten()
        .ok_or_else(|| {
            Box::new(RosWireError::config(
                "missing jump password; set --jump-password, --jump-key, or profile secret jump_password",
            ))
        })?;
    Ok((Some(password), None))
}

fn resolve_hop_secrets_from_profile(
    env: &BTreeMap<String, String>,
    profile: &ProfileConfig,
    index: usize,
    has_key: bool,
) -> RosWireResult<(Option<String>, Option<String>)> {
    if has_key {
        let passphrase = resolve_hop_secret_value(profile, env, index, "key_passphrase")?;
        return Ok((None, passphrase));
    }
    let password = resolve_hop_secret_value(profile, env, index, "password")?.ok_or_else(|| {
        Box::new(RosWireError::config(
            "missing jump password; set profile secret jump_password or jump key",
        ))
    })?;
    Ok((Some(password), None))
}

fn resolve_hop_secret_value(
    profile: &ProfileConfig,
    env: &BTreeMap<String, String>,
    index: usize,
    kind: &str,
) -> RosWireResult<Option<String>> {
    let indexed = format!("jump_{index}_{kind}");
    if let Some(value) = config::resolve_profile_secret_value(profile, &indexed, env)? {
        return Ok(Some(value));
    }
    let shared = format!("jump_{kind}");
    config::resolve_profile_secret_value(profile, &shared, env)
}

pub struct JumpIo {
    channel: Option<ssh2::Channel>,
    sessions: Vec<ssh2::Session>,
    pumps: Vec<JoinHandle<()>>,
    leftover: Arc<AtomicBool>,
    closed: bool,
    hops: Vec<JumpHopIdentity>,
    target_host: String,
    target_port: u16,
}

pub struct JumpSshSession {
    pub session: ssh2::Session,
    leftover: Arc<AtomicBool>,
    _pumps: Vec<JoinHandle<()>>,
    _sessions: Vec<ssh2::Session>,
}

impl JumpSshSession {
    pub fn leftover_handle(&self) -> Arc<AtomicBool> {
        self.leftover.clone()
    }
}

impl JumpIo {
    pub fn leftover_handle(&self) -> Arc<AtomicBool> {
        self.leftover.clone()
    }

    pub fn leftover(&self) -> bool {
        self.leftover.load(Ordering::SeqCst)
    }

    pub fn hops(&self) -> &[JumpHopIdentity] {
        &self.hops
    }

    pub fn audit_value(&self) -> Value {
        json!({
            "hops": self.hops,
            "target_host": self.target_host,
            "target_port": self.target_port,
            "local_bind": false,
            "leftover": self.leftover(),
        })
    }

    pub fn close(&mut self) -> RosWireResult<()> {
        self.close_inner()
    }

    fn close_inner(&mut self) -> RosWireResult<()> {
        if self.closed {
            return if self.leftover() {
                Err(Box::new(RosWireError::jump_teardown_failed(
                    "SSH jump channel was not fully torn down",
                )))
            } else {
                Ok(())
            };
        }

        let mut failed = false;
        if let Some(mut channel) = self.channel.take() {
            if channel.close().is_err() || channel.wait_close().is_err() {
                failed = true;
            }
        }
        for pump in self.pumps.drain(..) {
            if pump.join().is_err() {
                failed = true;
            }
        }
        for session in self.sessions.iter().rev() {
            if session
                .disconnect(None, "roswire jump teardown", None)
                .is_err()
            {
                failed = true;
            }
        }
        self.sessions.clear();
        self.closed = true;
        self.leftover.store(failed, Ordering::SeqCst);
        if failed {
            Err(Box::new(RosWireError::jump_teardown_failed(
                "SSH jump channel teardown failed",
            )))
        } else {
            Ok(())
        }
    }
}

impl Read for JumpIo {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.channel.as_mut() {
            Some(channel) => channel.read(buf),
            None => Ok(0),
        }
    }
}

impl Write for JumpIo {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.channel.as_mut() {
            Some(channel) => channel.write(buf),
            None => Ok(0),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self.channel.as_mut() {
            Some(channel) => channel.flush(),
            None => Ok(()),
        }
    }
}

impl Drop for JumpIo {
    fn drop(&mut self) {
        if !self.closed {
            let _ = self.close_inner();
        }
    }
}

pub fn open_channel(
    hops: &[ResolvedJumpHop],
    target_host: &str,
    target_port: u16,
    timeout: Duration,
    context: &ErrorContext,
) -> RosWireResult<JumpIo> {
    if hops.is_empty() {
        return Err(Box::new(RosWireError::config(
            "SSH jump requested without any configured hop",
        )));
    }

    let identities = identities_from_hops(hops);
    let leftover = Arc::new(AtomicBool::new(false));
    let mut sessions = Vec::with_capacity(hops.len());
    let mut pumps = Vec::new();

    let tcp = connect_tcp(&hops[0].host, hops[0].port, timeout, context)?;
    let mut current = handshake_and_auth(tcp, &hops[0], timeout, context)?;

    for hop in hops.iter().skip(1) {
        let channel = current
            .channel_direct_tcpip(
                &hop.host,
                hop.port,
                Some((ORIGINATOR_HOST, ORIGINATOR_PORT)),
            )
            .map_err(|error| {
                Box::new(
                    RosWireError::network(format!(
                        "SSH jump direct-tcpip to {}:{} failed: {error}",
                        hop.host, hop.port
                    ))
                    .with_context(context.clone()),
                )
            })?;
        sessions.push(current);
        let (local, remote) = stream_pair().map_err(|error| {
            Box::new(
                RosWireError::network(format!("failed to create jump stream pair: {error}"))
                    .with_context(context.clone()),
            )
        })?;
        pumps.push(thread::spawn(move || pump_channel(channel, remote)));
        current = handshake_and_auth(local, hop, timeout, context)?;
    }

    let channel = current
        .channel_direct_tcpip(
            target_host,
            target_port,
            Some((ORIGINATOR_HOST, ORIGINATOR_PORT)),
        )
        .map_err(|error| {
            Box::new(
                RosWireError::network(format!(
                    "SSH jump direct-tcpip to {target_host}:{target_port} failed: {error}"
                ))
                .with_context(context.clone()),
            )
        })?;
    sessions.push(current);

    Ok(JumpIo {
        channel: Some(channel),
        sessions,
        pumps,
        leftover,
        closed: false,
        hops: identities,
        target_host: target_host.to_owned(),
        target_port,
    })
}

fn connect_tcp(
    host: &str,
    port: u16,
    timeout: Duration,
    context: &ErrorContext,
) -> RosWireResult<TcpStream> {
    let mut addresses = (host, port).to_socket_addrs().map_err(|error| {
        Box::new(
            RosWireError::network(format!("failed to resolve jump host {host}: {error}"))
                .with_context(context.clone()),
        )
    })?;
    let address = addresses.next().ok_or_else(|| {
        Box::new(
            RosWireError::network(format!("failed to resolve jump host {host}"))
                .with_context(context.clone()),
        )
    })?;
    let stream = TcpStream::connect_timeout(&address, timeout).map_err(|error| {
        Box::new(
            RosWireError::network(format!(
                "failed to connect to jump host {host}:{port}: {error}"
            ))
            .with_context(context.clone()),
        )
    })?;
    stream.set_read_timeout(Some(timeout)).ok();
    stream.set_write_timeout(Some(timeout)).ok();
    Ok(stream)
}

#[cfg(unix)]
fn handshake_and_auth<S: AsRawFd + Send + 'static>(
    stream: S,
    hop: &ResolvedJumpHop,
    timeout: Duration,
    context: &ErrorContext,
) -> RosWireResult<ssh2::Session> {
    handshake_and_auth_inner(stream, hop, timeout, context)
}

#[cfg(windows)]
fn handshake_and_auth<S: AsRawSocket + Send + 'static>(
    stream: S,
    hop: &ResolvedJumpHop,
    timeout: Duration,
    context: &ErrorContext,
) -> RosWireResult<ssh2::Session> {
    handshake_and_auth_inner(stream, hop, timeout, context)
}

fn handshake_and_auth_inner<S: Send + 'static>(
    stream: S,
    hop: &ResolvedJumpHop,
    timeout: Duration,
    context: &ErrorContext,
) -> RosWireResult<ssh2::Session>
where
    ssh2::Session: HandshakeStream<S>,
{
    let mut session = ssh2::Session::new().map_err(|error| {
        Box::new(
            RosWireError::network(format!("failed to create jump SSH session: {error}"))
                .with_context(context.clone()),
        )
    })?;
    session.set_timeout(u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX));
    session.set_stream(stream);
    finish_handshake_and_auth(&mut session, hop, context)?;
    Ok(session)
}

trait HandshakeStream<S> {
    fn set_stream(&mut self, stream: S);
}

#[cfg(unix)]
impl<S: AsRawFd + 'static> HandshakeStream<S> for ssh2::Session {
    fn set_stream(&mut self, stream: S) {
        self.set_tcp_stream(stream);
    }
}

#[cfg(windows)]
impl<S: AsRawSocket + 'static> HandshakeStream<S> for ssh2::Session {
    fn set_stream(&mut self, stream: S) {
        self.set_tcp_stream(stream);
    }
}

fn finish_handshake_and_auth(
    session: &mut ssh2::Session,
    hop: &ResolvedJumpHop,
    context: &ErrorContext,
) -> RosWireResult<()> {
    session.handshake().map_err(|error| {
        Box::new(
            RosWireError::network(format!(
                "SSH jump handshake failed for {}: {error}",
                hop.host
            ))
            .with_context(context.clone()),
        )
    })?;
    verify_host_key(session, &hop.expected_host_key, context)?;

    if let Some(key_path) = &hop.key_path {
        session
            .userauth_pubkey_file(
                &hop.user,
                None,
                Path::new(key_path),
                hop.key_passphrase.as_deref(),
            )
            .map_err(|error| {
                Box::new(
                    RosWireError::auth_failed(format!(
                        "SSH jump key authentication failed: {error}"
                    ))
                    .with_context(context.clone()),
                )
            })?;
    } else {
        let password = hop.password.as_deref().ok_or_else(|| {
            Box::new(RosWireError::config("missing jump password").with_context(context.clone()))
        })?;
        session
            .userauth_password(&hop.user, password)
            .map_err(|error| {
                Box::new(
                    RosWireError::auth_failed(format!(
                        "SSH jump password authentication failed: {error}"
                    ))
                    .with_context(context.clone()),
                )
            })?;
    }

    if !session.authenticated() {
        return Err(Box::new(
            RosWireError::auth_failed("SSH jump authentication failed")
                .with_context(context.clone()),
        ));
    }

    Ok(())
}

#[cfg(unix)]
type PairStream = UnixStream;
#[cfg(windows)]
type PairStream = TcpStream;

fn stream_pair() -> std::io::Result<(PairStream, PairStream)> {
    #[cfg(unix)]
    {
        UnixStream::pair()
    }
    #[cfg(windows)]
    {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        let addr = listener.local_addr()?;
        let client = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
        let (server, _) = listener.accept()?;
        drop(listener);
        Ok((client, server))
    }
}

fn pump_channel(mut channel: ssh2::Channel, mut sock: PairStream) {
    let _ = sock.set_nonblocking(true);
    let mut buf = [0_u8; 8192];
    loop {
        let mut progressed = false;
        match channel.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if sock.write_all(&buf[..n]).is_err() {
                    break;
                }
                progressed = true;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => break,
        }
        match sock.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if channel.write_all(&buf[..n]).is_err() {
                    break;
                }
                progressed = true;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => break,
        }
        if !progressed {
            thread::sleep(Duration::from_millis(5));
        }
    }
}

impl JumpIo {
    pub fn into_target_ssh_session(
        mut self,
        user: &str,
        password: Option<&str>,
        key_path: Option<&str>,
        key_passphrase: Option<&str>,
        expected_host_key: &str,
        context: &ErrorContext,
    ) -> RosWireResult<JumpSshSession> {
        let channel = self
            .channel
            .take()
            .ok_or_else(|| Box::new(RosWireError::network("SSH jump channel is already closed")))?;
        let leftover = self.leftover.clone();
        let mut sessions = std::mem::take(&mut self.sessions);
        let mut pumps = std::mem::take(&mut self.pumps);
        self.closed = true;
        let (local, remote) = stream_pair().map_err(|error| {
            Box::new(RosWireError::network(format!(
                "failed to create jump stream pair: {error}"
            )))
        })?;
        pumps.push(thread::spawn(move || pump_channel(channel, remote)));
        let hop = ResolvedJumpHop {
            host: self.target_host.clone(),
            port: self.target_port,
            user: user.to_owned(),
            password: password.map(str::to_owned),
            key_path: key_path.map(str::to_owned),
            key_passphrase: key_passphrase.map(str::to_owned),
            expected_host_key: expected_host_key.to_owned(),
        };
        let session = handshake_and_auth(local, &hop, Duration::from_secs(10), context)?;
        sessions.push(session);
        let session = sessions.pop().expect("target session");
        Ok(JumpSshSession {
            session,
            leftover,
            _pumps: pumps,
            _sessions: sessions,
        })
    }
}

fn verify_host_key(
    session: &ssh2::Session,
    expected: &str,
    context: &ErrorContext,
) -> RosWireResult<()> {
    use base64::Engine as _;
    let actual = session
        .host_key_hash(ssh2::HashType::Sha256)
        .map(|bytes| {
            format!(
                "SHA256:{}",
                base64::engine::general_purpose::STANDARD_NO_PAD.encode(bytes)
            )
        })
        .ok_or_else(|| {
            Box::new(
                RosWireError::jump_host_key_mismatch(
                    "SSH jump host key fingerprint is unavailable",
                )
                .with_context(context.clone()),
            )
        })?;
    if expected.trim() != actual {
        return Err(Box::new(
            RosWireError::jump_host_key_mismatch(
                "SSH jump host key fingerprint does not match expected value",
            )
            .with_context(context.clone()),
        ));
    }
    Ok(())
}

pub fn finish_with_leftover<T>(
    result: RosWireResult<T>,
    leftover: &AtomicBool,
) -> RosWireResult<T> {
    match (result, leftover.load(Ordering::SeqCst)) {
        (Ok(value), false) => Ok(value),
        (Ok(_), true) => Err(Box::new(RosWireError::jump_teardown_failed(
            "SSH jump channel leftover after command completed",
        ))),
        (Err(error), _) => Err(error),
    }
}

pub struct JumpTeardown {
    closed: bool,
    leftover: Arc<AtomicBool>,
    fail_close: bool,
}

impl JumpTeardown {
    pub fn new(fail_close: bool) -> Self {
        Self {
            closed: false,
            leftover: Arc::new(AtomicBool::new(false)),
            fail_close,
        }
    }

    pub fn leftover_handle(&self) -> Arc<AtomicBool> {
        self.leftover.clone()
    }

    pub fn leftover(&self) -> bool {
        self.leftover.load(Ordering::SeqCst)
    }

    pub fn close(&mut self) -> RosWireResult<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        if self.fail_close {
            self.leftover.store(true, Ordering::SeqCst);
            return Err(Box::new(RosWireError::jump_teardown_failed(
                "forced jump teardown failure",
            )));
        }
        self.leftover.store(false, Ordering::SeqCst);
        Ok(())
    }
}

impl Drop for JumpTeardown {
    fn drop(&mut self) {
        if !self.closed {
            let _ = self.close();
        }
    }
}

pub fn tcp_probe(host: &str, port: u16, timeout: Duration) -> Result<(), ErrorCode> {
    let mut addresses = (host, port)
        .to_socket_addrs()
        .map_err(|_| ErrorCode::NetworkError)?;
    let address = addresses.next().ok_or(ErrorCode::NetworkError)?;
    TcpStream::connect_timeout(&address, timeout)
        .map(|_| ())
        .map_err(|_| ErrorCode::NetworkError)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use clap::Parser;

    fn hop(host: &str, port: u16, user: &str) -> JumpHopIdentity {
        JumpHopIdentity {
            host: host.to_owned(),
            port,
            user: user.to_owned(),
        }
    }

    #[test]
    fn plan_dials_single_hop_uses_tcp_then_direct_tcpip() {
        let hops = vec![hop("bastion.example", 22, "ops")];
        let steps = plan_dials(&hops, "192.168.88.1", 8728);
        assert_eq!(
            steps,
            vec![
                DialStep {
                    kind: DialKind::TcpConnect,
                    dest_host: "bastion.example".to_owned(),
                    dest_port: 22,
                },
                DialStep {
                    kind: DialKind::DirectTcpIp,
                    dest_host: "192.168.88.1".to_owned(),
                    dest_port: 8728,
                },
            ]
        );
        assert!(steps.iter().all(|step| step.dest_host != "127.0.0.1"));
    }

    #[test]
    fn plan_dials_multi_hop_chains_direct_tcpip() {
        let hops = vec![
            hop("edge.example", 22, "ops"),
            hop("core.example", 2222, "net"),
        ];
        let steps = plan_dials(&hops, "10.0.0.1", 443);
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].kind, DialKind::TcpConnect);
        assert_eq!(steps[1].kind, DialKind::DirectTcpIp);
        assert_eq!(steps[1].dest_host, "core.example");
        assert_eq!(steps[1].dest_port, 2222);
        assert_eq!(steps[2].dest_host, "10.0.0.1");
        assert_eq!(steps[2].dest_port, 443);
        assert!(!steps.iter().any(|step| step.dest_host == "127.0.0.1"));
    }

    #[test]
    fn plan_dials_without_hops_is_empty() {
        assert!(plan_dials(&[], "192.168.88.1", 8728).is_empty());
    }

    #[test]
    fn via_plan_omits_empty_hops_and_never_binds_locally() {
        assert!(JumpViaPlan::from_hops(Vec::new(), None, None).is_none());
        let plan = JumpViaPlan::from_hops(
            vec![hop("bastion.example", 22, "ops")],
            Some("192.168.88.1".to_owned()),
            Some(8728),
        )
        .expect("via plan");
        assert_eq!(plan.kind, "jump");
        assert!(!plan.local_bind);
        assert_eq!(plan.teardown, "process-exit");
    }

    #[test]
    fn teardown_drop_clears_leftover_on_success() {
        let flag;
        {
            let guard = JumpTeardown::new(false);
            flag = guard.leftover_handle();
        }
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[test]
    fn teardown_failure_sets_leftover_and_error_code() {
        let mut guard = JumpTeardown::new(true);
        let error = guard.close().expect_err("teardown should fail");
        assert_eq!(error.error_code, ErrorCode::JumpTeardownFailed);
        assert!(guard.leftover());
        let finished = finish_with_leftover(Ok("payload"), &guard.leftover_handle())
            .expect_err("success plus leftover must not look successful");
        assert_eq!(finished.error_code, ErrorCode::JumpTeardownFailed);
    }

    #[test]
    fn finish_with_leftover_keeps_ok_and_original_errors() {
        let clean = AtomicBool::new(false);
        assert_eq!(finish_with_leftover(Ok(7), &clean).expect("ok"), 7);
        let err = finish_with_leftover::<()>(
            Err(Box::new(RosWireError::network("unreachable"))),
            &AtomicBool::new(true),
        )
        .expect_err("original error wins");
        assert_eq!(err.error_code, ErrorCode::NetworkError);
    }

    #[test]
    fn resolve_identities_from_cli_and_profile() {
        let cli = Cli::try_parse_from([
            "roswire",
            "--jump-host",
            "bastion.example",
            "--jump-user",
            "ops",
            "--jump-port",
            "2222",
            "doctor",
        ])
        .expect("cli");
        let hops = resolve_jump_identities(&cli, None).expect("identities");
        assert_eq!(hops, vec![hop("bastion.example", 2222, "ops")]);

        let missing_user =
            Cli::try_parse_from(["roswire", "--jump-host", "bastion.example", "doctor"])
                .expect("cli");
        let error = resolve_jump_identities(&missing_user, None).expect_err("user required");
        assert_eq!(error.error_code, ErrorCode::ConfigError);

        let mac = Cli::try_parse_from([
            "roswire",
            "--jump-host",
            "48:8F:5A:A3:0E:A7",
            "--jump-user",
            "ops",
            "doctor",
        ])
        .expect("cli");
        assert!(resolve_jump_identities(&mac, None).is_err());

        let profile = ProfileConfig {
            jump: vec![
                crate::config::JumpHopConfig {
                    host: Some("edge.example".to_owned()),
                    user: Some("ops".to_owned()),
                    host_key: Some("SHA256:edge".to_owned()),
                    ..crate::config::JumpHopConfig::default()
                },
                crate::config::JumpHopConfig {
                    host: None,
                    user: Some("skip".to_owned()),
                    ..crate::config::JumpHopConfig::default()
                },
            ],
            ..ProfileConfig::default()
        };
        let bare = Cli::try_parse_from(["roswire", "doctor"]).expect("cli");
        let hops = resolve_jump_identities(&bare, Some(&profile)).expect("profile hops");
        assert_eq!(hops, vec![hop("edge.example", 22, "ops")]);
        assert!(resolve_jump_identities(&bare, None)
            .expect("no jump")
            .is_empty());
    }

    #[test]
    fn resolve_hops_requires_host_key_and_reads_secrets() {
        let missing_key = Cli::try_parse_from([
            "roswire",
            "--jump-host",
            "bastion.example",
            "--jump-user",
            "ops",
            "--jump-password",
            "secret",
            "doctor",
        ])
        .expect("cli");
        let error =
            resolve_jump_hops(&missing_key, &BTreeMap::new(), None).expect_err("host key required");
        assert_eq!(error.error_code, ErrorCode::JumpHostKeyRequired);

        let cli = Cli::try_parse_from([
            "roswire",
            "--jump-host",
            "bastion.example",
            "--jump-user",
            "ops",
            "--jump-host-key",
            "SHA256:bastion",
            "--jump-password",
            "secret",
            "doctor",
        ])
        .expect("cli");
        let hops = resolve_jump_hops(&cli, &BTreeMap::new(), None).expect("cli hop");
        assert_eq!(hops[0].host, "bastion.example");
        assert_eq!(hops[0].password.as_deref(), Some("secret"));
        assert!(format!("{:?}", hops[0]).contains("***REDACTED***"));
        assert!(!format!("{:?}", hops[0]).contains("secret"));

        let key_cli = Cli::try_parse_from([
            "roswire",
            "--jump-host",
            "bastion.example",
            "--jump-user",
            "ops",
            "--jump-host-key",
            "SHA256:bastion",
            "--jump-key",
            "/tmp/id_ed25519",
            "doctor",
        ])
        .expect("cli");
        let key_hops = resolve_jump_hops(&key_cli, &BTreeMap::new(), None).expect("key cli hop");
        assert_eq!(key_hops[0].key_path.as_deref(), Some("/tmp/id_ed25519"));
        assert!(key_hops[0].password.is_none());

        let profile = ProfileConfig {
            allow_plain_secrets: true,
            jump: vec![crate::config::JumpHopConfig {
                host: Some("edge.example".to_owned()),
                port: Some(22),
                user: Some("ops".to_owned()),
                host_key: Some("SHA256:edge".to_owned()),
                ..crate::config::JumpHopConfig::default()
            }],
            secrets: BTreeMap::from([(
                "jump_password".to_owned(),
                crate::config::SecretSpec::Plain {
                    value: "profile-secret".to_owned(),
                },
            )]),
            ..ProfileConfig::default()
        };
        let bare = Cli::try_parse_from(["roswire", "doctor"]).expect("cli");
        let hops = resolve_jump_hops(&bare, &BTreeMap::new(), Some(&profile)).expect("profile hop");
        assert_eq!(hops[0].user, "ops");
        assert_eq!(hops[0].password.as_deref(), Some("profile-secret"));
        assert_eq!(identities_from_hops(&hops)[0].host, "edge.example");

        assert!(resolve_jump_hops(&bare, &BTreeMap::new(), None)
            .expect("empty")
            .is_empty());

        let missing_profile_key = ProfileConfig {
            jump: vec![crate::config::JumpHopConfig {
                host: Some("edge.example".to_owned()),
                user: Some("ops".to_owned()),
                ..crate::config::JumpHopConfig::default()
            }],
            ..ProfileConfig::default()
        };
        let error = resolve_jump_hops(&bare, &BTreeMap::new(), Some(&missing_profile_key))
            .expect_err("profile host key");
        assert_eq!(error.error_code, ErrorCode::JumpHostKeyRequired);

        let missing_user = ProfileConfig {
            jump: vec![crate::config::JumpHopConfig {
                host: Some("edge.example".to_owned()),
                host_key: Some("SHA256:edge".to_owned()),
                ..crate::config::JumpHopConfig::default()
            }],
            ..ProfileConfig::default()
        };
        assert!(resolve_jump_hops(&bare, &BTreeMap::new(), Some(&missing_user)).is_err());
        assert!(resolve_jump_identities(&bare, Some(&missing_user)).is_err());

        let missing_password = ProfileConfig {
            jump: vec![crate::config::JumpHopConfig {
                host: Some("edge.example".to_owned()),
                user: Some("ops".to_owned()),
                host_key: Some("SHA256:edge".to_owned()),
                ..crate::config::JumpHopConfig::default()
            }],
            ..ProfileConfig::default()
        };
        assert!(resolve_jump_hops(&bare, &BTreeMap::new(), Some(&missing_password)).is_err());
    }

    #[test]
    fn resolve_profile_hop_indexed_secret_and_key_auth() {
        let profile = ProfileConfig {
            allow_plain_secrets: true,
            jump: vec![crate::config::JumpHopConfig {
                host: Some("edge.example".to_owned()),
                user: Some("ops".to_owned()),
                key: Some("/tmp/id_ed25519".to_owned()),
                host_key: Some("SHA256:edge".to_owned()),
                ..crate::config::JumpHopConfig::default()
            }],
            secrets: BTreeMap::from([(
                "jump_0_key_passphrase".to_owned(),
                crate::config::SecretSpec::Plain {
                    value: "phrase".to_owned(),
                },
            )]),
            ..ProfileConfig::default()
        };
        let bare = Cli::try_parse_from(["roswire", "doctor"]).expect("cli");
        let hops = resolve_jump_hops(&bare, &BTreeMap::new(), Some(&profile)).expect("key hop");
        assert_eq!(hops[0].key_path.as_deref(), Some("/tmp/id_ed25519"));
        assert_eq!(hops[0].key_passphrase.as_deref(), Some("phrase"));
        assert!(hops[0].password.is_none());
    }

    #[test]
    fn tcp_probe_and_stream_pair_and_open_channel_errors() {
        assert_eq!(
            tcp_probe("192.0.2.1", 1, Duration::from_millis(50)).unwrap_err(),
            ErrorCode::NetworkError,
        );
        let (mut left, mut right) = stream_pair().expect("pair");
        right.write_all(b"ping").expect("write");
        let mut buf = [0_u8; 4];
        left.read_exact(&mut buf).expect("read");
        assert_eq!(&buf, b"ping");

        let error = match open_channel(
            &[],
            "192.168.88.1",
            8728,
            Duration::from_millis(50),
            &ErrorContext::default(),
        ) {
            Err(error) => error,
            Ok(_) => panic!("empty hops should fail"),
        };
        assert_eq!(error.error_code, ErrorCode::ConfigError);

        let hop = ResolvedJumpHop {
            host: "192.0.2.1".to_owned(),
            port: 1,
            user: "ops".to_owned(),
            password: Some("secret".to_owned()),
            key_path: None,
            key_passphrase: None,
            expected_host_key: "SHA256:test".to_owned(),
        };
        let error = match open_channel(
            &[hop],
            "192.168.88.1",
            8728,
            Duration::from_millis(80),
            &ErrorContext::default(),
        ) {
            Err(error) => error,
            Ok(_) => panic!("unreachable bastion should fail"),
        };
        assert_eq!(error.error_code, ErrorCode::NetworkError);
    }

    #[test]
    fn jump_io_empty_channel_close_and_audit() {
        let mut io = JumpIo {
            channel: None,
            sessions: Vec::new(),
            pumps: Vec::new(),
            leftover: Arc::new(AtomicBool::new(false)),
            closed: false,
            hops: vec![hop("bastion.example", 22, "ops")],
            target_host: "192.168.88.1".to_owned(),
            target_port: 8728,
        };
        assert_eq!(io.hops()[0].host, "bastion.example");
        assert!(!io.leftover());
        let mut buf = [0_u8; 1];
        assert_eq!(io.read(&mut buf).expect("read"), 0);
        assert_eq!(io.write(b"x").expect("write"), 0);
        io.flush().expect("flush");
        let audit = io.audit_value();
        assert_eq!(audit["local_bind"], false);
        io.close().expect("close");
        io.close().expect("idempotent close");
        let error = match io.into_target_ssh_session(
            "admin",
            Some("secret"),
            None,
            None,
            "SHA256:test",
            &ErrorContext::default(),
        ) {
            Err(error) => error,
            Ok(_) => panic!("closed channel cannot start target ssh"),
        };
        assert_eq!(error.error_code, ErrorCode::NetworkError);
    }
}
