use crate::error::{RosWireError, RosWireResult};
use base64::{engine::general_purpose::STANDARD_NO_PAD as BASE64_NO_PAD, Engine as _};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, WebPkiSupportedAlgorithms};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, RootCertStore, StreamOwned};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

const TLS_FINGERPRINT_PREFIX: &str = "SHA256:";
const TLS_FINGERPRINT_LEN: usize = 32;

/// TLS trust policy for the RouterOS api-ssl and REST transports.
///
/// `WebPki` performs full chain + hostname verification against the bundled
/// web PKI roots (the default). `Pinned` accepts the connection iff the server
/// leaf certificate's SHA-256 fingerprint matches the configured value, giving
/// parity with the SSH host-key pin for default self-signed RouterOS devices.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TlsTrust {
    #[default]
    WebPki,
    Pinned(String),
}

impl TlsTrust {
    pub fn from_fingerprint(fingerprint: Option<String>) -> Self {
        match fingerprint {
            Some(value) if !value.trim().is_empty() => Self::Pinned(value),
            _ => Self::WebPki,
        }
    }
}

/// Render a SHA-256 fingerprint over a certificate DER body in the same
/// `SHA256:<base64-no-pad>` format used by the SSH host-key pin.
fn certificate_fingerprint(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    format!("{TLS_FINGERPRINT_PREFIX}{}", BASE64_NO_PAD.encode(digest))
}

/// Decode and validate a configured pin into its 32 raw SHA-256 bytes.
fn parse_pin(pin: &str) -> RosWireResult<[u8; TLS_FINGERPRINT_LEN]> {
    let encoded = pin
        .trim()
        .strip_prefix(TLS_FINGERPRINT_PREFIX)
        .ok_or_else(|| {
            Box::new(RosWireError::config(format!(
            "invalid TLS certificate fingerprint; expected `{TLS_FINGERPRINT_PREFIX}<base64>` form",
        )))
        })?;
    let decoded = BASE64_NO_PAD.decode(encoded.trim()).map_err(|error| {
        Box::new(RosWireError::config(format!(
            "invalid TLS certificate fingerprint base64: {error}",
        )))
    })?;
    decoded.try_into().map_err(|_| {
        Box::new(RosWireError::config(
            "invalid TLS certificate fingerprint; SHA-256 must decode to 32 bytes",
        ))
    })
}

#[derive(Debug)]
struct PinnedCertVerifier {
    expected: [u8; TLS_FINGERPRINT_LEN],
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinnedCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let actual = Sha256::digest(end_entity.as_ref());
        if actual.as_slice() == self.expected.as_slice() {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(format!(
                "RouterOS TLS certificate fingerprint mismatch; expected {}, server presented {}. \
                 Verify the actual fingerprint out-of-band before trusting it.",
                certificate_fingerprint_from_expected(&self.expected),
                certificate_fingerprint(end_entity.as_ref()),
            )))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

fn certificate_fingerprint_from_expected(expected: &[u8; TLS_FINGERPRINT_LEN]) -> String {
    format!("{TLS_FINGERPRINT_PREFIX}{}", BASE64_NO_PAD.encode(expected))
}

/// Build a rustls client configuration honoring the requested trust policy.
///
/// Shared by the classic api-ssl `TlsApiStream` and the ureq-based REST client
/// so both transports apply identical trust decisions.
pub fn build_tls_client_config(trust: &TlsTrust) -> RosWireResult<Arc<ClientConfig>> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|error| {
            Box::new(network_error(format!(
                "failed to initialize RouterOS TLS configuration: {error}",
            )))
        })?;

    let config = match trust {
        TlsTrust::WebPki => {
            let mut root_store = RootCertStore::empty();
            root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            builder
                .with_root_certificates(root_store)
                .with_no_client_auth()
        }
        TlsTrust::Pinned(pin) => {
            let expected = parse_pin(pin)?;
            let verifier = Arc::new(PinnedCertVerifier {
                expected,
                algorithms: provider.signature_verification_algorithms,
            });
            builder
                .dangerous()
                .with_custom_certificate_verifier(verifier)
                .with_no_client_auth()
        }
    };

    Ok(Arc::new(config))
}

pub trait ApiStream: Read + Write + Send {}

impl<T> ApiStream for T where T: Read + Write + Send {}

#[derive(Debug)]
pub struct TcpApiStream {
    inner: TcpStream,
}

impl TcpApiStream {
    pub fn connect(host: &str, port: u16, timeout: Duration) -> RosWireResult<Self> {
        Ok(Self {
            inner: connect_tcp_stream(host, port, timeout, "RouterOS API")?,
        })
    }
}

