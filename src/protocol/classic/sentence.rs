use crate::error::{RosWireError, RosWireResult};
use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SentenceKind {
    Re,
    Done,
    Trap,
    Fatal,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiSentence {
    pub kind: SentenceKind,
    pub attributes: BTreeMap<String, String>,
}

impl ApiSentence {
    pub fn trap_error(&self) -> Option<RosWireError> {
        match self.kind {
            SentenceKind::Trap | SentenceKind::Fatal => {
                let message = self
                    .attributes
                    .get("message")
                    .cloned()
                    .unwrap_or_else(|| "RouterOS returned an API failure".to_owned());
                Some(RosWireError::ros_api_failure(message))
            }
            _ => None,
        }
    }
}

pub fn encode_word_length(length: usize) -> RosWireResult<Vec<u8>> {
    if length < 0x80 {
        Ok(vec![length as u8])
    } else if length < 0x4000 {
        let value = (length as u16) | 0x8000;
        Ok(value.to_be_bytes().to_vec())
    } else if length < 0x20_0000 {
        let value = (length as u32) | 0x00C0_0000;
        Ok(vec![
            ((value >> 16) & 0xFF) as u8,
            ((value >> 8) & 0xFF) as u8,
            (value & 0xFF) as u8,
        ])
    } else if length < 0x1000_0000 {
        let value = (length as u32) | 0xE000_0000;
        Ok(value.to_be_bytes().to_vec())
    } else if length <= u32::MAX as usize {
        let mut bytes = Vec::with_capacity(5);
        bytes.push(0xF0);
        bytes.extend_from_slice(&(length as u32).to_be_bytes());
        Ok(bytes)
    } else {
        Err(Box::new(RosWireError::usage(
            "RouterOS API word length exceeds u32::MAX",
        )))
    }
}

pub fn decode_word_length<R: Read + ?Sized>(reader: &mut R) -> RosWireResult<usize> {
    let mut first = [0_u8; 1];
    read_exact(reader, &mut first, "read word length")?;

    let first = first[0];
    if first & 0x80 == 0 {
        return Ok(first as usize);
    }

    if first & 0xC0 == 0x80 {
        let mut rest = [0_u8; 1];
        read_exact(reader, &mut rest, "read two-byte word length")?;
        let length = (((first & !0xC0) as usize) << 8) | rest[0] as usize;
        return Ok(length);
    }

    if first & 0xE0 == 0xC0 {
        let mut rest = [0_u8; 2];
        read_exact(reader, &mut rest, "read three-byte word length")?;
        let length =
            (((first & !0xE0) as usize) << 16) | ((rest[0] as usize) << 8) | rest[1] as usize;
        return Ok(length);
    }

    if first & 0xF0 == 0xE0 {
        let mut rest = [0_u8; 3];
        read_exact(reader, &mut rest, "read four-byte word length")?;
        let length = (((first & !0xF0) as usize) << 24)
            | ((rest[0] as usize) << 16)
            | ((rest[1] as usize) << 8)
            | rest[2] as usize;
        return Ok(length);
    }

    if first == 0xF0 {
        let mut rest = [0_u8; 4];
        read_exact(reader, &mut rest, "read five-byte word length")?;
        return Ok(u32::from_be_bytes(rest) as usize);
    }

    Err(Box::new(RosWireError::network(
        "invalid RouterOS API word length prefix",
    )))
}

pub fn read_word<R: Read + ?Sized>(reader: &mut R) -> RosWireResult<Option<String>> {
    let length = decode_word_length(reader)?;
    if length == 0 {
        return Ok(None);
    }

    let mut bytes = vec![0_u8; length];
    read_exact(reader, &mut bytes, "read API word")?;
    String::from_utf8(bytes).map(Some).map_err(|error| {
        Box::new(RosWireError::ros_api_failure(format!(
            "RouterOS API word is not valid UTF-8: {error}",
        )))
    })
}

pub fn read_sentence<R: Read + ?Sized>(reader: &mut R) -> RosWireResult<Vec<String>> {
    let mut words = Vec::new();
    while let Some(word) = read_word(reader)? {
        words.push(word);
    }
    Ok(words)
}

pub fn write_word<W: Write + ?Sized>(writer: &mut W, word: &str) -> RosWireResult<()> {
    let length = encode_word_length(word.len())?;
    writer
        .write_all(&length)
        .map_err(|error| Box::new(io_error("write API word length", error)))?;
    writer
        .write_all(word.as_bytes())
        .map_err(|error| Box::new(io_error("write API word", error)))?;
    Ok(())
}

pub fn write_sentence<W: Write + ?Sized>(writer: &mut W, words: &[String]) -> RosWireResult<()> {
    for word in words {
        write_word(writer, word)?;
    }
    writer
        .write_all(&[0])
        .map_err(|error| Box::new(io_error("write API sentence terminator", error)))?;
    writer
        .flush()
        .map_err(|error| Box::new(io_error("flush API sentence", error)))?;
    Ok(())
}

pub fn parse_api_sentence(words: &[String]) -> RosWireResult<ApiSentence> {
    let Some(first) = words.first() else {
        return Err(Box::new(RosWireError::ros_api_failure(
            "RouterOS API returned an empty sentence",
        )));
    };

    let kind = match first.as_str() {
        "!re" => SentenceKind::Re,
        "!done" => SentenceKind::Done,
        "!trap" => SentenceKind::Trap,
        "!fatal" => SentenceKind::Fatal,
        other => SentenceKind::Other(other.to_owned()),
    };

    Ok(ApiSentence {
        kind,
        attributes: parse_attributes(&words[1..]),
    })
}

pub fn parse_attributes(words: &[String]) -> BTreeMap<String, String> {
    words
        .iter()
        .filter_map(|word| {
            let stripped = word.strip_prefix('=')?;
            let (key, value) = stripped.split_once('=')?;
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}

fn read_exact<R: Read + ?Sized>(
    reader: &mut R,
    buffer: &mut [u8],
    operation: &str,
) -> RosWireResult<()> {
    reader
        .read_exact(buffer)
        .map_err(|error| Box::new(io_error(operation, error)))
}

fn io_error(operation: &str, error: std::io::Error) -> RosWireError {
    let message = if error.kind() == ErrorKind::UnexpectedEof {
        format!("RouterOS API stream ended while attempting to {operation}")
    } else {
        format!("RouterOS API I/O error while attempting to {operation}: {error}")
    };
    RosWireError::network(message)
}

#[cfg(test)]
mod tests {
    use super::{
        decode_word_length, encode_word_length, parse_api_sentence, read_sentence, write_sentence,
        SentenceKind,
    };
    use crate::error::ErrorCode;
    use std::io::Cursor;

    #[test]
    fn encodes_word_length_boundaries() {
        assert_eq!(encode_word_length(0).unwrap(), vec![0x00]);
        assert_eq!(encode_word_length(0x7F).unwrap(), vec![0x7F]);
        assert_eq!(encode_word_length(0x80).unwrap(), vec![0x80, 0x80]);
        assert_eq!(encode_word_length(0x3FFF).unwrap(), vec![0xBF, 0xFF]);
        assert_eq!(encode_word_length(0x4000).unwrap(), vec![0xC0, 0x40, 0x00]);
        assert_eq!(
            encode_word_length(0x1F_FFFF).unwrap(),
            vec![0xDF, 0xFF, 0xFF],
        );
        assert_eq!(
            encode_word_length(0x20_0000).unwrap(),
            vec![0xE0, 0x20, 0x00, 0x00],
        );
        assert_eq!(
            encode_word_length(0x0FFF_FFFF).unwrap(),
            vec![0xEF, 0xFF, 0xFF, 0xFF],
        );
        assert_eq!(
            encode_word_length(0x1000_0000).unwrap(),
            vec![0xF0, 0x10, 0x00, 0x00, 0x00],
        );
    }

    #[test]
    fn decodes_word_length_boundaries() {
        for length in [
            0,
            0x7F,
            0x80,
            0x3FFF,
            0x4000,
            0x1F_FFFF,
            0x20_0000,
            0x0FFF_FFFF,
            0x1000_0000,
        ] {
            let encoded = encode_word_length(length).unwrap();
            let decoded = decode_word_length(&mut Cursor::new(encoded)).unwrap();
            assert_eq!(decoded, length);
        }
    }

    #[test]
    fn writes_and_reads_sentence_until_empty_word() {
        let words = vec![
            "/ip/address/print".to_owned(),
            "=.proplist=.id,address,interface".to_owned(),
        ];
        let mut bytes = Vec::new();

        write_sentence(&mut bytes, &words).expect("sentence should write");
        let decoded = read_sentence(&mut Cursor::new(bytes)).expect("sentence should read");

        assert_eq!(decoded, words);
    }

    #[test]
    fn invalid_length_prefix_maps_to_network_error() {
        let error = decode_word_length(&mut Cursor::new(vec![0xF8]))
            .expect_err("invalid prefix should fail");

        assert_eq!(error.error_code, ErrorCode::NetworkError);
    }

    #[test]
    fn parses_re_done_and_trap_sentences() {
        let re = parse_api_sentence(&[
            "!re".to_owned(),
            "=.id=*1".to_owned(),
            "=name=ether1".to_owned(),
        ])
        .expect("!re should parse");
        assert_eq!(re.kind, SentenceKind::Re);
        assert_eq!(re.attributes.get(".id").map(String::as_str), Some("*1"));
        assert_eq!(
            re.attributes.get("name").map(String::as_str),
            Some("ether1"),
        );

        let done = parse_api_sentence(&["!done".to_owned()]).expect("!done should parse");
        assert_eq!(done.kind, SentenceKind::Done);

        let trap = parse_api_sentence(&[
            "!trap".to_owned(),
            "=category=2".to_owned(),
            "=message=no such item".to_owned(),
        ])
        .expect("!trap should parse");
        assert_eq!(trap.kind, SentenceKind::Trap);
        let error = trap.trap_error().expect("trap should map to error");
        assert_eq!(error.error_code, ErrorCode::RosApiFailure);
        assert_eq!(error.message, "no such item");
    }
}
