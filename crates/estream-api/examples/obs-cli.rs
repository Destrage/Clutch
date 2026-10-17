//! A tiny obs-websocket v5 client for trying the remote API by hand.
//!
//! ```text
//! cargo run -p estream-api --example obs-cli -- [--url ws://127.0.0.1:4455] [--password pw]
//!     <RequestType> [json-data] [<RequestType> [json-data] ...]
//! cargo run -p estream-api --example obs-cli -- --listen 10      # print events for 10 s
//! ```
#![allow(clippy::unwrap_used, clippy::expect_used)]

use estream_api::protocol;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut take = |flag: &str| -> Option<String> {
        let i = args.iter().position(|a| a == flag)?;
        let value = args.get(i + 1).cloned();
        args.drain(i..(i + 2).min(args.len()));
        value
    };
    let url = take("--url").unwrap_or_else(|| "ws://127.0.0.1:4455".into());
    let password = take("--password");
    let listen: Option<u64> = take("--listen").and_then(|s| s.parse().ok());

    let (mut ws, _) = tungstenite::connect(url.as_str()).expect("connect");
    let hello = read(&mut ws);
    let mut identify = json!({ "rpcVersion": 1 });
    if let Some(auth) = hello["d"].get("authentication") {
        let pw = password.as_deref().expect("the server wants --password");
        identify["authentication"] = protocol::auth_response(
            pw,
            auth["salt"].as_str().unwrap(),
            auth["challenge"].as_str().unwrap(),
        )
        .into();
    }
    send(&mut ws, json!({ "op": 1, "d": identify }));
    let identified = read(&mut ws);
    assert_eq!(identified["op"], 2, "not identified: {identified}");

    let mut requests = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let kind = args[i].clone();
        let data = args
            .get(i + 1)
            .and_then(|a| serde_json::from_str::<Value>(a).ok());
        i += if data.is_some() { 2 } else { 1 };
        requests.push((kind, data));
    }
    for (n, (kind, data)) in requests.into_iter().enumerate() {
        let id = format!("cli-{n}");
        let mut d = json!({ "requestType": kind, "requestId": id });
        if let Some(data) = data {
            d["requestData"] = data;
        }
        send(&mut ws, json!({ "op": 6, "d": d }));
        loop {
            let msg = read(&mut ws);
            if msg["op"] == 7 && msg["d"]["requestId"] == id {
                println!(
                    "{kind}: {}",
                    serde_json::to_string_pretty(&msg["d"]).unwrap()
                );
                break;
            }
            if msg["op"] == 5 {
                println!("event {}", msg["d"]);
            }
        }
    }
    if let Some(secs) = listen {
        let until = Instant::now() + Duration::from_secs(secs);
        if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_ref() {
            s.set_read_timeout(Some(Duration::from_millis(200)))
                .unwrap();
        }
        while Instant::now() < until {
            match ws.read() {
                Ok(tungstenite::Message::Text(t)) => println!("{t}"),
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => {
                    println!("closed: {e}");
                    break;
                }
            }
        }
    }
    let _ = ws.close(None);
}

type Socket = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;

fn send(ws: &mut Socket, v: Value) {
    ws.send(tungstenite::Message::text(v.to_string())).unwrap();
}

fn read(ws: &mut Socket) -> Value {
    loop {
        match ws.read().expect("read") {
            tungstenite::Message::Text(t) => return serde_json::from_str(t.as_str()).unwrap(),
            tungstenite::Message::Close(f) => panic!("closed: {f:?}"),
            _ => {}
        }
    }
}
