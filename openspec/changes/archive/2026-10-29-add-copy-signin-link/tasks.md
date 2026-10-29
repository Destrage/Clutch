# Tasks: add-copy-signin-link

## 1. Copy the link
- [x] 1.1 `shell::copy_text`: plain text on the clipboard (Windows clipboard; wl-copy / xclip on Linux)
- [x] 1.2 Account action `copy` before connecting (YouTube, Kick): start the sign-in without opening a browser, copy its link, toast
- [x] 1.3 Account action `copy` while waiting (YouTube, Kick, Twitch code page): copy the page, toast

## 2. UI
- [x] 2.1 Account row: Copy link next to Connect (YouTube, Kick) and next to Open page while waiting

## 3. Verify
- [x] 3.1 `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test -p estream`
- [x] 3.2 In the app: copy the YouTube and Kick links, sign in from a non-default browser, both connect
