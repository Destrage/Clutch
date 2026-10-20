//! A small blocking WebSocket client for the chat readers: reads time out every second so a
//! reader notices when it should stop.

use std::net::TcpStream;
use std::time::Duration;
use tungstenite::stream::MaybeTlsStream;

pub type Socket = tungstenite::WebSocket<MaybeTlsStream<TcpStream>>;

pub fn connect(url: &str) -> Result<Socket, String> {
    let (socket, _) = tungstenite::connect(url).map_err(|e| e.to_string())?;
    Ok(with_timeout(socket))
}

/// Connects with a bearer token. `Err(Some(status))` when the server answered with an HTTP
/// error instead (401: the token was refused), `Err(None)` when it couldn't be reached.
pub fn connect_authorized(url: &str, token: &str) -> Result<Socket, Option<u16>> {
    use tungstenite::client::IntoClientRequest;
    let mut request = url.into_client_request().map_err(|_| None)?;
    let value = format!("Bearer {token}").parse().map_err(|_| None)?;
    request.headers_mut().insert("Authorization", value);
    match tungstenite::connect(request) {
        Ok((socket, _)) => Ok(with_timeout(socket)),
        Err(tungstenite::Error::Http(response)) => Err(Some(response.status().as_u16())),
        Err(e) => {
            tracing::debug!(error = %e, url, "WebSocket connection");
            Err(None)
        }
    }
}

fn with_timeout(socket: Socket) -> Socket {
    let tcp = match socket.get_ref() {
        MaybeTlsStream::Plain(s) => Some(s),
        MaybeTlsStream::Rustls(s) => Some(s.get_ref()),
        _ => None,
    };
    if let Some(tcp) = tcp {
        let _ = tcp.set_read_timeout(Some(Duration::from_secs(1)));
    }
    socket
}

/// What the next read brought.
pub enum Next {
    Text(String),
    /// Nothing yet (or a control frame); check whether to stop and read again.
    Idle,
    Closed,
}

pub fn next(socket: &mut Socket) -> Next {
    use tungstenite::{Error, Message};
    match socket.read() {
        Ok(Message::Text(text)) => Next::Text(text.to_string()),
        Ok(Message::Close(_)) => Next::Closed,
        Ok(_) => Next::Idle,
        Err(Error::Io(e))
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            Next::Idle
        }
        Err(_) => Next::Closed,
    }
}

pub fn send(socket: &mut Socket, text: &str) -> bool {
    socket
        .send(tungstenite::Message::text(text.to_owned()))
        .is_ok()
}
