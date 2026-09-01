use std::io::Error as IoError;

use lsp_server::ProtocolError;
use serde_json::Error as JsonError;
use thiserror::Error as ThisError;

#[derive(Debug, ThisError)]
pub(crate) enum Error {
    #[error("the language-server protocol failed: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("the language-server transport failed: {0}")]
    Transport(#[source] IoError),
    #[error("the language-server client disconnected")]
    Disconnected,
    #[error("could not encode the `{method}` response: {source}")]
    EncodeResponse {
        method: String,
        #[source]
        source: JsonError,
    },
}
