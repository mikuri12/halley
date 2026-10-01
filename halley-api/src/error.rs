use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Connection,
    Protocol,
    InvalidRequest,
    NotFound,
    Ambiguous,
    Unsupported,
    VersionMismatch,
    Busy,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    message: String,
}

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

impl From<halley_ipc::CodecError> for Error {
    fn from(value: halley_ipc::CodecError) -> Self {
        Self::new(ErrorKind::Connection, value.to_string())
    }
}

impl From<halley_ipc::ServerError> for Error {
    fn from(value: halley_ipc::ServerError) -> Self {
        let kind = match value.kind {
            halley_ipc::ServerErrorKind::InvalidRequest => ErrorKind::InvalidRequest,
            halley_ipc::ServerErrorKind::NotFound => ErrorKind::NotFound,
            halley_ipc::ServerErrorKind::Ambiguous => ErrorKind::Ambiguous,
            halley_ipc::ServerErrorKind::Unsupported => ErrorKind::Unsupported,
            halley_ipc::ServerErrorKind::VersionMismatch => ErrorKind::VersionMismatch,
            halley_ipc::ServerErrorKind::Busy => ErrorKind::Busy,
            halley_ipc::ServerErrorKind::Internal => ErrorKind::Internal,
        };
        Self::new(kind, value.message)
    }
}