impl TlsStream<TcpStream> {
    pub fn connect(
        host: &str,
        port: u16,
        timeout: Duration,
        trust: &TlsTrust,
    ) -> RosWireResult<Self> {
        let stream = connect_tcp_stream(host, port, timeout, "RouterOS API TLS")?;
        wrap_tls_stream(stream, host, trust)
    }
}

pub fn wrap_tls_stream<S: Read + Write>(
    stream: S,
    host: &str,
    trust: &TlsTrust,
) -> RosWireResult<TlsStream<S>> {
    let server_name_host = tls_server_name_host(host);
    let server_name = ServerName::try_from(server_name_host.clone()).map_err(|error| {
        Box::new(network_error(format!(
            "invalid RouterOS API TLS server name `{server_name_host}`: {error}",
        )))
    })?;
    let config = build_tls_client_config(trust)?;
    let connection = ClientConnection::new(config, server_name).map_err(|error| {
        Box::new(network_error(format!(
            "failed to initialize RouterOS API TLS connection: {error}",
        )))
    })?;
    let mut inner = StreamOwned::new(connection, stream);
    while inner.conn.is_handshaking() {
        inner.conn.complete_io(&mut inner.sock).map_err(|error| {
            Box::new(network_error(format!(
                "RouterOS API TLS handshake failed for `{host}`: {error}",
            )))
        })?;
    }

    Ok(TlsStream { inner })
}

pub struct TlsStream<S: Read + Write> {
    inner: StreamOwned<ClientConnection, S>,
}

pub type TlsApiStream = TlsStream<TcpStream>;

impl<S: Read + Write> TlsStream<S> {
    pub fn into_socket(self) -> S {
        self.inner.sock
    }
}

fn tls_server_name_host(host: &str) -> String {
    let unbracketed = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);

    if unbracketed.contains(':') {
        unbracketed
            .split_once('%')
            .map(|(address, _)| address)
            .unwrap_or(unbracketed)
            .to_owned()
    } else {
        unbracketed.to_owned()
    }
}

impl Read for TcpApiStream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buffer)
    }
}

impl Write for TcpApiStream {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl<S: Read + Write> Read for TlsStream<S> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buffer)
    }
}

impl<S: Read + Write> Write for TlsStream<S> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn connect_tcp_stream(
    host: &str,
    port: u16,
    timeout: Duration,
    service_label: &str,
) -> RosWireResult<TcpStream> {
    let mut addresses = (host, port).to_socket_addrs().map_err(|error| {
        Box::new(network_error(format!(
            "failed to resolve {service_label} address: {error}",
        )))
    })?;

    let address = addresses.next().ok_or_else(|| {
        Box::new(network_error(format!(
            "failed to resolve {service_label} address: no socket addresses returned",
        )))
    })?;

    let stream = TcpStream::connect_timeout(&address, timeout).map_err(|error| {
        Box::new(network_error(format!(
            "failed to connect to {service_label} at {host}:{port}: {error}",
        )))
    })?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| Box::new(map_io_error("set API read timeout", error)))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| Box::new(map_io_error("set API write timeout", error)))?;

    Ok(stream)
}

pub fn map_io_error(operation: &str, error: std::io::Error) -> RosWireError {
    network_error(format!(
        "RouterOS API transport I/O error while attempting to {operation}: {error}",
    ))
}

fn network_error(message: impl Into<String>) -> RosWireError {
    RosWireError::network(message)
}

#[cfg(test)]
mod tests {
    use super::{
        build_tls_client_config, certificate_fingerprint, map_io_error, parse_pin,
        tls_server_name_host, ApiStream, PinnedCertVerifier, TlsApiStream, TlsTrust,
        TLS_FINGERPRINT_LEN,
    };
    use crate::error::ErrorCode;
    use base64::Engine as _;
    use rustls::client::danger::ServerCertVerifier;
    use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
    use sha2::{Digest, Sha256};
    use std::io::{Cursor, Read, Result, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    struct FakeApiStream {
        rx: Cursor<Vec<u8>>,
        tx: Vec<u8>,
    }

    impl FakeApiStream {
        fn new(rx: Vec<u8>) -> Self {
            Self {
                rx: Cursor::new(rx),
                tx: Vec::new(),
            }
        }
    }

    impl Read for FakeApiStream {
        fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
            self.rx.read(buffer)
        }
    }

    impl Write for FakeApiStream {
        fn write(&mut self, buffer: &[u8]) -> Result<usize> {
            self.tx.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> Result<()> {
            Ok(())
        }
    }

    fn assert_api_stream<T: ApiStream>(_stream: &T) {}

    #[test]
    fn fake_stream_can_satisfy_transport_boundary() {
        let stream = FakeApiStream::new(Vec::new());
        assert_api_stream(&stream);
    }

