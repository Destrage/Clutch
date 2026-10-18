//! The WebSocket server: one `tokio` thread for I/O, sessions per client. Requests are queued
//! for the app thread ([`Server::pending`]); events are pushed with [`Server::broadcast`].

use crate::protocol::{self, Encoding, close, intent, op, status};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

/// A failed request: an OBS status code and an optional explanation.
#[derive(Clone, Debug, PartialEq)]
pub struct RequestError {
    pub code: u16,
    pub comment: Option<String>,
}

impl RequestError {
    pub fn new(code: u16, comment: impl Into<String>) -> Self {
        Self {
            code,
            comment: Some(comment.into()),
        }
    }
}

pub type Reply = Result<Value, RequestError>;

/// Message counters of the session a request came from (for `GetStats`).
#[derive(Clone, Debug, Default)]
pub struct SessionCounters {
    pub incoming: Arc<AtomicU64>,
    pub outgoing: Arc<AtomicU64>,
}

/// A request waiting for the app thread.
pub struct PendingRequest {
    pub request_type: String,
    pub data: Value,
    pub session: SessionCounters,
    reply: oneshot::Sender<Reply>,
}

impl PendingRequest {
    pub fn respond(self, reply: Reply) {
        let _ = self.reply.send(reply);
    }
}

enum Outgoing {
    Message(Arc<Value>),
}

struct Client {
    identified: Arc<AtomicBool>,
    subscriptions: Arc<AtomicU32>,
    tx: mpsc::Sender<Outgoing>,
}

#[derive(Default)]
struct Shared {
    clients: Mutex<HashMap<u64, Client>>,
    next_client: AtomicU64,
    /// Frame duration of the main canvas, for `Sleep` with `sleepFrames`.
    frame_ns: AtomicU64,
}

impl Shared {
    fn clients(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Client>> {
        self.clients.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The running server.
pub struct Server {
    port: u16,
    shared: Arc<Shared>,
    requests: crossbeam_channel::Receiver<PendingRequest>,
    shutdown: watch::Sender<Option<u16>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("port {0} is already in use")]
    PortInUse(u16),
    #[error("couldn't start the server: {0}")]
    Io(#[from] std::io::Error),
}

impl Server {
    /// Listens on all interfaces at `port` (0 picks a free port). With a password, clients
    /// must authenticate.
    pub fn start(port: u16, password: Option<String>) -> Result<Self, ServerError> {
        let listener = std::net::TcpListener::bind(("0.0.0.0", port)).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AddrInUse {
                ServerError::PortInUse(port)
            } else {
                ServerError::Io(e)
            }
        })?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let shared = Arc::new(Shared {
            frame_ns: AtomicU64::new(16_666_667),
            ..Default::default()
        });
        let (req_tx, requests) = crossbeam_channel::unbounded();
        let (shutdown, shutdown_rx) = watch::channel(None);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let thread = {
            let shared = shared.clone();
            let password = password.filter(|p| !p.is_empty()).map(Arc::new);
            std::thread::Builder::new()
                .name("estream-api".into())
                .spawn(move || {
                    runtime.block_on(serve(listener, shared, password, req_tx, shutdown_rx));
                })?
        };
        tracing::info!(port, "remote control server listening");
        Ok(Self {
            port,
            shared,
            requests,
            shutdown,
            thread: Some(thread),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Requests waiting for the app thread.
    pub fn pending(&self) -> impl Iterator<Item = PendingRequest> + '_ {
        self.requests.try_iter()
    }

    /// Sends an event to every identified client subscribed to `intent`.
    pub fn broadcast(&self, event_type: &str, event_intent: u32, data: Option<Value>) {
        let mut d = json!({ "eventType": event_type, "eventIntent": event_intent });
        if let Some(data) = data {
            d["eventData"] = data;
        }
        let message = Arc::new(protocol::message(op::EVENT, d));
        for client in self.shared.clients().values() {
            if client.identified.load(Ordering::Acquire)
                && client.subscriptions.load(Ordering::Acquire) & event_intent != 0
            {
                // A client that can't keep up loses events rather than stalling the app.
                let _ = client.tx.try_send(Outgoing::Message(message.clone()));
            }
        }
    }

    /// True if any identified client subscribes to `intent` (skip building unwanted events).
    pub fn wants(&self, event_intent: u32) -> bool {
        self.shared.clients().values().any(|c| {
            c.identified.load(Ordering::Acquire)
                && c.subscriptions.load(Ordering::Acquire) & event_intent != 0
        })
    }

    /// Identified (connected and authenticated) clients.
    pub fn client_count(&self) -> usize {
        self.shared
            .clients()
            .values()
            .filter(|c| c.identified.load(Ordering::Acquire))
            .count()
    }

    /// Frame duration used for `Sleep` with `sleepFrames`.
    pub fn set_frame_time(&self, frame: Duration) {
        self.shared
            .frame_ns
            .store(frame.as_nanos().max(1) as u64, Ordering::Relaxed);
    }

    /// Clutch is exiting: sends `ExitStarted` and closes every session.
    pub fn shutdown(mut self) {
        self.broadcast("ExitStarted", intent::GENERAL, None);
        self.stop(close::GOING_AWAY);
    }

    /// The settings changed: closes every session with "session invalidated".
    pub fn invalidate(mut self) {
        self.stop(close::SESSION_INVALIDATED);
    }

    fn stop(&mut self, code: u16) {
        let _ = self.shutdown.send(Some(code));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop(close::GOING_AWAY);
    }
}

async fn serve(
    listener: std::net::TcpListener,
    shared: Arc<Shared>,
    password: Option<Arc<String>>,
    requests: crossbeam_channel::Sender<PendingRequest>,
    mut shutdown: watch::Receiver<Option<u16>>,
) {
    let listener = match tokio::net::TcpListener::from_std(listener) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = %e, "remote control listener failed");
            return;
        }
    };
    let mut sessions = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                if let Ok((stream, peer)) = accepted {
                    tracing::debug!(%peer, "remote control connection");
                    let _ = stream.set_nodelay(true);
                    sessions.spawn(session(
                        stream,
                        shared.clone(),
                        password.clone(),
                        requests.clone(),
                        shutdown.clone(),
                    ));
                }
            }
            _ = shutdown.changed() => break,
        }
    }
    // Let sessions send their close frames.
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        while sessions.join_next().await.is_some() {}
    })
    .await;
}

