use crate::error::{RosWireError, RosWireResult};
use crate::mapping::{ProtocolRequest, RestMethod};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct RestClient {
    base_url: String,
    user: String,
    password: String,
    agent: ureq::Agent,
}

impl RestClient {
    pub fn https(host: &str, port: u16, user: &str, password: &str) -> Self {
        Self::with_base_url(format!("https://{host}:{port}"), user, password)
    }

    pub fn with_base_url(base_url: impl Into<String>, user: &str, password: &str) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(10))
            .build();
        Self {
            base_url: trim_trailing_slash(base_url.into()),
            user: user.to_owned(),
            password: password.to_owned(),
            agent,
        }
    }

    pub fn execute_request(&self, request: &ProtocolRequest) -> RosWireResult<Value> {
        let rest_mapping = request.mapping.rest_mapping.as_ref().ok_or_else(|| {
            Box::new(RosWireError::unsupported_action(format!(
                "REST mapping unavailable for {}",
                request.mapping.routeros_path,
            )))
        })?;

        let path = expand_rest_path(&rest_mapping.path, &request.resolved_args)?;
        self.send(
            rest_mapping.method,
            &path,
            request_body(rest_mapping.method, request),
        )
    }

    pub fn get(&self, path: &str) -> RosWireResult<Value> {
        self.send(RestMethod::Get, path, None)
    }

    pub fn post_json(&self, path: &str, body: Value) -> RosWireResult<Value> {
        self.send(RestMethod::Post, path, Some(body))
    }

    pub fn patch_json(&self, path: &str, body: Value) -> RosWireResult<Value> {
        self.send(RestMethod::Patch, path, Some(body))
    }

    pub fn system_resource(&self) -> RosWireResult<Value> {
        self.get("/rest/system/resource")
    }

    fn send(&self, method: RestMethod, path: &str, body: Option<Value>) -> RosWireResult<Value> {
        let url = build_url(&self.base_url, path);
        let request = self
            .agent
            .request(method.as_str(), &url)
            .set("Accept", "application/json")
            .set(
                "Authorization",
                &basic_auth_header(&self.user, &self.password),
            );

        let result = match body {
            Some(body) => {
                let payload = serde_json::to_string(&body).map_err(|error| {
                    Box::new(RosWireError::internal(format!(
                        "failed to serialize RouterOS REST request body: {error}",
                    )))
                })?;
                request
                    .set("Content-Type", "application/json")
                    .send_string(&payload)
            }
            None => request.call(),
        };

        match result {
            Ok(response) => parse_response(method, response),
            Err(ureq::Error::Status(status, response)) => Err(Box::new(map_status_error(
                status,
                response.into_string().unwrap_or_default(),
            ))),
            Err(ureq::Error::Transport(error)) => Err(Box::new(RosWireError::network(format!(
                "RouterOS REST transport error: {error}",
            )))),
        }
    }
}

fn parse_response(method: RestMethod, response: ureq::Response) -> RosWireResult<Value> {
    let body = response.into_string().map_err(|error| {
        Box::new(RosWireError::network(format!(
            "failed to read RouterOS REST response: {error}",
        )))
    })?;
    if method != RestMethod::Get && body.trim().is_empty() {
        return Ok(serde_json::json!({ "status": "ok" }));
    }
    serde_json::from_str(&body).map_err(|error| {
        Box::new(RosWireError::ros_api_failure(format!(
            "RouterOS REST response is not valid JSON: {error}",
        )))
    })
}

pub fn map_status_error(status: u16, body: String) -> RosWireError {
    if status == 401 {
        return RosWireError::auth_failed("RouterOS REST authentication failed");
    }

    let message = routeros_error_message(&body)
        .unwrap_or_else(|| format!("RouterOS REST returned HTTP status {status}"));
    RosWireError::ros_api_failure(message)
}

