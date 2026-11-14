//! The client's side: find the owner, say hello, ask.

use std::time::Duration;

use crate::frame::{read_message, write_message};
use crate::proto::{NOT_OWNER_KEY, Role, VERSION_ERROR_KEY};
use crate::transport::{self, BoxStream};
use crate::{Endpoint, Error, PROTOCOL_VERSION, Request, Response};

/// How long the owner may take to answer hello.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

/// What [`Client::reach`] found at an endpoint.
#[derive(Debug)]
#[non_exhaustive]
pub enum Reach {
    /// The owner answered.
    Connected(Client),
    /// Nobody serves the endpoint: no app is running.
    Nobody,
    /// A process serves it but owns nothing right now: another process
    /// (a standalone `boltget`) holds the database.
    NotOwner,
}

/// A connection to the owner of a data directory.
pub struct Client {
    pub(crate) io: BoxStream,
    server: String,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("server", &self.server)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// Connects to the owner at `endpoint` and agrees on the protocol.
    /// `Ok(None)` when no process serves it (no app is running), or the one
    /// that does owns nothing right now (another process holds the
    /// database); an error
    /// when one does but cannot be talked to, for example
    /// [`Error::Version`] when it speaks another protocol version.
    /// `client` names the caller in the owner's logs.
    pub async fn connect(endpoint: &Endpoint, client: &str) -> Result<Option<Self>, Error> {
        match Self::reach(endpoint, client, None).await? {
            Reach::Connected(c) => Ok(Some(c)),
            Reach::Nobody | Reach::NotOwner => Ok(None),
        }
    }

    /// [`Client::connect`] in a `role`, telling apart nobody serving the
    /// endpoint from a process that serves it but owns nothing. The native
    /// host connects as [`Role::Browser`], and may then send only
    /// [`Request::Browser`].
    pub async fn reach(
        endpoint: &Endpoint,
        client: &str,
        role: Option<Role>,
    ) -> Result<Reach, Error> {
        let Some(mut io) = transport::connect(endpoint).await? else {
            return Ok(Reach::Nobody);
        };
        let hello = Request::Hello {
            v: PROTOCOL_VERSION,
            client: client.to_owned(),
            role,
        };
        write_message(&mut io, &hello).await?;
        let reply = tokio::time::timeout(HELLO_TIMEOUT, read_message::<Response, _>(&mut io))
            .await
            .map_err(|_| Error::Protocol("no answer to hello".into()))??;
        match reply {
            Some(Response::Hello { v, server }) if v == PROTOCOL_VERSION => {
                Ok(Reach::Connected(Self { io, server }))
            }
            Some(Response::Hello { v, server }) => Err(Error::Version(format!(
                "{server} answered with pipe protocol {v}, this client speaks {PROTOCOL_VERSION}"
            ))),
            Some(Response::Error { error }) if error.key == VERSION_ERROR_KEY => {
                Err(Error::Version(error.message))
            }
            Some(Response::Error { error }) if error.key == NOT_OWNER_KEY => Ok(Reach::NotOwner),
            Some(Response::Error { error }) => Err(Error::Protocol(error.message)),
            Some(other) => Err(Error::Protocol(format!("{other:?} instead of hello"))),
            None => Err(Error::Closed),
        }
    }

    /// The owner, as it introduced itself (`Boltget 0.2.0`).
    pub fn server(&self) -> &str {
        &self.server
    }

    /// Sends a request and reads its response. Errors the owner reports
    /// arrive as [`Response::Error`]; only the pipe's own failures are an
    /// `Err`. After [`Request::Watch`], read the stream with
    /// [`Client::next`].
    pub async fn request(&mut self, req: &Request) -> Result<Response, Error> {
        write_message(&mut self.io, req).await?;
        self.next().await
    }

    /// Reads the next message: during [`Request::Watch`], the next event,
    /// progress report or the final [`Response::Finished`].
    pub async fn next(&mut self) -> Result<Response, Error> {
        read_message(&mut self.io).await?.ok_or(Error::Closed)
    }
}