/// What a message handler wants the session to do next.
enum Flow {
    Continue,
    Close(u16, String),
}

struct Session {
    identified: Arc<AtomicBool>,
    subscriptions: Arc<AtomicU32>,
    auth: Option<(Arc<String>, String, String)>,
    tx: mpsc::Sender<Outgoing>,
    requests: crossbeam_channel::Sender<PendingRequest>,
    counters: SessionCounters,
    shared: Arc<Shared>,
}

async fn session(
    stream: tokio::net::TcpStream,
    shared: Arc<Shared>,
    password: Option<Arc<String>>,
    requests: crossbeam_channel::Sender<PendingRequest>,
    mut shutdown: watch::Receiver<Option<u16>>,
) {
    use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};

    let mut encoding = None;
    // The error type is fixed by tungstenite's handshake callback signature.
    #[allow(clippy::result_large_err)]
    let callback = |req: &Request, mut resp: Response| -> Result<Response, ErrorResponse> {
        let offered = req
            .headers()
            .get("Sec-WebSocket-Protocol")
            .and_then(|v| v.to_str().ok());
        let Some(chosen) = Encoding::negotiate(offered) else {
            let mut err = ErrorResponse::new(Some("unsupported subprotocol".into()));
            *err.status_mut() = tokio_tungstenite::tungstenite::http::StatusCode::BAD_REQUEST;
            return Err(err);
        };
        if offered.is_some_and(|o| !o.trim().is_empty()) {
            resp.headers_mut().insert(
                "Sec-WebSocket-Protocol",
                tokio_tungstenite::tungstenite::http::HeaderValue::from_static(chosen.protocol()),
            );
        }
        encoding = Some(chosen);
        Ok(resp)
    };
    let ws = match tokio_tungstenite::accept_hdr_async(stream, callback).await {
        Ok(ws) => ws,
        Err(e) => {
            tracing::debug!(error = %e, "websocket handshake failed");
            return;
        }
    };
    let encoding = encoding.unwrap_or_default();
    let (mut sink, mut source) = ws.split();
    let (tx, mut rx) = mpsc::channel::<Outgoing>(1024);
    let id = shared.next_client.fetch_add(1, Ordering::Relaxed);
    let session = Session {
        identified: Arc::new(AtomicBool::new(false)),
        subscriptions: Arc::new(AtomicU32::new(0)),
        auth: password.map(|p| (p, protocol::random_token(32), protocol::random_token(32))),
        tx: tx.clone(),
        requests,
        counters: SessionCounters::default(),
        shared: shared.clone(),
    };
    shared.clients().insert(
        id,
        Client {
            identified: session.identified.clone(),
            subscriptions: session.subscriptions.clone(),
            tx,
        },
    );

    let hello = protocol::hello(
        session
            .auth
            .as_ref()
            .map(|(_, challenge, salt)| (challenge.as_str(), salt.as_str())),
    );
    let mut close_with: Option<(u16, String)> = None;
    if let Ok(msg) = protocol::encode(&hello, encoding)
        && sink.send(msg).await.is_ok()
    {
        session.counters.outgoing.fetch_add(1, Ordering::Relaxed);
        loop {
            tokio::select! {
                incoming = source.next() => {
                    let Some(Ok(msg)) = incoming else { break };
                    if matches!(msg, Message::Close(_)) {
                        break;
                    }
                    let Some(decoded) = protocol::decode(&msg, encoding) else { continue };
                    session.counters.incoming.fetch_add(1, Ordering::Relaxed);
                    let flow = match decoded {
                        Ok(value) => session.handle(value),
                        Err(e) => Flow::Close(close::MESSAGE_DECODE_ERROR, e),
                    };
                    if let Flow::Close(code, reason) = flow {
                        close_with = Some((code, reason));
                        break;
                    }
                }
                outgoing = rx.recv() => {
                    let Some(Outgoing::Message(value)) = outgoing else { break };
                    let Ok(msg) = protocol::encode(&value, encoding) else { continue };
                    if sink.send(msg).await.is_err() {
                        break;
                    }
                    session.counters.outgoing.fetch_add(1, Ordering::Relaxed);
                }
                _ = shutdown.changed() => {
                    let code = (*shutdown.borrow()).unwrap_or(close::GOING_AWAY);
                    // Flush what's queued (e.g. ExitStarted) before closing.
                    while let Ok(Outgoing::Message(value)) = rx.try_recv() {
                        if let Ok(msg) = protocol::encode(&value, encoding) {
                            let _ = sink.send(msg).await;
                        }
                    }
                    close_with = Some((code, "Clutch closed the session".into()));
                    break;
                }
            }
        }
    }
    shared.clients().remove(&id);
    if let Some((code, reason)) = close_with {
        let frame = CloseFrame {
            code: CloseCode::from(code),
            reason: reason.into(),
        };
        let _ = sink.send(Message::Close(Some(frame))).await;
        let _ = tokio::time::timeout(Duration::from_millis(500), async {
            while let Some(Ok(_)) = source.next().await {}
        })
        .await;
    }
}

