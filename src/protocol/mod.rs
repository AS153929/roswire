use crate::error::RosWireError;
use crate::error::RosWireResult;

pub mod classic;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestedProtocol {
    Auto,
    Rest,
    ApiSsl,
    Api,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SelectedProtocol {
    Rest,
    ApiSsl,
    Api,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouterOsMajor {
    Unknown,
    V6,
    V7,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeResult {
    Success {
        routeros_major: RouterOsMajor,
        rest_supported_for_action: bool,
    },
    NetworkFailure,
    AuthFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteDecision {
    pub requested_protocol: RequestedProtocol,
    pub selected_protocol: SelectedProtocol,
    pub routeros_major: RouterOsMajor,
}

pub trait ProtocolProbe {
    fn probe(&self, protocol: SelectedProtocol) -> ProbeResult;
}

pub fn route_protocol(
    requested: RequestedProtocol,
    action_has_rest_mapping: bool,
    port_override: Option<u16>,
    probe: &impl ProtocolProbe,
) -> RosWireResult<RouteDecision> {
    if requested == RequestedProtocol::Auto && port_override.is_some() {
        return Err(Box::new(RosWireError::config(
            "--port cannot be used with --protocol auto",
        )));
    }

    match requested {
        RequestedProtocol::Auto => auto_route(action_has_rest_mapping, probe),
        RequestedProtocol::Rest => explicit_route(SelectedProtocol::Rest, requested, probe),
        RequestedProtocol::ApiSsl => explicit_route(SelectedProtocol::ApiSsl, requested, probe),
        RequestedProtocol::Api => explicit_route(SelectedProtocol::Api, requested, probe),
    }
}

fn explicit_route(
    selected: SelectedProtocol,
    requested: RequestedProtocol,
    probe: &impl ProtocolProbe,
) -> RosWireResult<RouteDecision> {
    match probe.probe(selected) {
        ProbeResult::Success { routeros_major, .. } => Ok(RouteDecision {
            requested_protocol: requested,
            selected_protocol: selected,
            routeros_major,
        }),
        ProbeResult::AuthFailed => Err(Box::new(RosWireError::auth_failed(
            "authentication failed while probing protocol",
        ))),
        ProbeResult::NetworkFailure => Err(Box::new(RosWireError::network(
            "unable to reach RouterOS service for requested protocol",
        ))),
    }
}

fn auto_route(
    action_has_rest_mapping: bool,
    probe: &impl ProtocolProbe,
) -> RosWireResult<RouteDecision> {
    for candidate in [
        SelectedProtocol::Rest,
        SelectedProtocol::ApiSsl,
        SelectedProtocol::Api,
    ] {
        match probe.probe(candidate) {
            ProbeResult::AuthFailed => {
                return Err(Box::new(RosWireError::auth_failed(
                    "authentication failed during protocol auto-detection",
                )));
            }
            ProbeResult::NetworkFailure => continue,
            ProbeResult::Success {
                routeros_major,
                rest_supported_for_action,
            } => {
                if candidate == SelectedProtocol::Rest
                    && routeros_major == RouterOsMajor::V7
                    && (!action_has_rest_mapping || !rest_supported_for_action)
                {
                    continue;
                }

                if candidate == SelectedProtocol::Rest && routeros_major == RouterOsMajor::V6 {
                    continue;
                }

                return Ok(RouteDecision {
                    requested_protocol: RequestedProtocol::Auto,
                    selected_protocol: candidate,
                    routeros_major,
                });
            }
        }
    }

    Err(Box::new(RosWireError::network(
        "all protocol candidates failed during auto-detection",
    )))
}

#[cfg(test)]
mod tests {
    use super::{
        route_protocol, ProbeResult, ProtocolProbe, RequestedProtocol, RouterOsMajor,
        SelectedProtocol,
    };
    use crate::error::ErrorCode;
    use std::collections::BTreeMap;

    struct FakeProbe {
        responses: BTreeMap<SelectedProtocol, ProbeResult>,
    }

    impl ProtocolProbe for FakeProbe {
        fn probe(&self, protocol: SelectedProtocol) -> ProbeResult {
            self.responses
                .get(&protocol)
                .copied()
                .unwrap_or(ProbeResult::NetworkFailure)
        }
    }

    #[test]
    fn auto_prefers_rest_when_v7_and_mapped() {
        let probe = FakeProbe {
            responses: BTreeMap::from([(
                SelectedProtocol::Rest,
                ProbeResult::Success {
                    routeros_major: RouterOsMajor::V7,
                    rest_supported_for_action: true,
                },
            )]),
        };

        let decision = route_protocol(RequestedProtocol::Auto, true, None, &probe)
            .expect("auto should choose rest");
        assert_eq!(decision.selected_protocol, SelectedProtocol::Rest);
    }

    #[test]
    fn auto_falls_back_when_rest_unavailable() {
        let probe = FakeProbe {
            responses: BTreeMap::from([
                (SelectedProtocol::Rest, ProbeResult::NetworkFailure),
                (
                    SelectedProtocol::ApiSsl,
                    ProbeResult::Success {
                        routeros_major: RouterOsMajor::V7,
                        rest_supported_for_action: false,
                    },
                ),
            ]),
        };

        let decision = route_protocol(RequestedProtocol::Auto, true, None, &probe)
            .expect("auto should fall back to api-ssl");
        assert_eq!(decision.selected_protocol, SelectedProtocol::ApiSsl);
    }

    #[test]
    fn auto_falls_back_when_action_has_no_rest_mapping() {
        let probe = FakeProbe {
            responses: BTreeMap::from([
                (
                    SelectedProtocol::Rest,
                    ProbeResult::Success {
                        routeros_major: RouterOsMajor::V7,
                        rest_supported_for_action: true,
                    },
                ),
                (
                    SelectedProtocol::Api,
                    ProbeResult::Success {
                        routeros_major: RouterOsMajor::V7,
                        rest_supported_for_action: false,
                    },
                ),
            ]),
        };

        let decision = route_protocol(RequestedProtocol::Auto, false, None, &probe)
            .expect("auto should choose classic api when no rest mapping");
        assert_eq!(decision.selected_protocol, SelectedProtocol::Api);
    }

    #[test]
    fn explicit_protocol_is_not_overridden() {
        let probe = FakeProbe {
            responses: BTreeMap::from([(
                SelectedProtocol::Api,
                ProbeResult::Success {
                    routeros_major: RouterOsMajor::V6,
                    rest_supported_for_action: false,
                },
            )]),
        };

        let decision = route_protocol(RequestedProtocol::Api, true, None, &probe)
            .expect("explicit api should succeed");
        assert_eq!(decision.selected_protocol, SelectedProtocol::Api);
    }

    #[test]
    fn auth_failure_short_circuits_auto_fallback() {
        let probe = FakeProbe {
            responses: BTreeMap::from([(SelectedProtocol::Rest, ProbeResult::AuthFailed)]),
        };

        let error = route_protocol(RequestedProtocol::Auto, true, None, &probe)
            .expect_err("auth failure should stop auto fallback");
        assert_eq!(error.error_code, ErrorCode::AuthFailed);
    }

    #[test]
    fn auto_with_port_override_returns_config_error() {
        let probe = FakeProbe {
            responses: BTreeMap::new(),
        };

        let error = route_protocol(RequestedProtocol::Auto, true, Some(443), &probe)
            .expect_err("auto + port should be rejected");
        assert_eq!(error.error_code, ErrorCode::ConfigError);
    }
}
