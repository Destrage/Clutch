# Clutch events relay

A Cloudflare Worker at `events.destrosphere.com` that passes Kick's channel events (follows, subs,
gifted subs, Kicks) to the Clutch apps connected for that channel. Kick only delivers these events
by webhook to one public address per app, which a desktop app can't receive itself.

- `POST /kick/webhook`: Kick sends events here. Each one is checked against Kick's signature
  (the public key is fetched from Kick and cached) and refused when older than 10 minutes.
- `GET /kick/live`: Clutch connects here (WebSocket) with its Kick sign-in in the
  `Authorization` header. The relay asks Kick whose sign-in it is and connects it to that
  channel only.

Each channel is a Durable Object holding its open connections. Nothing is stored; an event with
no connection is dropped.

## Tests

```sh
npm test
```

## Deploying

1. `npm install`
2. `npx wrangler login` (opens Cloudflare in the browser; use the account that has
   destrosphere.com)
3. `npx wrangler deploy`. This creates `events.destrosphere.com` (a custom domain on the
   destrosphere.com zone) and the Durable Object.
4. On the Kick app (kick.com/settings/developer → the app → Edit): tick **Subscribe to
   events**, turn on **Enable webhooks**, and set the URL to
   `https://events.destrosphere.com/kick/webhook`.

Check it with `curl https://events.destrosphere.com/` ("Clutch events relay"), and watch events
arrive with `npm run logs`.

## Limits

Cloudflare's free plan covers beta use (Workers requests and SQLite Durable Objects). Kick allows
10,000 subscriptions per event type per app, so about 10,000 channels.