fn field_u64(d: &Value, key: &str) -> Result<Option<u64>, Flow> {
    match d.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| {
            Flow::Close(
                close::INVALID_DATA_FIELD_TYPE,
                format!("`{key}` must be a number"),
            )
        }),
    }
}

impl Session {
    fn send(&self, value: Value) {
        let _ = self.tx.try_send(Outgoing::Message(Arc::new(value)));
    }

    fn handle(&self, value: Value) -> Flow {
        match self.dispatch(value) {
            Ok(()) => Flow::Continue,
            Err(flow) => flow,
        }
    }

    fn dispatch(&self, value: Value) -> Result<(), Flow> {
        let Some(code) = value.get("op").and_then(Value::as_u64) else {
            return Err(Flow::Close(
                close::MISSING_DATA_FIELD,
                "missing `op`".into(),
            ));
        };
        let d = value.get("d").cloned().unwrap_or(Value::Null);
        if !d.is_object() {
            return Err(Flow::Close(
                close::MISSING_DATA_FIELD,
                "missing `d` object".into(),
            ));
        }
        let identified = self.identified.load(Ordering::Acquire);
        match code {
            op::IDENTIFY if identified => Err(Flow::Close(
                close::ALREADY_IDENTIFIED,
                "already identified".into(),
            )),
            op::IDENTIFY => self.identify(&d),
            _ if !identified => Err(Flow::Close(close::NOT_IDENTIFIED, "identify first".into())),
            op::REIDENTIFY => {
                if let Some(subs) = field_u64(&d, "eventSubscriptions")? {
                    self.subscriptions.store(subs as u32, Ordering::Release);
                }
                self.send(protocol::message(
                    op::IDENTIFIED,
                    json!({ "negotiatedRpcVersion": protocol::RPC_VERSION }),
                ));
                Ok(())
            }
            op::REQUEST => self.request(d),
            op::REQUEST_BATCH => self.batch(d),
            _ => Err(Flow::Close(
                close::UNKNOWN_OP_CODE,
                format!("unknown op {code}"),
            )),
        }
    }

