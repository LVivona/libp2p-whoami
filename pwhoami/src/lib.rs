use libp2p::{
    futures::{AsyncReadExt, AsyncWriteExt},
    PeerId, Stream, StreamProtocol,
};
use libp2p_stream as stream;
use serde::{Deserialize, Deserializer, Serialize};
use std::{
    fmt::{self, Debug},
    io,
    str::FromStr,
};

use cbor4ii::serde::from_slice;

pub const WHOAMI_PROTOCOL: StreamProtocol = StreamProtocol::new("/whoami/0.0.1");

#[derive(Debug)]
pub enum UserAgent {
    Client,
    Server,
}

#[derive(Debug)]
pub struct ParseUserAgentError;

impl fmt::Display for ParseUserAgentError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Invalid UserAgent string")
    }
}

impl FromStr for UserAgent {
    type Err = ParseUserAgentError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with("rust-client") {
            Ok(UserAgent::Client)
        } else if s.starts_with("rust-server") {
            Ok(UserAgent::Server)
        } else {
            Err(ParseUserAgentError)
        }
    }
}

impl Serialize for UserAgent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            UserAgent::Client => serializer.serialize_str("rust-client/0.0.1"),
            UserAgent::Server => serializer.serialize_str("rust-server/0.0.1"),
        }
    }
}

fn deserialize_user_agent<'de, D>(deserializer: D) -> Result<UserAgent, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    UserAgent::from_str(&s).map_err(serde::de::Error::custom)
}

#[warn(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    #[serde(deserialize_with = "deserialize_user_agent")]
    pub user_agent: UserAgent,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    #[serde(deserialize_with = "deserialize_user_agent")]
    user_agent: UserAgent,
    f_name: String,
    l_name: Option<String>,
}

impl Response {
    pub fn new(agent: UserAgent, f_name: String, l_name: Option<String>) -> Self {
        Self {
            user_agent: agent,
            f_name,
            l_name,
        }
    }
}

pub async fn send_request<RequestT: Serialize>(
    mut stream: Stream,
    request: &RequestT,
) -> io::Result<()> {
    let bytes = match cbor4ii::serde::to_vec(Vec::new(), request) {
        Ok(bytes) => bytes,
        Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e.to_string())),
    };

    tracing::info!("Sending request...");
    stream.write_all(&bytes).await?;
    stream.flush().await?; // Added flush to ensure data is sent
    tracing::info!("Request sent, waiting for response...");

    // Use a fixed-size buffer for reading and track actual bytes read
    let mut buffer = vec![0u8; 4096];
    let mut total_bytes = 0;

    // Read chunks until we have data or encounter an error/EOF
    loop {
        match stream.read(&mut buffer[total_bytes..]).await {
            Ok(0) => break, // EOF reached
            Ok(n) => {
                total_bytes += n;
                // Resize buffer if needed
                if total_bytes >= buffer.len() {
                    buffer.resize(buffer.len() * 2, 0);
                }
            }
            Err(e) => return Err(e),
        }

        // Try to parse what we have so far
        if let Ok(response) = from_slice::<Response>(&buffer[..total_bytes]) {
            tracing::info!(?total_bytes, ?response, "Received valid response");

            // Wait a moment before closing to ensure peer has processed everything
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            stream.close().await?;
            return Ok(());
        }
    }

    // If we reached here, we got EOF but couldn't parse a valid response
    if total_bytes > 0 {
        match from_slice::<Response>(&buffer[..total_bytes]) {
            Ok(response) => {
                tracing::info!(?total_bytes, ?response, "Received valid response at EOF");
                stream.close().await?;
                Ok(())
            }
            Err(e) => {
                let err = io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Failed to deserialize response: {}", e),
                );
                tracing::warn!(?total_bytes, ?err, "Deserialization failed");
                stream.close().await?;
                Err(err)
            }
        }
    } else {
        let err = io::Error::new(io::ErrorKind::UnexpectedEof, "No data received");
        tracing::warn!(?err);
        stream.close().await?;
        Err(err)
    }
}

pub async fn read_bytes(mut stream: Stream) -> io::Result<()> {
    // Use a fixed-size buffer for reading and track actual bytes read
    let mut buffer = vec![0u8; 4096];
    let mut total_bytes = 0;

    // Read chunks until we have data or encounter an error/EOF
    loop {
        match stream.read(&mut buffer[total_bytes..]).await {
            Ok(0) => break, // EOF reached
            Ok(n) => {
                total_bytes += n;
                // Resize buffer if needed
                if total_bytes >= buffer.len() {
                    buffer.resize(buffer.len() * 2, 0);
                }
            }
            Err(e) => return Err(e),
        }
    }

    if total_bytes > 0 {
        match from_slice::<Response>(&buffer[..total_bytes]) {
            Ok(response) => {
                tracing::info!(?total_bytes, ?response, "Received and parsed response");
                // Wait a moment before closing to ensure peer has processed everything
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                stream.close().await?;
                Ok(())
            }
            Err(e) => {
                let err = io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Failed to deserialize response: {}", e),
                );
                tracing::warn!(?total_bytes, ?err, "Deserialization failed");
                stream.close().await?;
                Err(err)
            }
        }
    } else {
        let err = io::Error::new(io::ErrorKind::UnexpectedEof, "No data received");
        tracing::warn!(?err);
        stream.close().await?;
        Err(err)
    }
}

pub async fn send_response<ResponseT: Serialize>(
    mut stream: Stream,
    response: &ResponseT,
) -> io::Result<()> {
    tracing::info!("Request received, preparing response...");

    let bytes = match cbor4ii::serde::to_vec(Vec::new(), response) {
        Ok(bytes) => bytes,
        Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e.to_string())),
    };

    stream.write_all(&bytes).await?;
    stream.flush().await?;
    tracing::info!("Response sent successfully");

    // Wait a moment before closing to ensure peer has processed everything
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    // stream.close().await?;

    Ok(())
}

pub async fn on_connection<RequestT: Serialize + Debug>(
    peer: PeerId,
    mut control: stream::Control,
    request: RequestT,
    tx: tokio::sync::mpsc::Sender<bool>,
) {
    tracing::info!(%peer, "Attempting to open stream");
    let stream = match control.open_stream(peer, WHOAMI_PROTOCOL).await {
        Ok(stream) => {
            tracing::info!(%peer, "Stream opened successfully");
            stream
        }
        Err(error @ stream::OpenStreamError::UnsupportedProtocol(_)) => {
            tracing::warn!(%peer, %error, "Protocol not supported");
            return;
        }
        Err(error) => {
            tracing::warn!(%peer, %error, "Failed to open stream");
            return;
        }
    };

    match send_request::<RequestT>(stream, &request).await {
        Ok(_) => {
            tracing::info!(%peer, ?request, "Request completed successfully");
            let _ = tx.send(true).await;
        }
        Err(e) => tracing::warn!(%peer, ?request, "Echo protocol failed: {}", e),
    }
}
