## 1. Platform APIs
- [ ] 1.1 YouTube: list upcoming and active broadcasts with details and bound stream; categories
      for the channel's region; tests on recorded replies
- [ ] 1.2 YouTube: create (insert, then the video's category and made-for-kids), update, and bind;
      read one broadcast; delete only when it never went live; tests for that check
- [ ] 1.3 Twitch: read and update channel details; search categories; tag checks; add
      `channel:manage:broadcast` to the sign-in
- [ ] 1.4 Kick: read and update channel details; search categories; add `channel:write` to the
      sign-in (and on the Kick app)
- [ ] 1.5 A missing permission asks the user to connect again, as for stream keys

## 2. Details and settings
- [ ] 2.1 `details` per destination and `stream.ask_details` in settings, with defaults; old
      settings files load unchanged
- [ ] 2.2 Pre-fill: saved details, else YouTube's last broadcast, else the channel's current details

## 3. Go live window
- [ ] 3.1 Slint window: one section per connected platform being streamed to, a list of
      destinations streamed with a key only, per-section loading and errors
- [ ] 3.2 YouTube section: broadcast list with "uses this key" marks, New broadcast, the details
      form (title, description, privacy, category, made for kids, latency, auto-start, auto-stop)
- [ ] 3.3 Twitch section (title, category search, tags, language); Kick section (title, category
      search)
- [ ] 3.4 "Go live without asking" in the window and in Settings → Stream

## 4. Going live
- [ ] 4.1 Apply the details per platform on worker threads, then start the video; on a failure
      keep the window open with Try again and Go live without <platform>
- [ ] 4.2 Choosing a YouTube broadcast bound to another stream switches the destination's key first
- [ ] 4.3 Hotkey, macro and remote "start stream" use the remembered details without the window,
      and toast failures
- [ ] 4.4 A message per platform after going live, with a link to the YouTube broadcast
- [ ] 4.5 Remove beta 2's create-in-the-checker and the reconnect after it; the checker only reports
- [ ] 4.6 Cleanup of a created broadcast that never went live, checked against YouTube first

## 5. While live
- [ ] 5.1 Edit details in the live panel: the same window without the broadcast choice; saved
      without reconnecting; YouTube's latency and auto-start/auto-stop read-only

## 6. Docs and check
- [ ] 6.1 Website streaming page (going live, details per platform, reconnecting Twitch and Kick);
      testing guide
- [ ] 6.2 Spec: drop the "No broadcast to go live on" scenario from `add-account-stream-setup`
- [ ] 6.3 In-app check:
  - YouTube with none, one and two upcoming broadcasts;
  - YouTube with auto-start off;
  - Twitch and Kick title and category;
  - a hotkey go-live;
  - Edit details while live;
  - stopping before YouTube goes live.
