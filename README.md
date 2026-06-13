# chat-rs

A fast, native desktop app for text, voice, (eventually) video, and (eventually) screen sharing with a small group of friends.

## Why chat-rs?

Discord does this well and serves enormous communities. chat-rs aims at something narrower: a lightweight, native experience for close friend groups. Two ideas drive it:

- **Light on resources** — a native app rather than an Electron/web wrapper, so it stays easy on memory and CPU.
- **Focused feature set** — core communication essentials done well.

## Features

- Text messaging — direct messages and server/room-based channels
- Voice chat
- Cross-platform, with an easy install for non-technical users

## Stack

**Client**
- [iced](https://github.com/iced-rs/iced) — Elm-style cross-platform native GUI
- [webrtc](https://github.com/webrtc-rs/webrtc) — voice, video, and screen sharing

**Server**
- [tonic](https://github.com/hyperium/tonic) — gRPC with bidirectional streaming for messaging and call signaling
- SFU for media — server-side fan-out of RTP tracks to room participants
- [SeaORM](https://www.sea-ql.org/SeaORM/) + PostgreSQL — persistence
- JWT-based auth
- [tokio](https://tokio.rs/) — async runtime

The project is organized as a Cargo workspace, with separate crates for the protobuf definitions, domain types, and the conversions between them.
