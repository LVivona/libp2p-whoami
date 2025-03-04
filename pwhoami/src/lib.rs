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

#[derive(Debug, Clone)]
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


trait Codec 
    where Self : Sized + Serialize + Debug
{

    /// encode Request into [`Vec<u8>`] where it will 
    /// then be sent over the network via stream behavoir. 
    fn encode(&self) -> io::Result<Vec<u8>> {
        match cbor4ii::serde::to_vec(Vec::new(), self) {
        Ok(bytes) => return Ok(bytes),
            Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e.to_string())),
        }
    }
}

impl Request {

    pub fn default() -> Self {
        Self {
            user_agent: UserAgent::Client,
        }
    }
}

impl Codec for Request {}


#[derive(Debug, Serialize, Deserialize, Clone)]
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

impl Codec for Response {}

pub async fn send_request(
    mut stream: Stream,
    reposne: &Request,
) -> io::Result<Response> {
    let bytes = reposne.encode()?;
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
            return Ok(response);
        }
    }

    // If we reached here, we got EOF but couldn't parse a valid response
    if total_bytes > 0 {
        match from_slice::<Response>(&buffer[..total_bytes]) {
            Ok(response) => {
                tracing::info!(?total_bytes, ?response, "Received valid response at EOF");
                stream.close().await?;
                Ok(response)
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

pub async fn send_response(
    mut stream: Stream,
    response: &Response,
) -> io::Result<()> {
    tracing::debug!("Request received, preparing response...");
    
    let mut buf = vec![0; 1024];
    let mut total_bytes = 0;

        // Read chunks until we have data or encounter an error/EOF
    loop {
            match stream.read(&mut buf[total_bytes..]).await {
                Ok(0) => break, // EOF reached
                Ok(n) => {
                    total_bytes += n;
                    // Resize buffer if needed
                    if total_bytes >= buf.len() {
                        buf.resize(buf.len() * 2, 0);
                    }
                }
                Err(e) => return Err(e),
            }
    
            // Try to parse what we have so far
            if let Ok(response) = from_slice::<Request>(&buf[..total_bytes]) {
                tracing::info!(?total_bytes, ?response, "Received valid response");
                break;
            }
    }
    
    let bytes = response.encode()?;

    stream.write_all(&bytes).await?;
    stream.flush().await?;
    tracing::debug!("Response sent successfully");

    // close stream as we are finised streaming
    stream.close().await?;

    Ok(())
}

pub async fn on_connection(
    peer: PeerId,
    mut control: stream::Control,
    request: Request,
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

    match send_request(stream, &request).await {
        Ok(res) => {
            tracing::info!(%peer, ?res, "Request completed successfully");
            let _ = tx.send(true).await;
        }
        Err(e) => tracing::warn!(%peer, ?request, "Echo protocol failed: {}", e),
    }
}