pub fn routeros_error_message(body: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(body).ok()?;
    value
        .get("detail")
        .or_else(|| value.get("message"))
        .or_else(|| value.get("error"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

pub fn build_url(base_url: &str, path: &str) -> String {
    format!(
        "{}/{}",
        trim_trailing_slash(base_url),
        path.trim_start_matches('/')
    )
}

fn trim_trailing_slash(value: impl AsRef<str>) -> String {
    value.as_ref().trim_end_matches('/').to_owned()
}

fn basic_auth_header(user: &str, password: &str) -> String {
    let credentials = format!("{user}:{password}");
    format!("Basic {}", BASE64_STANDARD.encode(credentials))
}

fn request_body(method: RestMethod, request: &ProtocolRequest) -> Option<Value> {
    match method {
        RestMethod::Get | RestMethod::Delete => None,
        RestMethod::Post | RestMethod::Put | RestMethod::Patch => Some(
            request
                .resolved_args
                .iter()
                .filter(|(key, _)| method != RestMethod::Patch || key.as_str() != ".id")
                .map(|(key, value)| (key.clone(), Value::String(value.clone())))
                .collect::<serde_json::Map<_, _>>()
                .into(),
        ),
    }
}

fn expand_rest_path(
    path: &str,
    args: &std::collections::BTreeMap<String, String>,
) -> RosWireResult<String> {
    if path.contains("{.id}") {
        let id = args.get(".id").ok_or_else(|| {
            Box::new(RosWireError::usage(
                "REST item path requires .id=<id> argument",
            ))
        })?;
        Ok(path.replace("{.id}", id))
    } else {
        Ok(path.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        basic_auth_header, build_url, map_status_error, request_body, routeros_error_message,
        RestClient,
    };
    use crate::args::ParsedInvocation;
    use crate::error::ErrorCode;
    use crate::mapping::{build_protocol_request, RestMethod};
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn builds_stable_urls() {
        assert_eq!(
            build_url("http://127.0.0.1:8080/", "/rest/ip/address"),
            "http://127.0.0.1:8080/rest/ip/address",
        );
    }

    #[test]
    fn builds_basic_auth_header_without_plain_credentials() {
        let credential = test_credential();
        let header = basic_auth_header("admin", &credential);

        assert!(header.starts_with("Basic "));
        assert!(!header.contains(&credential));
    }

    #[test]
    fn maps_rest_error_statuses() {
        let auth = map_status_error(401, String::new());
        assert_eq!(auth.error_code, ErrorCode::AuthFailed);

        let trap = map_status_error(400, r#"{"detail":"invalid interface"}"#.to_owned());
        assert_eq!(trap.error_code, ErrorCode::RosApiFailure);
        assert_eq!(trap.message, "invalid interface");
        assert_eq!(
            routeros_error_message(r#"{"message":"no such item"}"#).as_deref(),
            Some("no such item"),
        );
    }

    #[test]
    fn rest_system_resource_probe_reads_json() {
        let server = TestServer::responding_with(
            200,
            "application/json",
            r#"{"version":"7.15.3","architecture-name":"arm64"}"#,
        );
        let credential = test_credential();
        let client = RestClient::with_base_url(server.base_url(), "admin", &credential);

        let value = client.system_resource().expect("resource should parse");
        let request = server.request();

        assert_eq!(value["version"], "7.15.3");
        assert!(request.contains("GET /rest/system/resource HTTP/1.1"));
        assert!(request.contains("Authorization: Basic"));
    }

    #[test]
    fn rest_ip_address_print_reads_json_array() {
        let server = TestServer::responding_with(
            200,
            "application/json",
            r#"[{".id":"*1","address":"192.0.2.1/24"}]"#,
        );
        let credential = test_credential();
        let client = RestClient::with_base_url(server.base_url(), "admin", &credential);
        let request = build_protocol_request(&ParsedInvocation {
            path: vec!["ip".to_owned(), "address".to_owned()],
            action: "print".to_owned(),
            resolved_args: BTreeMap::new(),
        })
        .expect("request should map");

        let value = client
            .execute_request(&request)
            .expect("REST print should work");
        let http_request = server.request();

        assert_eq!(value[0][".id"], "*1");
        assert!(http_request.contains("GET /rest/ip/address HTTP/1.1"));
    }

    #[test]
    fn rest_ip_route_print_reads_json_array() {
        let server = TestServer::responding_with(
            200,
            "application/json",
            r#"[{".id":"*1","dst-address":"0.0.0.0/0","gateway":"192.0.2.1"}]"#,
        );
        let credential = test_credential();
        let client = RestClient::with_base_url(server.base_url(), "admin", &credential);
        let request = build_protocol_request(&ParsedInvocation {
            path: vec!["ip".to_owned(), "route".to_owned()],
            action: "print".to_owned(),
            resolved_args: BTreeMap::new(),
        })
        .expect("request should map");

        let value = client
            .execute_request(&request)
            .expect("REST route print should work");
        let http_request = server.request();

        assert_eq!(value[0]["gateway"], "192.0.2.1");
        assert!(http_request.contains("GET /rest/ip/route HTTP/1.1"));
    }

    #[test]
    fn rest_firewall_address_list_print_reads_json_array() {
        let server = TestServer::responding_with(
            200,
            "application/json",
            r#"[{".id":"*1","list":"trusted","address":"192.0.2.10"}]"#,
        );
        let credential = test_credential();
        let client = RestClient::with_base_url(server.base_url(), "admin", &credential);
        let request = build_protocol_request(&ParsedInvocation {
            path: vec![
                "ip".to_owned(),
                "firewall".to_owned(),
                "address-list".to_owned(),
            ],
            action: "print".to_owned(),
            resolved_args: BTreeMap::new(),
        })
        .expect("request should map");

        let value = client
            .execute_request(&request)
            .expect("REST firewall address-list print should work");
        let http_request = server.request();

        assert_eq!(value[0]["list"], "trusted");
        assert!(http_request.contains("GET /rest/ip/firewall/address-list HTTP/1.1"));
    }

    #[test]
    fn rest_tool_netwatch_print_reads_json_array() {
        let server = TestServer::responding_with(
            200,
            "application/json",
            r#"[{".id":"*1","host":"192.0.2.1","status":"up"}]"#,
        );
        let credential = test_credential();
        let client = RestClient::with_base_url(server.base_url(), "admin", &credential);
        let request = build_protocol_request(&ParsedInvocation {
            path: vec!["tool".to_owned(), "netwatch".to_owned()],
            action: "print".to_owned(),
            resolved_args: BTreeMap::new(),
        })
        .expect("request should map");

        let value = client
            .execute_request(&request)
            .expect("REST tool netwatch print should work");
        let http_request = server.request();

        assert_eq!(value[0]["status"], "up");
        assert!(http_request.contains("GET /rest/tool/netwatch HTTP/1.1"));
    }

    #[test]
    fn rest_wireguard_print_reads_json_array() {
        let server = TestServer::responding_with(
            200,
            "application/json",
            r#"[{".id":"*1","name":"wg0","listen-port":"13231"}]"#,
        );
        let credential = test_credential();
        let client = RestClient::with_base_url(server.base_url(), "admin", &credential);
        let request = build_protocol_request(&ParsedInvocation {
            path: vec!["interface".to_owned(), "wireguard".to_owned()],
            action: "print".to_owned(),
            resolved_args: BTreeMap::new(),
        })
        .expect("request should map");

        let value = client
            .execute_request(&request)
            .expect("REST wireguard print should work");
        let http_request = server.request();

        assert_eq!(value[0]["name"], "wg0");
        assert!(http_request.contains("GET /rest/interface/wireguard HTTP/1.1"));
    }

    #[test]
    fn rest_wireguard_peers_print_reads_json_array() {
        let server = TestServer::responding_with(
            200,
            "application/json",
            r#"[{".id":"*1","interface":"wg0","public-key":"pub"}]"#,
        );
        let credential = test_credential();
        let client = RestClient::with_base_url(server.base_url(), "admin", &credential);
        let request = build_protocol_request(&ParsedInvocation {
            path: vec![
                "interface".to_owned(),
                "wireguard".to_owned(),
                "peers".to_owned(),
            ],
            action: "print".to_owned(),
            resolved_args: BTreeMap::new(),
        })
        .expect("request should map");

        let value = client
            .execute_request(&request)
            .expect("REST wireguard peers print should work");
        let http_request = server.request();

        assert_eq!(value[0]["interface"], "wg0");
        assert!(http_request.contains("GET /rest/interface/wireguard/peers HTTP/1.1"));
    }

    #[test]
    fn rest_system_package_print_reads_json_array() {
        let server = TestServer::responding_with(
            200,
            "application/json",
            r#"[{".id":"*1","name":"routeros","version":"7.15.3"}]"#,
        );
        let credential = test_credential();
        let client = RestClient::with_base_url(server.base_url(), "admin", &credential);
        let request = build_protocol_request(&ParsedInvocation {
            path: vec!["system".to_owned(), "package".to_owned()],
            action: "print".to_owned(),
            resolved_args: BTreeMap::new(),
        })
        .expect("request should map");

        let value = client
            .execute_request(&request)
            .expect("REST package print should work");
        let http_request = server.request();

        assert_eq!(value[0]["name"], "routeros");
        assert!(http_request.contains("GET /rest/system/package HTTP/1.1"));
    }

    #[test]
    fn rest_system_script_add_sends_put_body() {
        let server = TestServer::responding_with(204, "application/json", "");
        let credential = test_credential();
        let client = RestClient::with_base_url(server.base_url(), "admin", &credential);
        let request = build_protocol_request(&ParsedInvocation {
            path: vec!["system".to_owned(), "script".to_owned()],
            action: "add".to_owned(),
            resolved_args: BTreeMap::from([
                ("name".to_owned(), "bootstrap".to_owned()),
                ("source".to_owned(), ":put hello".to_owned()),
            ]),
        })
        .expect("request should map");

        let value = client
            .execute_request(&request)
            .expect("REST script add should work");
        let http_request = server.request();

        assert_eq!(value, serde_json::json!({ "status": "ok" }));
        assert!(http_request.contains("PUT /rest/system/script HTTP/1.1"));
        assert!(http_request.contains(r#""name":"bootstrap""#));
        assert!(http_request.contains(r#""source":":put hello""#));
    }

    #[test]
    fn rest_user_print_reads_json_array() {
        let server = TestServer::responding_with(
            200,
            "application/json",
            r#"[{".id":"*1","name":"admin","group":"full","disabled":"false"}]"#,
        );
        let credential = test_credential();
        let client = RestClient::with_base_url(server.base_url(), "admin", &credential);
        let request = build_protocol_request(&ParsedInvocation {
            path: vec!["user".to_owned()],
            action: "print".to_owned(),
            resolved_args: BTreeMap::new(),
        })
        .expect("request should map");

        let value = client
            .execute_request(&request)
            .expect("REST user print should work");
        let http_request = server.request();

        assert_eq!(value[0]["name"], "admin");
        assert!(http_request.contains("GET /rest/user HTTP/1.1"));
    }

    #[test]
    fn rest_patch_expands_id_and_sends_json_body() {
        let server = TestServer::responding_with(200, "application/json", r#"{}"#);
        let credential = test_credential();
        let client = RestClient::with_base_url(server.base_url(), "admin", &credential);
        let request = build_protocol_request(&ParsedInvocation {
            path: vec!["ip".to_owned(), "address".to_owned()],
            action: "set".to_owned(),
            resolved_args: BTreeMap::from([
                (".id".to_owned(), "*1".to_owned()),
                ("comment".to_owned(), "uplink".to_owned()),
            ]),
        })
        .expect("request should map");

        let value = client
            .execute_request(&request)
            .expect("REST patch should work");
        let http_request = server.request();

        assert_eq!(value, serde_json::json!({}));
        assert!(http_request.contains("PATCH /rest/ip/address/*1 HTTP/1.1"));
        assert!(http_request.contains("Content-Type: application/json"));
        assert!(http_request.contains(r#""comment":"uplink""#));
        assert!(!http_request.contains(r#"".id""#));
    }

    #[test]
    fn rest_patch_requires_id_argument_before_network() {
        let error = build_protocol_request(&ParsedInvocation {
            path: vec!["ip".to_owned(), "address".to_owned()],
            action: "set".to_owned(),
            resolved_args: BTreeMap::new(),
        })
        .expect_err("missing id should fail before HTTP");

        assert_eq!(error.error_code, ErrorCode::UsageError);
    }

    #[test]
    fn rest_put_and_delete_support_empty_success_bodies() {
        let put_server = TestServer::responding_with(204, "application/json", "");
        let credential = test_credential();
        let client = RestClient::with_base_url(put_server.base_url(), "admin", &credential);
        let add_request = build_protocol_request(&ParsedInvocation {
            path: vec!["ip".to_owned(), "address".to_owned()],
            action: "add".to_owned(),
            resolved_args: BTreeMap::from([
                ("address".to_owned(), "192.0.2.10/24".to_owned()),
                ("interface".to_owned(), "bridge".to_owned()),
            ]),
        })
        .expect("add should map");

        let value = client
            .execute_request(&add_request)
            .expect("empty PUT success should be accepted");
        let put_request = put_server.request();

        assert_eq!(value, serde_json::json!({ "status": "ok" }));
        assert!(put_request.contains("PUT /rest/ip/address HTTP/1.1"));
        assert!(put_request.contains(r#""address":"192.0.2.10/24""#));

        let delete_server = TestServer::responding_with(204, "application/json", "");
        let client = RestClient::with_base_url(delete_server.base_url(), "admin", &credential);
        let remove_request = build_protocol_request(&ParsedInvocation {
            path: vec!["ip".to_owned(), "address".to_owned()],
            action: "remove".to_owned(),
            resolved_args: BTreeMap::from([(".id".to_owned(), "*1".to_owned())]),
        })
        .expect("remove should map");

        let value = client
            .execute_request(&remove_request)
            .expect("empty DELETE success should be accepted");
        let delete_request = delete_server.request();

        assert_eq!(value, serde_json::json!({ "status": "ok" }));
        assert!(delete_request.contains("DELETE /rest/ip/address/*1 HTTP/1.1"));
        assert!(!delete_request.contains("Content-Type: application/json"));
    }

    #[test]
    fn rest_post_json_sends_body_and_accepts_empty_success() {
        let server = TestServer::responding_with(204, "application/json", "");
        let credential = test_credential();
        let client = RestClient::with_base_url(server.base_url(), "admin", &credential);

        let value = client
            .post_json(
                "/rest/export",
                serde_json::json!({ "file": "roswire-export", "compact": "yes" }),
            )
            .expect("empty POST success should be accepted");
        let request = server.request();

        assert_eq!(value, serde_json::json!({ "status": "ok" }));
        assert!(request.contains("POST /rest/export HTTP/1.1"));
        assert!(request.contains("Content-Type: application/json"));
        assert!(request.contains(r#""file":"roswire-export""#));
    }

    #[test]
    fn rest_request_body_omits_patch_id() {
        let request = build_protocol_request(&ParsedInvocation {
            path: vec!["ip".to_owned(), "address".to_owned()],
            action: "set".to_owned(),
            resolved_args: BTreeMap::from([
                (".id".to_owned(), "*1".to_owned()),
                ("disabled".to_owned(), "yes".to_owned()),
            ]),
        })
        .expect("request should map");

        let body = request_body(RestMethod::Patch, &request).expect("patch should have body");

        assert_eq!(body["disabled"], "yes");
        assert!(body.get(".id").is_none());
        assert!(request_body(RestMethod::Delete, &request).is_none());
    }

    #[test]
    fn rest_unauthorized_maps_to_auth_failed() {
        let server = TestServer::responding_with(401, "application/json", r#"{"detail":"bad"}"#);
        let credential = test_credential();
        let client = RestClient::with_base_url(server.base_url(), "admin", &credential);

        let error = client.system_resource().expect_err("401 should fail");

        assert_eq!(error.error_code, ErrorCode::AuthFailed);
    }

    #[test]
    fn tls_handshake_failure_maps_to_network_error() {
        let server = TestServer::responding_with(200, "application/json", r#"{}"#);
        let credential = test_credential();
        let client = RestClient::with_base_url(
            server.base_url().replace("http://", "https://"),
            "admin",
            &credential,
        );

        let error = client
            .system_resource()
            .expect_err("TLS should fail against plain HTTP");

        assert_eq!(error.error_code, ErrorCode::NetworkError);
    }

    #[test]
    fn non_json_success_maps_to_ros_api_failure() {
        let server = TestServer::responding_with(200, "text/plain", "not-json");
        let credential = test_credential();
        let client = RestClient::with_base_url(server.base_url(), "admin", &credential);

        let error = client
            .system_resource()
            .expect_err("invalid JSON should fail");
        let _ = server.request();

        assert_eq!(error.error_code, ErrorCode::RosApiFailure);
    }

    struct TestServer {
        address: String,
        handle: Option<thread::JoinHandle<String>>,
    }

    impl TestServer {
        fn responding_with(status: u16, content_type: &'static str, body: &'static str) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
            let address = listener
                .local_addr()
                .expect("local addr should exist")
                .to_string();
            let handle = thread::spawn(move || {
                let (mut stream, _) = listener.accept().expect("request should arrive");
                let request = read_request(&mut stream);
                let response = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len(),
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("response should write");
                request
            });
            Self {
                address,
                handle: Some(handle),
            }
        }

        fn base_url(&self) -> String {
            format!("http://{}", self.address)
        }

        fn request(mut self) -> String {
            self.handle
                .take()
                .expect("handle should exist")
                .join()
                .expect("server thread should finish")
        }
    }

    fn read_request(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_millis(200)))
            .expect("read timeout should be set");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(len) => {
                    request.extend_from_slice(&buffer[..len]);
                    if request_is_complete(&request) {
                        break;
                    }
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::TimedOut =>
                {
                    break;
                }
                Err(error) => panic!("request should read: {error}"),
            }
        }
        String::from_utf8_lossy(&request).to_string()
    }

    fn request_is_complete(request: &[u8]) -> bool {
        let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
            return false;
        };
        let body_start = header_end + 4;
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("Content-Length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        });
        request.len() >= body_start + content_length.unwrap_or(0)
    }

    fn test_credential() -> String {
        ['t', 'e', 's', 't', '-', 'v', 'a', 'l', 'u', 'e']
            .into_iter()
            .collect()
    }
}