    fn identify(&self, d: &Value) -> Result<(), Flow> {
        let Some(rpc) = field_u64(d, "rpcVersion")? else {
            return Err(Flow::Close(
                close::MISSING_DATA_FIELD,
                "missing `rpcVersion`".into(),
            ));
        };
        if rpc != protocol::RPC_VERSION {
            return Err(Flow::Close(
                close::UNSUPPORTED_RPC_VERSION,
                format!("RPC version {rpc} isn't supported (use 1)"),
            ));
        }
        if let Some((password, challenge, salt)) = &self.auth {
            let expected = protocol::auth_response(password, salt, challenge);
            let given = d.get("authentication").and_then(Value::as_str);
            if given != Some(expected.as_str()) {
                return Err(Flow::Close(
                    close::AUTHENTICATION_FAILED,
                    "authentication failed".into(),
                ));
            }
        }
        let subs = field_u64(d, "eventSubscriptions")?.unwrap_or(intent::ALL as u64);
        self.subscriptions.store(subs as u32, Ordering::Release);
        self.identified.store(true, Ordering::Release);
        self.send(protocol::message(
            op::IDENTIFIED,
            json!({ "negotiatedRpcVersion": protocol::RPC_VERSION }),
        ));
        Ok(())
    }

    fn request(&self, d: Value) -> Result<(), Flow> {
        let Some(request_id) = d.get("requestId").cloned().filter(|v| !v.is_null()) else {
            return Err(Flow::Close(
                close::MISSING_DATA_FIELD,
                "missing `requestId`".into(),
            ));
        };
        let request_type = d.get("requestType").and_then(Value::as_str);
        let Some(request_type) = request_type.map(str::to_owned) else {
            self.send(protocol::message(
                op::REQUEST_RESPONSE,
                response(
                    "",
                    Some(&request_id),
                    &Err(RequestError::new(
                        status::MISSING_REQUEST_TYPE,
                        "missing `requestType`",
                    )),
                ),
            ));
            return Ok(());
        };
        let data = d.get("requestData").cloned().unwrap_or(Value::Null);
        let requests = self.requests.clone();
        let counters = self.counters.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let reply = if request_type == "Sleep" {
                Err(RequestError::new(
                    status::UNSUPPORTED_REQUEST_BATCH_EXECUTION_TYPE,
                    "Sleep only works in request batches",
                ))
            } else {
                run(&requests, &counters, &request_type, data).await
            };
            let message = protocol::message(
                op::REQUEST_RESPONSE,
                response(&request_type, Some(&request_id), &reply),
            );
            let _ = tx.send(Outgoing::Message(Arc::new(message))).await;
        });
        Ok(())
    }

    fn batch(&self, d: Value) -> Result<(), Flow> {
        let Some(request_id) = d.get("requestId").cloned().filter(|v| !v.is_null()) else {
            return Err(Flow::Close(
                close::MISSING_DATA_FIELD,
                "missing `requestId`".into(),
            ));
        };
        let Some(list) = d.get("requests").and_then(Value::as_array).cloned() else {
            return Err(Flow::Close(
                close::MISSING_DATA_FIELD,
                "missing `requests` array".into(),
            ));
        };
        let halt = d
            .get("haltOnFailure")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let execution = d.get("executionType").and_then(Value::as_i64).unwrap_or(0);
        if !(0..=2).contains(&execution) {
            return Err(Flow::Close(
                close::INVALID_DATA_FIELD_VALUE,
                format!("unsupported executionType {execution}"),
            ));
        }
        let requests = self.requests.clone();
        let counters = self.counters.clone();
        let tx = self.tx.clone();
        let frame_ns = self.shared.frame_ns.load(Ordering::Relaxed);
        tokio::spawn(async move {
            let mut results = Vec::with_capacity(list.len());
            for item in list {
                let request_type = item
                    .get("requestType")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let data = item.get("requestData").cloned().unwrap_or(Value::Null);
                let reply = match request_type.as_str() {
                    "" => Err(RequestError::new(
                        status::MISSING_REQUEST_TYPE,
                        "missing `requestType`",
                    )),
                    "Sleep" => sleep(execution, &data, frame_ns).await,
                    _ => run(&requests, &counters, &request_type, data).await,
                };
                let failed = reply.is_err();
                results.push(response(&request_type, item.get("requestId"), &reply));
                if halt && failed {
                    break;
                }
            }
            let message = protocol::message(
                op::REQUEST_BATCH_RESPONSE,
                json!({ "requestId": request_id, "results": results }),
            );
            let _ = tx.send(Outgoing::Message(Arc::new(message))).await;
        });
        Ok(())
    }
}

