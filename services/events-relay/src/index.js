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
//
// Only Clutch can use it: a sign-in must have been made through Clutch's Kick app (KICK_CLIENT_ID),
// and an event must come from one of that app's own subscriptions (listed with the app's secret,
// KICK_CLIENT_SECRET). Other Kick apps pointing their webhooks here, or their users' sign-ins,
// are refused.

import { importKey, verify, broadcasterOf, fresh } from "./kick.js";

const KICK_API = "https://api.kick.com/public/v1";
const KICK_ID = "https://id.kick.com/oauth";

// Kick's public key, cached (Kick may rotate it, so it is fetched, not hardcoded).
let keyCache = null; // { key, at }
const KEY_TTL_MS = 60 * 60 * 1000;

async function kickKey(refresh = false) {
  const age = keyCache ? Date.now() - keyCache.at : Infinity;
  // A refresh after a failed check happens at most once a minute, so forged webhooks can't make
  // the relay keep asking Kick for its key.
  if (keyCache && age < (refresh ? 60 * 1000 : KEY_TTL_MS)) return keyCache.key;
  const res = await fetch(`${KICK_API}/public-key`);
  if (!res.ok) throw new Error(`public key: ${res.status}`);
  const pem = (await res.json())?.data?.public_key;
  if (!pem) throw new Error("public key missing");
  keyCache = { key: await importKey(pem), at: Date.now() };
  return keyCache.key;
}

// Clutch's own event subscriptions (ids), from Kick with an app token, cached and refreshed when
// an unknown id arrives (at most every 30 s, so unknown ids can't make it hammer Kick).
let appToken = null; // { token, until }
let subscriptions = null; // { ids: Set, at }
const SUBS_TTL_MS = 10 * 60 * 1000;
const SUBS_MIN_REFRESH_MS = 30 * 1000;

async function kickAppToken(env) {
  if (appToken && Date.now() < appToken.until) return appToken.token;
  const res = await fetch(`${KICK_ID}/token`, {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      grant_type: "client_credentials",
      client_id: env.KICK_CLIENT_ID,
      client_secret: env.KICK_CLIENT_SECRET,
    }),
  });
  if (!res.ok) throw new Error(`app token: ${res.status}`);
  const body = await res.json();
  appToken = { token: body.access_token, until: Date.now() + Math.max(60, (body.expires_in ?? 3600) - 60) * 1000 };
  return appToken.token;
}

async function ourSubscription(env, id) {
  if (!id) return false;
  const age = subscriptions ? Date.now() - subscriptions.at : Infinity;
  // Known and recent: yes. Unknown but just refreshed: no (a new one waits for the next refresh).
  if (subscriptions?.ids.has(id) && age < SUBS_TTL_MS) return true;
  if (age < SUBS_MIN_REFRESH_MS) return false;
  const res = await fetch(`${KICK_API}/events/subscriptions`, {
    headers: { Authorization: `Bearer ${await kickAppToken(env)}` },
  });
  if (!res.ok) throw new Error(`subscriptions: ${res.status}`);
  const list = (await res.json())?.data ?? [];
  subscriptions = { ids: new Set(list.map((s) => s.id)), at: Date.now() };
  return subscriptions.ids.has(id);
}

// Which channel a Kick access token belongs to, cached briefly. Only sign-ins made through
// Clutch's Kick app count.
const tokenCache = new Map(); // token → { id, at }
const TOKEN_TTL_MS = 10 * 60 * 1000;
// Refused sign-ins, remembered so trying the same one again costs nothing.
const refused = new Map(); // token → at
const REFUSED_TTL_MS = 10 * 60 * 1000;

async function channelOfToken(env, token) {
  const hit = tokenCache.get(token);
  if (hit && Date.now() - hit.at < TOKEN_TTL_MS) return hit.id;
  const no = refused.get(token);
  if (no && Date.now() - no < REFUSED_TTL_MS) return null;
  const id = await askKick(env, token);
  if (id) {
    if (tokenCache.size > 1000) tokenCache.clear();
    tokenCache.set(token, { id, at: Date.now() });
  } else {
    if (refused.size > 5000) refused.clear();
    refused.set(token, Date.now());
  }
  return id;
}

/** The channel of a sign-in made through Clutch's Kick app, or null. */
async function askKick(env, token) {
  const check = await fetch(`${KICK_ID}/token/introspect`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}` },
  });
  if (!check.ok) return null;
  const info = (await check.json())?.data;
  if (!info?.active || info.token_type !== "user" || info.client_id !== env.KICK_CLIENT_ID) return null;
  const res = await fetch(`${KICK_API}/users`, { headers: { Authorization: `Bearer ${token}` } });
  if (!res.ok) return null;
  const id = (await res.json())?.data?.[0]?.user_id;
  if (!Number.isSafeInteger(id) || id <= 0) return null;
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
  // Signed by Kick, but for another Kick app's subscription: not ours to pass on.
  if (!(await ourSubscription(env, request.headers.get("Kick-Event-Subscription-Id")))) {
    return new Response("not a Clutch subscription", { status: 403 });
  }

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
  // Each address gets a few tries a minute; more are turned away before Kick is asked anything.
  const ip = request.headers.get("CF-Connecting-IP") ?? "unknown";
  const { success } = await env.LIVE_LIMIT.limit({ key: ip });
  if (!success) return new Response("too many attempts, try again in a minute", { status: 429 });
  const token = request.headers.get("Authorization")?.replace(/^Bearer\s+/i, "");
  const channel = token ? await channelOfToken(env, token) : null;
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
