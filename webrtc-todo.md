# WebRTC Implementation TODO

## Server

- [ ] **Multiple rooms** — Currently only one global room. Room struct exists; follow the same approach as text message signaling to handle multiple rooms.
- [ ] **Glare protection** — Server initiated renegotiations can collide with each other or an initial join on the same PC. The designed fix is per-Participant negotiating/needs_renegotiation atomics, with negotiating initialized true until the initial answer is sent (this also closes the answer-vs-offer channel-ordering hazard and the insert→collect double-wire window)
- [ ] **m-line accumulation over churn** — Relays use add_transceiver_from_track(Sendonly) because add_track slot-reuse after remove_track fails with ErrRTPSenderNewTrackHasIncorrectEnvelope (upstream webrtc-rs bug; link the issue if you file it). Consequence: long-lived connections grow one dead m-line per departed peer. Future fix: replace_track-based slot pool, blocked on the upstream bug.
- [ ]   **NAT 1:1 + UDPMux are mutually incompatible in webrtc 0.17** —  (mux conn closes during gathering, suspected IPv6 no-mapping error path). Current deploys don't need NAT 1:1 (Hetzner has the public IP on-interface); document do not combine for future NATed hosts (AWS etc.), plus consider set_network_types(Udp4) as standard mux config.
- [ ]   **TURN fallback** —  clients on UDP-blocking networks can't connect at all; needs a coturn deployment and client-side TURN credentials when it bites.
- [ ]   **Half-joined peers on error** —   if handle_offer fails after the answer is sent (add_track/reneg), the client believes it joined but is silently broken; tie cleanup of this state into handle_leave.

## Client

- [ ] **Explicit join/leave** — Currently auto-joins global room on client open and leaves on close. Add explicit join/leave controls.
- [ ] **Room presence UI** — Nothing shows which users are in each room.
- [ ] **Separate audio tracks** — All audio channels are mixed into a single mono track with no processing. Add per-peer audio tracks.
- [ ] **Mic pre-processing** — No static removal, echo cancellation, or noise gate on input: intended tool (webrtc-audio-processing bindings for AEC/NS/AGC — AEC being the one that makes speakers usable at all) so the item has a concrete next step.
- [ ] **Jitter buffer** — Input mixer has no jitter buffer for incoming packets on lossy connections, should be handled in per track decode loop.
- [ ] **Packet loss correction (PLC)** — No PLC setup; silence gaps are played back. Implement PLC to fill in lost packets: detect RTP sequence gaps in the per-track decode loop and call decode_float(None, ...) to let Opus synthesize the missing frame.
- [ ] **Generic audio configuration** — Currently assumes and asserts a default input/output device, 48kHz sample rate, f32 sample format. These need to become generic over possible input configurations and handled without assertions.

## App

- [ ] **Domain level signals for room leaving/joining** - Add explicit signaling to allow clients to render/sync their room state with the server.
- [ ]   **No E2E encryption (by design, for now)** - SRTP is hop-by-hop; the SFU can technically access audio.
