// npm test
import test from "node:test";
import assert from "node:assert/strict";
import { importKey, verify, broadcasterOf, fresh } from "../src/kick.js";

async function keyPair() {
  const pair = await crypto.subtle.generateKey(
    { name: "RSASSA-PKCS1-v1_5", modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: "SHA-256" },
    true,
    ["sign", "verify"],
  );
  const spki = new Uint8Array(await crypto.subtle.exportKey("spki", pair.publicKey));
  const b64 = btoa(String.fromCharCode(...spki)).replace(/(.{64})/g, "$1\n");
  return { privateKey: pair.privateKey, pem: `-----BEGIN PUBLIC KEY-----\n${b64}\n-----END PUBLIC KEY-----\n` };
}

async function sign(privateKey, id, ts, body) {
  const data = new TextEncoder().encode(`${id}.${ts}.${body}`);
  const sig = new Uint8Array(await crypto.subtle.sign("RSASSA-PKCS1-v1_5", privateKey, data));
  return btoa(String.fromCharCode(...sig));
}

test("a webhook signed by Kick passes, a changed one doesn't", async () => {
  const { privateKey, pem } = await keyPair();
  const key = await importKey(pem);
  const body = JSON.stringify({ broadcaster: { user_id: 42 }, follower: { username: "ali" } });
  const ts = new Date().toISOString();
  const sig = await sign(privateKey, "01ABC", ts, body);
  assert.equal(await verify(key, "01ABC", ts, body, sig), true);
  assert.equal(await verify(key, "01ABC", ts, body.replace("ali", "eve"), sig), false);
  assert.equal(await verify(key, "01XYZ", ts, body, sig), false);
  assert.equal(await verify(key, "01ABC", ts, body, "not base64!"), false);
  assert.equal(await verify(key, "01ABC", ts, body, null), false);
});

test("a key that isn't Kick's is refused", async () => {
  const kick = await keyPair();
  const other = await keyPair();
  const body = "{}";
  const ts = new Date().toISOString();
  const forged = await sign(other.privateKey, "01", ts, body);
  assert.equal(await verify(await importKey(kick.pem), "01", ts, body, forged), false);
});

test("events name their channel", () => {
  assert.equal(broadcasterOf({ broadcaster: { user_id: 676 } }), "676");
  assert.equal(broadcasterOf({ broadcaster: { user_id: "676" } }), null);
  assert.equal(broadcasterOf({}), null);
});

test("old webhooks are stale", () => {
  const now = Date.parse("2026-10-21T10:00:00Z");
  assert.equal(fresh("2026-10-21T09:55:00Z", now), true);
  assert.equal(fresh("2026-10-21T09:00:00Z", now), false);
  assert.equal(fresh("not a date", now), false);
});
