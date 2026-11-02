//! Platform connection to the endpoint named in the discovery file.

use std::io;

use tokio::io::{AsyncRead, AsyncWrite};

pub(crate) trait Stream: AsyncRead + AsyncWrite + Unpin {}
impl<T: AsyncRead + AsyncWrite + Unpin> Stream for T {}

#[cfg(unix)]
pub(crate) async fn open(endpoint: &str) -> io::Result<impl Stream> {
    tokio::net::UnixStream::connect(endpoint).await
}

#[cfg(windows)]
pub(crate) async fn open(endpoint: &str) -> io::Result<impl Stream> {
    use tokio::net::windows::named_pipe::ClientOptions;

    // ERROR_PIPE_BUSY: every instance is in use for a moment; retry briefly.
    const ERROR_PIPE_BUSY: i32 = 231;
    let mut attempts = 0;
    loop {
        match ClientOptions::new().open(endpoint) {
            Ok(client) => return Ok(client),
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY) && attempts < 20 => {
                attempts += 1;
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(error) => return Err(error),
        }
    }
}
