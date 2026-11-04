//! Platform code. Only Windows needs any: the named pipe's ACL and session
//! checks. On Unix the socket's file permissions and peer credentials, which
//! `std` and Tokio expose safely, do the same job (`transport.rs`).

mod windows;

pub(crate) use self::windows::{client_in_session, create_pipe, user_sid, verify_server};
