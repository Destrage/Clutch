// Kick webhook checks, kept apart from the Worker so they can be tested with Node.

/** The DER bytes of a PEM "PUBLIC KEY". */
export function pemToDer(pem) {
  const body = pem.replace(/-----(BEGIN|END) PUBLIC KEY-----/g, "").replace(/\s+/g, "");
  return Uint8Array.from(atob(body), (c) => c.charCodeAt(0));
}

/** Imports Kick's public key for checking signatures (RSA PKCS#1 v1.5 with SHA-256). */
export function importKey(pem) {
  return crypto.subtle.importKey(
    "spki",
    pemToDer(pem),
    { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" },
    false,
    ["verify"],
  );
}

/**
 * Whether a webhook is really from Kick: the signature covers
 * `message id + "." + timestamp + "." + raw body`.
 */
export async function verify(key, messageId, timestamp, body, signature) {
  if (!messageId || !timestamp || !signature) return false;
  let sig;
  try {
    sig = Uint8Array.from(atob(signature), (c) => c.charCodeAt(0));
  } catch {
    return false;
  }
  const data = new TextEncoder().encode(`${messageId}.${timestamp}.${body}`);
  return crypto.subtle.verify("RSASSA-PKCS1-v1_5", key, sig, data);
}

/** The channel (broadcaster user id) an event belongs to. */
export function broadcasterOf(event) {
  const id = event?.broadcaster?.user_id;
  return Number.isSafeInteger(id) && id > 0 ? String(id) : null;
}

/** Webhooks older than this are refused, so a captured one can't be replayed later. */
export const MAX_AGE_MS = 10 * 60 * 1000;

export function fresh(timestamp, now = Date.now()) {
  const at = Date.parse(timestamp);
  return Number.isFinite(at) && Math.abs(now - at) <= MAX_AGE_MS;
}