    #[test]
    fn io_errors_map_to_network_error() {
        let error = map_io_error(
            "read sentence",
            std::io::Error::from(std::io::ErrorKind::TimedOut),
        );

        assert_eq!(error.error_code, ErrorCode::NetworkError);
        assert!(error.message.contains("read sentence"));
    }

    #[test]
    fn tls_server_name_host_strips_ipv6_zone_identifier() {
        assert_eq!(tls_server_name_host("fe80::1%en0"), "fe80::1");
        assert_eq!(tls_server_name_host("[fe80::1%en0]"), "fe80::1");
        assert_eq!(tls_server_name_host("router.example"), "router.example");
    }

    #[test]
    fn tls_handshake_failure_maps_to_network_error() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let port = listener
            .local_addr()
            .expect("local addr should exist")
            .port();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("connection should arrive");
            stream
                .write_all(b"not a tls server")
                .expect("fixture response should write");
        });

        let error = match TlsApiStream::connect(
            "127.0.0.1",
            port,
            Duration::from_millis(500),
            &TlsTrust::WebPki,
        ) {
            Ok(_) => panic!("plain TCP server should fail TLS handshake"),
            Err(error) => error,
        };
        handle.join().expect("server thread should finish");

        assert_eq!(error.error_code, ErrorCode::NetworkError);
        assert!(error.message.contains("TLS handshake failed"));
    }

    #[test]
    fn tls_trust_from_fingerprint_maps_presence() {
        assert_eq!(TlsTrust::from_fingerprint(None), TlsTrust::WebPki);
        assert_eq!(
            TlsTrust::from_fingerprint(Some("  ".to_owned())),
            TlsTrust::WebPki
        );
        assert_eq!(
            TlsTrust::from_fingerprint(Some("SHA256:abc".to_owned())),
            TlsTrust::Pinned("SHA256:abc".to_owned()),
        );
    }

    #[test]
    fn parse_pin_accepts_valid_fingerprint() {
        let raw = [7_u8; TLS_FINGERPRINT_LEN];
        let fingerprint = certificate_fingerprint(b"some-cert-der");
        // certificate_fingerprint over known bytes round-trips through parse_pin.
        let parsed = parse_pin(&fingerprint).expect("valid fingerprint should parse");
        assert_eq!(
            parsed.as_slice(),
            Sha256::digest(b"some-cert-der").as_slice()
        );

        let manual = format!(
            "SHA256:{}",
            base64::engine::general_purpose::STANDARD_NO_PAD.encode(raw)
        );
        assert_eq!(parse_pin(&manual).expect("32-byte pin parses"), raw);
    }

    #[test]
    fn parse_pin_rejects_malformed_fingerprints() {
        assert!(parse_pin("deadbeef").is_err(), "missing SHA256: prefix");
        assert!(parse_pin("SHA256:!!!!").is_err(), "invalid base64");
        let short = format!(
            "SHA256:{}",
            base64::engine::general_purpose::STANDARD_NO_PAD.encode([1_u8; 8])
        );
        assert!(parse_pin(&short).is_err(), "wrong decoded length");
    }

    #[test]
    fn build_tls_client_config_supports_both_policies() {
        build_tls_client_config(&TlsTrust::WebPki).expect("webpki config builds");
        let pin = certificate_fingerprint(b"router-leaf-cert");
        build_tls_client_config(&TlsTrust::Pinned(pin)).expect("pinned config builds");
        assert!(
            build_tls_client_config(&TlsTrust::Pinned("not-a-pin".to_owned())).is_err(),
            "invalid pin should fail config construction",
        );
    }

    #[test]
    fn pinned_verifier_accepts_matching_leaf_and_rejects_others() {
        let leaf = b"router-leaf-cert-der-bytes";
        let expected: [u8; TLS_FINGERPRINT_LEN] = Sha256::digest(leaf).into();
        let provider = rustls::crypto::ring::default_provider();
        let verifier = PinnedCertVerifier {
            expected,
            algorithms: provider.signature_verification_algorithms,
        };
        let server_name = ServerName::try_from("router.example").expect("server name");
        let now = UnixTime::now();

        let matching = CertificateDer::from(leaf.to_vec());
        assert!(
            verifier
                .verify_server_cert(&matching, &[], &server_name, &[], now)
                .is_ok(),
            "matching leaf fingerprint should be accepted",
        );

        let other = CertificateDer::from(b"a-different-cert".to_vec());
        let error = verifier
            .verify_server_cert(&other, &[], &server_name, &[], now)
            .expect_err("non-matching leaf must be rejected");
        let message = error.to_string();
        assert!(message.contains("fingerprint mismatch"), "{message}");
        assert!(
            message.contains(&certificate_fingerprint(b"a-different-cert")),
            "mismatch error should surface the actual fingerprint: {message}",
        );
    }
}
