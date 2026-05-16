pub mod dialect;
pub mod login;
pub mod sentence;
pub mod transport;

use crate::error::{RosWireError, RosWireResult};
use crate::mapping::ProtocolRequest;
use crate::protocol::classic::dialect::{ClassicDialect, Dialect};
use crate::protocol::RouterOsMajor;
use sentence::{parse_api_sentence, read_sentence, write_sentence, SentenceKind};
use std::collections::BTreeMap;
use transport::ApiStream;

#[derive(Debug)]
pub struct ClassicApiSession<S> {
    stream: S,
}

impl<S: ApiStream> ClassicApiSession<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    pub fn login(&mut self, user: &str, password: &str) -> RosWireResult<()> {
        login::login(&mut self.stream, user, password)
    }

    pub fn probe_resource(&mut self) -> RosWireResult<ResourceInfo> {
        probe_resource(&mut self.stream)
    }

    pub fn execute_request(
        &mut self,
        request: &ProtocolRequest,
    ) -> RosWireResult<Vec<BTreeMap<String, String>>> {
        self.execute_words(&request.classic_api_words())
    }

    pub fn execute_words(
        &mut self,
        words: &[String],
    ) -> RosWireResult<Vec<BTreeMap<String, String>>> {
        write_sentence(&mut self.stream, words)?;

        let mut rows = Vec::new();
        loop {
            let sentence_words = read_sentence(&mut self.stream)?;
            let sentence = parse_api_sentence(&sentence_words)?;
            match sentence.kind {
                SentenceKind::Re => rows.push(sentence.attributes),
                SentenceKind::Done => return Ok(rows),
                SentenceKind::Trap | SentenceKind::Fatal => {
                    return Err(Box::new(sentence.trap_error().unwrap_or_else(|| {
                        RosWireError::ros_api_failure("RouterOS API command failed")
                    })));
                }
                SentenceKind::Other(kind) => {
                    return Err(Box::new(RosWireError::ros_api_failure(format!(
                        "RouterOS API returned unsupported sentence kind: {kind}",
                    ))));
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceInfo {
    pub version: String,
    pub architecture: String,
    pub board_name: String,
}

impl ResourceInfo {
    pub fn routeros_major(&self) -> RouterOsMajor {
        ClassicDialect::from_resource_info(self).routeros_major()
    }
}

pub fn probe_resource<S: ApiStream + ?Sized>(stream: &mut S) -> RosWireResult<ResourceInfo> {
    write_sentence(
        stream,
        &[
            "/system/resource/print".to_owned(),
            "=.proplist=version,architecture-name,architecture,board-name".to_owned(),
        ],
    )?;

    let mut row = None;
    loop {
        let words = read_sentence(stream)?;
        let sentence = parse_api_sentence(&words)?;
        match sentence.kind {
            SentenceKind::Re => {
                row = Some(sentence.attributes);
            }
            SentenceKind::Done => {
                let Some(attributes) = row else {
                    return Err(Box::new(RosWireError::ros_api_failure(
                        "RouterOS resource probe returned no rows",
                    )));
                };
                return resource_info_from_attributes(&attributes);
            }
            SentenceKind::Trap | SentenceKind::Fatal => {
                return Err(Box::new(sentence.trap_error().unwrap_or_else(|| {
                    RosWireError::ros_api_failure("resource probe failed")
                })));
            }
            SentenceKind::Other(_) => continue,
        }
    }
}

fn resource_info_from_attributes(
    attributes: &BTreeMap<String, String>,
) -> RosWireResult<ResourceInfo> {
    let version = required_attribute(attributes, "version")?;
    let architecture = attributes
        .get("architecture-name")
        .or_else(|| attributes.get("architecture"))
        .cloned()
        .ok_or_else(|| {
            Box::new(RosWireError::ros_api_failure(
                "RouterOS resource probe did not include architecture",
            ))
        })?;
    let board_name = required_attribute(attributes, "board-name")?;

    Ok(ResourceInfo {
        version,
        architecture,
        board_name,
    })
}

fn required_attribute(attributes: &BTreeMap<String, String>, name: &str) -> RosWireResult<String> {
    attributes.get(name).cloned().ok_or_else(|| {
        Box::new(RosWireError::ros_api_failure(format!(
            "RouterOS resource probe did not include {name}",
        )))
    })
}

#[cfg(test)]
mod tests {
    use super::{probe_resource, ClassicApiSession, ResourceInfo};
    use crate::args::ParsedInvocation;
    use crate::error::ErrorCode;
    use crate::mapping::build_protocol_request;
    use crate::protocol::classic::sentence::{read_sentence, write_sentence};
    use crate::protocol::RouterOsMajor;
    use std::collections::BTreeMap;
    use std::io::{Cursor, Read, Result, Write};

    struct FakeApiStream {
        rx: Cursor<Vec<u8>>,
        tx: Vec<u8>,
    }

    impl FakeApiStream {
        fn with_sentences(sentences: &[Vec<String>]) -> Self {
            let mut rx = Vec::new();
            for sentence in sentences {
                write_sentence(&mut rx, sentence).expect("fixture sentence should encode");
            }
            Self {
                rx: Cursor::new(rx),
                tx: Vec::new(),
            }
        }

        fn written_sentences(&self) -> Vec<Vec<String>> {
            let mut cursor = Cursor::new(self.tx.clone());
            let mut sentences = Vec::new();
            while (cursor.position() as usize) < cursor.get_ref().len() {
                sentences.push(read_sentence(&mut cursor).expect("written sentence should decode"));
            }
            sentences
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

    #[test]
    fn resource_probe_parses_version_architecture_and_board() {
        let mut stream = FakeApiStream::with_sentences(&[
            vec![
                "!re".to_owned(),
                "=version=7.15.3".to_owned(),
                "=architecture-name=arm64".to_owned(),
                "=board-name=RB5009".to_owned(),
            ],
            vec!["!done".to_owned()],
        ]);

        let info = probe_resource(&mut stream).expect("resource probe should succeed");

        assert_eq!(info.version, "7.15.3");
        assert_eq!(info.architecture, "arm64");
        assert_eq!(info.board_name, "RB5009");
        assert_eq!(info.routeros_major(), RouterOsMajor::V7);
        assert_eq!(stream.written_sentences()[0][0], "/system/resource/print");
    }

    #[test]
    fn resource_info_major_handles_v6_v7_and_unknown() {
        let mut info = ResourceInfo {
            version: "6.49.10".to_owned(),
            architecture: "mipsbe".to_owned(),
            board_name: "RB2011".to_owned(),
        };
        assert_eq!(info.routeros_major(), RouterOsMajor::V6);

        info.version = "7.15.3".to_owned();
        assert_eq!(info.routeros_major(), RouterOsMajor::V7);

        info.version = "unknown".to_owned();
        assert_eq!(info.routeros_major(), RouterOsMajor::Unknown);
    }

    #[test]
    fn executor_collects_re_rows_until_done() {
        let stream = FakeApiStream::with_sentences(&[
            vec![
                "!re".to_owned(),
                "=.id=*1".to_owned(),
                "=name=ether1".to_owned(),
            ],
            vec![
                "!re".to_owned(),
                "=.id=*2".to_owned(),
                "=name=bridge".to_owned(),
            ],
            vec!["!done".to_owned()],
        ]);
        let mut session = ClassicApiSession::new(stream);
        let request = build_protocol_request(&ParsedInvocation {
            path: vec!["interface".to_owned()],
            action: "print".to_owned(),
            resolved_args: BTreeMap::new(),
        })
        .expect("request should map");

        let rows = session
            .execute_request(&request)
            .expect("executor should succeed");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("name").map(String::as_str), Some("ether1"));
        assert_eq!(rows[1].get(".id").map(String::as_str), Some("*2"));
        assert_eq!(session.stream.written_sentences()[0][0], "/interface/print");
    }

    #[test]
    fn executor_maps_trap_to_ros_api_failure() {
        let stream = FakeApiStream::with_sentences(&[vec![
            "!trap".to_owned(),
            "=message=no such item".to_owned(),
        ]]);
        let mut session = ClassicApiSession::new(stream);

        let error = session
            .execute_words(&["/ip/address/print".to_owned()])
            .expect_err("trap should fail");

        assert_eq!(error.error_code, ErrorCode::RosApiFailure);
        assert_eq!(error.message, "no such item");
    }
}
