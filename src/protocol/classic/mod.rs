pub mod login;
pub mod sentence;
pub mod transport;

use crate::error::{RosWireError, RosWireResult};
use crate::protocol::RouterOsMajor;
use sentence::{parse_api_sentence, read_sentence, write_sentence, SentenceKind};
use std::collections::BTreeMap;
use transport::ApiStream;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceInfo {
    pub version: String,
    pub architecture: String,
    pub board_name: String,
}

impl ResourceInfo {
    pub fn routeros_major(&self) -> RouterOsMajor {
        if self.version.starts_with('6') {
            RouterOsMajor::V6
        } else if self.version.starts_with('7') {
            RouterOsMajor::V7
        } else {
            RouterOsMajor::Unknown
        }
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
    use super::{probe_resource, ResourceInfo};
    use crate::protocol::classic::sentence::{read_sentence, write_sentence};
    use crate::protocol::RouterOsMajor;
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
}
