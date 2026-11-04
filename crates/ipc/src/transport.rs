//! The byte stream under the protocol: a named pipe on Windows, a Unix
//! domain socket elsewhere. Both sides see a boxed `AsyncRead + AsyncWrite`.

use std::io;
#[cfg(windows)]
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::{Endpoint, Error};

/// A connected stream.
pub(crate) trait Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Stream for T {}

pub(crate) type BoxStream = Box<dyn Stream>;

/// How long a client waits while every pipe instance is busy.
#[cfg(windows)]
const BUSY_WAIT: Duration = Duration::from_secs(5);

#[cfg(windows)]
mod imp {
    use std::time::Instant;

    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer};

    use super::{BUSY_WAIT, BoxStream, Duration, Endpoint, Error, io};
    use crate::platform;

    const ERROR_FILE_NOT_FOUND: i32 = 2;
    const ERROR_ACCESS_DENIED: i32 = 5;
    const ERROR_PIPE_BUSY: i32 = 231;

    #[derive(Debug)]
    pub(crate) struct Listener {
        name: String,
        next: NamedPipeServer,
    }

    impl Listener {
        #[expect(
            clippy::unused_async,
            clippy::unused_async_trait_impl,
            reason = "the same signature as on Unix, where binding probes a stale socket"
        )]
        pub(crate) async fn bind(endpoint: &Endpoint) -> Result<Self, Error> {
            let name = endpoint.pipe_name().to_owned();
            let next = platform::create_pipe(&name, true).map_err(|e| {
                // The first instance exists already: another process owns it.
                if e.raw_os_error() == Some(ERROR_ACCESS_DENIED) {
                    Error::InUse(name.clone())
                } else {
                    Error::Io(e)
                }
            })?;
            Ok(Self { name, next })
        }

        /// Waits for the next client in this user's session.
        pub(crate) async fn accept(&mut self) -> io::Result<BoxStream> {
            loop {
                let connected = self.next.connect().await;
                // A new instance waits for the next client before this one
                // is handed out, so a client never finds the pipe missing.
                let fresh = platform::create_pipe(&self.name, false)?;
                let conn = std::mem::replace(&mut self.next, fresh);
                if let Err(e) = connected {
                    tracing::debug!(error = %e, "a pipe client went away while connecting");
                    continue;
                }
                match platform::client_in_session(&conn) {
                    Ok(true) => return Ok(Box::new(conn)),
                    Ok(false) => tracing::warn!("refused a pipe client from another session"),
                    Err(e) => tracing::warn!(error = %e, "could not check a pipe client"),
                }
            }
        }
    }

    pub(crate) async fn connect(endpoint: &Endpoint) -> io::Result<Option<BoxStream>> {
        let started = Instant::now();
        loop {
            match ClientOptions::new().open(endpoint.pipe_name()) {
                Ok(pipe) => {
                    platform::verify_server(&pipe)?;
                    return Ok(Some(Box::new(pipe)));
                }
                Err(e) if e.raw_os_error() == Some(ERROR_FILE_NOT_FOUND) => return Ok(None),
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                    if started.elapsed() > BUSY_WAIT {
                        return Err(e);
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::PathBuf;

    use tokio::net::{UnixListener, UnixStream};

    use super::{BoxStream, Endpoint, Error, io};

    #[derive(Debug)]
    pub(crate) struct Listener {
        listener: UnixListener,
        path: PathBuf,
        uid: u32,
    }

    impl Listener {
        pub(crate) async fn bind(endpoint: &Endpoint) -> Result<Self, Error> {
            let path = endpoint.socket_path().to_path_buf();
            let listener = match UnixListener::bind(&path) {
                Ok(l) => l,
                Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
                    if UnixStream::connect(&path).await.is_ok() {
                        return Err(Error::InUse(path.display().to_string()));
                    }
                    // Left behind by an owner that died.
                    std::fs::remove_file(&path)?;
                    UnixListener::bind(&path)?
                }
                Err(e) => return Err(e.into()),
            };
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            let uid = std::fs::metadata(&path)?.uid();
            Ok(Self {
                listener,
                path,
                uid,
            })
        }

        /// Waits for the next client of this user.
        pub(crate) async fn accept(&mut self) -> io::Result<BoxStream> {
            loop {
                let (stream, _) = self.listener.accept().await?;
                match stream.peer_cred() {
                    Ok(cred) if cred.uid() == self.uid => return Ok(Box::new(stream)),
                    Ok(_) => tracing::warn!("refused a socket client of another user"),
                    Err(e) => tracing::warn!(error = %e, "could not check a socket client"),
                }
            }
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    pub(crate) async fn connect(endpoint: &Endpoint) -> io::Result<Option<BoxStream>> {
        let path = endpoint.socket_path();
        match UnixStream::connect(path).await {
            // The socket sits in the user's own runtime (or data) directory,
            // which no other user can write to; the server checks our uid.
            Ok(stream) => Ok(Some(Box::new(stream))),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) =>
            {
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }
}

pub(crate) use imp::{Listener, connect};
