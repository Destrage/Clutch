# Tasks: add-music-source

## 1. Jamendo
- [x] 1.1 Client ID from `.secrets/clutch.env` or the environment (build.rs)
- [x] 1.2 Track search by tags with license filter; parse title, artist, license, stream URL
- [x] 1.3 Tests against recorded API responses

## 2. Source
- [x] 2.1 `music` source type: settings (genre/tags, safe for monetized streams, shuffle,
      Skip button), playlist refill, skip, play/pause
- [x] 2.2 Play HTTPS stream URLs in the player; keep playing and retry on network trouble
- [x] 2.3 Credit text file (shown on screen by a Text source)

## 3. Studio and docs
- [x] 3.1 In the source picker; Now playing and Skip in its settings
- [ ] 3.2 Website docs page and testing guide scenario

## 4. Check
- [ ] 4.1 Tests; in-app check of genres, skip, filter, credit, and a build without a client ID
