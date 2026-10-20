// Clutch events relay (Cloudflare Worker).
//
// Kick delivers channel events (follows, subs, gifts, Kicks) only by webhook, to one address per
// app. This Worker receives them, checks Kick's signature, and passes each event to the Clutch
// apps connected for that channel. Nothing is stored.
//
//   POST /kick/webhook   Kick → relay (signed)
//   GET  /kick/live      Clutch → relay, a WebSocket; `Authorization: Bearer <Kick access token>`.
//                        The token is checked with Kick, so a connection only ever gets the
//                        events of the channel that signed in.

import { importKey, verify, broadcasterOf, fresh } from "./kick.js";

const KICK_API = "https://api.kick.com/public/v1";

// Kick's public key, cached (Kick may rotate it, so it is fetched, not hardcoded).
let keyCache = null; // { key, at }
const KEY_TTL_MS = 60 * 60 * 1000;

async function kickKey(refresh = false) {
  if (!refresh && keyCache && Date.now() - keyCache.at < KEY_TTL_MS) return keyCache.key;
  const res = await fetch(`${KICK_API}/public-key`);
  if (!res.ok) throw new Error(`public key: ${res.status}`);
  const pem = (await res.json())?.data?.public_key;
  if (!pem) throw new Error("public key missing");
  keyCache = { key: await importKey(pem), at: Date.now() };
  return keyCache.key;
}

// Which channel a Kick access token belongs to, cached briefly.
const tokenCache = new Map(); // token → { id, at }
const TOKEN_TTL_MS = 10 * 60 * 1000;

async function channelOfToken(token) {
  const hit = tokenCache.get(token);
  if (hit && Date.now() - hit.at < TOKEN_TTL_MS) return hit.id;
  const res = await fetch(`${KICK_API}/users`, { headers: { Authorization: `Bearer ${token}` } });
  if (!res.ok) return null;
  const id = (await res.json())?.data?.[0]?.user_id;
  if (!Number.isSafeInteger(id) || id <= 0) return null;
  if (tokenCache.size > 1000) tokenCache.clear();
  tokenCache.set(token, { id: String(id), at: Date.now() });
  return String(id);
}

async function webhook(request, env) {
  const body = await request.text();
  const messageId = request.headers.get("Kick-Event-Message-Id");
  const timestamp = request.headers.get("Kick-Event-Message-Timestamp");
  const signature = request.headers.get("Kick-Event-Signature");
  const type = request.headers.get("Kick-Event-Type");
  if (!fresh(timestamp)) return new Response("stale", { status: 400 });
  let ok = await verify(await kickKey(), messageId, timestamp, body, signature);
  // A failure right after Kick rotates its key: fetch it again once.
  if (!ok) ok = await verify(await kickKey(true), messageId, timestamp, body, signature);
  if (!ok) return new Response("bad signature", { status: 401 });

  let event;
  try {
    event = JSON.parse(body);
  } catch {
    return new Response("bad body", { status: 400 });
  }
  const channel = broadcasterOf(event);
  if (!channel) return new Response("no channel", { status: 400 });
  const stub = env.CHANNELS.get(env.CHANNELS.idFromName(channel));
  await stub.fetch("https://relay/publish", {
    method: "POST",
    body: JSON.stringify({ id: messageId, message: JSON.stringify({ type: "event", event: type, data: event }) }),
  });
  return new Response("ok");
}

async function live(request, env) {
  if (request.headers.get("Upgrade")?.toLowerCase() !== "websocket") {
    return new Response("expected a WebSocket", { status: 426 });
  }
  const token = request.headers.get("Authorization")?.replace(/^Bearer\s+/i, "");
  const channel = token ? await channelOfToken(token) : null;
  if (!channel) return new Response("sign in to Kick", { status: 401 });
  const stub = env.CHANNELS.get(env.CHANNELS.idFromName(channel));
  return stub.fetch("https://relay/connect", { headers: { Upgrade: "websocket" } });
}

export default {
  async fetch(request, env) {
    const { pathname } = new URL(request.url);
    if (pathname === "/kick/webhook" && request.method === "POST") return webhook(request, env);
    if (pathname === "/kick/live") return live(request, env);
    if (pathname === "/") return new Response("Clutch events relay");
    return new Response("not found", { status: 404 });
  },
};

/** One Kick channel: the Clutch apps connected to it, and passing events to them. */
export class Channel {
  constructor(state) {
    this.state = state;
    // Clutch pings every 30 s; answered without waking the object.
    state.setWebSocketAutoResponse(new WebSocketRequestResponsePair("ping", "pong"));
    // Kick may deliver a webhook more than once; recent message ids are skipped.
    this.recent = [];
  }

  async fetch(request) {
    const { pathname } = new URL(request.url);
    if (pathname === "/connect") {
      const [client, server] = Object.values(new WebSocketPair());
      this.state.acceptWebSocket(server);
      return new Response(null, { status: 101, webSocket: client });
    }
    if (pathname === "/publish") {
      const { id, message } = await request.json();
      if (id && this.recent.includes(id)) return new Response("duplicate");
      if (id) {
        this.recent.push(id);
        if (this.recent.length > 100) this.recent.shift();
      }
      for (const socket of this.state.getWebSockets()) {
        try {
          socket.send(message);
        } catch {
          // A closing socket; the runtime cleans it up.
        }
      }
      return new Response("ok");
    }
    return new Response("not found", { status: 404 });
  }

  webSocketMessage() {}

  webSocketClose(socket, code) {
    try {
      socket.close(code, "bye");
    } catch {
      // Already closed.
    }
  }
}
