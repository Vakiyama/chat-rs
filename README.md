# chat-rs

## PRD

Key problem: Fast and secure text, voice, video and screen sharing based
communication with a close set of friends.

Motivations: Discord is the leader in this space, but has some key issues:

- Client performance: Electron/web based apps are memory hungry and not
  necessary for discord to exist.
- Bloated feature set: Discord is desperately looking for monetization
  opportunities, bloating the experience.

### Requirements

Text based instant messaging, supporting direct messaging and server based
messaging.

Peer to peer direct and room based voice chat.

Cross platform, easy install for non technical users.

## Potential Stack

### Frontend

iced for ELM style cross platform native GUI

webrtc-rs for webRTC (voice calls, streaming)

reqwest - http client

### Backend Frontend

axum - http server + tokie-tungstenite for web sockets

libsql - db stuff

tokio - for the async runtime