/// `Sleep` inside a batch: `sleepMillis` (serial realtime) or `sleepFrames` (serial frame).
async fn sleep(execution: i64, data: &Value, frame_ns: u64) -> Reply {
    let millis = |key: &str, max: u64| -> Result<u64, RequestError> {
        let v = data.get(key).ok_or_else(|| {
            RequestError::new(status::MISSING_REQUEST_FIELD, format!("missing `{key}`"))
        })?;
        let n = v.as_u64().ok_or_else(|| {
            RequestError::new(
                status::INVALID_REQUEST_FIELD_TYPE,
                format!("`{key}` must be a number"),
            )
        })?;
        if n > max {
            return Err(RequestError::new(
                status::REQUEST_FIELD_OUT_OF_RANGE,
                format!("`{key}` must be at most {max}"),
            ));
        }
        Ok(n)
    };
    let duration = match execution {
        0 => Duration::from_millis(millis("sleepMillis", 50_000)?),
        1 => Duration::from_nanos(millis("sleepFrames", 10_000)? * frame_ns),
        _ => {
            return Err(RequestError::new(
                status::UNSUPPORTED_REQUEST_BATCH_EXECUTION_TYPE,
                "Sleep isn't available in parallel batches",
            ));
        }
    };
    tokio::time::sleep(duration).await;
    Ok(Value::Null)
}

/// Hands a request to the app thread and waits for the reply.
async fn run(
    requests: &crossbeam_channel::Sender<PendingRequest>,
    counters: &SessionCounters,
    request_type: &str,
    data: Value,
) -> Reply {
    let (reply, rx) = oneshot::channel();
    let pending = PendingRequest {
        request_type: request_type.to_owned(),
        data,
        session: counters.clone(),
        reply,
    };
    if requests.send(pending).is_err() {
        return Err(RequestError::new(
            status::NOT_READY,
            "Clutch is shutting down",
        ));
    }
    match tokio::time::timeout(Duration::from_secs(30), rx).await {
        Ok(Ok(reply)) => reply,
        _ => Err(RequestError::new(
            status::NOT_READY,
            "Clutch didn't answer in time",
        )),
    }
}

/// A RequestResponse body (also used for batch results).
fn response(request_type: &str, request_id: Option<&Value>, reply: &Reply) -> Value {
    let mut d = json!({ "requestType": request_type });
    if let Some(id) = request_id {
        d["requestId"] = id.clone();
    }
    match reply {
        Ok(data) => {
            d["requestStatus"] = json!({ "result": true, "code": status::SUCCESS });
            if !data.is_null() {
                d["responseData"] = data.clone();
            }
        }
        Err(e) => {
            let mut s = json!({ "result": false, "code": e.code });
            if let Some(comment) = &e.comment {
                s["comment"] = comment.clone().into();
            }
            d["requestStatus"] = s;
        }
    }
    d
}
