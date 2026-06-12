# WebRTC Implementation TODO

## Server

- [ ] **Multiple rooms** — Currently only one global room. Room struct exists; follow the same approach as text message signaling to handle multiple rooms.
- [ ] **Glare protection** — Two clients sending/renegotiating offers within a short window can cause zombie clients with no audio stream or other undefined behavior. Need a locking/queuing mechanism for client-initiated offers.

## Client

- [ ] **Explicit join/leave** — Currently auto-joins global room on client open and leaves on close. Add explicit join/leave controls.
- [ ] **Room presence UI** — Nothing shows which users are in each room.
- [ ] **Separate audio tracks** — All audio channels are mixed into a single mono track with no processing. Add per-peer audio tracks.
- [ ] **Mic pre-processing** — No static removal, echo cancellation, or noise gate on input.
- [ ] **Jitter buffer** — Input mixer has no jitter buffer for incoming packets on lossy connections.
- [ ] **Packet loss correction (PLC)** — No PLC setup; silence gaps are played back. Implement PLC to fill in lost packets.
- [ ] **Generic audio configuration** — Currently assumes and asserts a default input/output device, 48kHz sample rate, f32 sample format. These need to become generic over possible input configurations and handled without assertions.

## App

- [ ] **Domain level signals for room leaving/joining** - Add explicit signaling to allow clients to render/sync their room state with the server.

