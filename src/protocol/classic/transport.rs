use crate::error::{RosWireError, RosWireResult};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub trait ApiStream: Read + Write + Send {}

impl<T> ApiStream for T where T: Read + Write + Send {}

#[derive(Debug)]
pub struct TcpApiStream {
    inner: TcpStream,
}

impl TcpApiStream {
    pub fn connect(host: &str, port: u16, timeout: Duration) -> RosWireResult<Self> {
        let mut addresses = (host, port).to_socket_addrs().map_err(|error| {
            Box::new(network_error(format!(
                "failed to resolve RouterOS API address: {error}",
            )))
        })?;

        let address = addresses.next().ok_or_else(|| {
            Box::new(network_error(
                "failed to resolve RouterOS API address: no socket addresses returned",
            ))
        })?;

        let stream = TcpStream::connect_timeout(&address, timeout).map_err(|error| {
            Box::new(network_error(format!(
                "failed to connect to RouterOS API at {host}:{port}: {error}",
            )))
        })?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|error| Box::new(map_io_error("set API read timeout", error)))?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|error| Box::new(map_io_error("set API write timeout", error)))?;

        Ok(Self { inner: stream })
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
    use super::{map_io_error, ApiStream};
    use crate::error::ErrorCode;
    use std::io::{Cursor, Read, Result, Write};

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
}
