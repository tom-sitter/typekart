# Milestone 7 Plan: Internet Play

## Goal

Let players join races from outside the host's local network without requiring normal users to understand router configuration.

Milestone 7 should preserve the current server-authoritative game model. The host or a hosted service should continue to own race state, item resolution, bonus claims, countdowns, and results. Clients should continue to send input and render authoritative snapshots.

## Current Starting Point

Implemented before Milestone 7:

- Local-network `host` and `join` commands over TCP.
- Newline-delimited JSON protocol messages.
- Server-authoritative lobby, countdown, typing, bonuses, items, results, and rematch flow.
- Network AI racers for easier multiplayer validation.
- Debug logs for host and joiner sessions.
- Mod metadata and stable hashes in lobby and race snapshots.
- Phase-aware terminal controls.

Important current constraint:

- The transport is coupled to `TcpListener` and `TcpStream` in `src/net/server.rs` and `src/net/client.rs`.

## Internet Play Options

| Option | Description | Pros | Cons | Fit |
| --- | --- | --- | --- | --- |
| Direct port forwarding | Host exposes the existing TCP port publicly. | Smallest code change. Keeps current host model. | Poor user experience. Fails behind many NAT/firewall setups. Requires router knowledge. | Useful only as an expert/dev mode. |
| NAT traversal / hole punching | Peers coordinate through a rendezvous server, then connect directly. | Can avoid relay bandwidth. Lower latency when it works. | Hard to implement reliably. UDP-oriented. More edge cases than the game needs right now. | Defer. |
| Public relay | Host and joiners connect outbound to a relay; relay forwards messages for a room. | Works behind typical NAT. Preserves host-authoritative game state. Easier than full hosted authoritative server. | Requires hosted infrastructure. Relay must handle room lifecycle and backpressure. Adds latency. | Best first internet-play target. |
| Hosted authoritative server | A public server owns all race state. Clients connect directly to it. | Best reliability and fairness. No host upload bottleneck. Easier to supervise rooms centrally. | Most operational work. Requires moving host state into a deployable server process. | Good future target, more than Milestone 7 needs. |
| WebSocket relay | Public relay uses WebSockets instead of raw TCP. | Easy to deploy on common hosts. Firewall-friendly. Browser-compatible later. Message framing is built in. | Requires adapting current TCP newline protocol to WebSocket frames. | Recommended transport for the first relay. |

## Recommendation

Build a **WebSocket relay** first.

This gives the best balance between user experience and engineering scope:

- Players only need a room code or join URL/token.
- The existing host remains authoritative.
- The relay does not need to understand most game rules.
- Clients make outbound connections, so home NAT is not a blocker.
- The current JSON protocol can be carried inside WebSocket text frames with limited reshaping.

Direct port forwarding can remain available through the existing `host --bind 0.0.0.0:4000` and `join --server HOST:4000` commands for technical users, but it should not be the primary internet-play path.

## Target User Flow

Host:

```sh
cargo run -- host-online --name tom
```

Expected behavior:

- Client connects to the public relay.
- Relay creates a room.
- Host sees a room code, for example `rocket-salad-tiger`.
- Host terminal behaves like the current host UI.

Joiner:

```sh
cargo run -- join-online --name alex --room rocket-salad-tiger
```

Expected behavior:

- Joiner connects to the relay.
- Relay routes the join request to the host.
- Host accepts or rejects using the same capacity/name/version/mod compatibility logic as local networking.
- Joiner renders the same network UI as local `join`.

## Recommended Architecture

### Roles

Host client:

- Owns authoritative `HostState`.
- Connects outbound to the relay.
- Receives client messages from relay frames.
- Sends lobby/race/results snapshots back through relay frames.

Join client:

- Connects outbound to the relay.
- Sends `ClientMessage` frames.
- Receives `ServerMessage` frames.
- Reuses the existing network renderer and input handling.

Relay:

- Owns rooms and participant connections.
- Assigns room codes.
- Routes frames between one host and multiple joiners.
- Does not resolve race logic.
- Enforces basic room limits and idle timeouts.

### Message Routing

Current local TCP shape:

```text
join client -> host TCP server: ClientMessage
host TCP server -> join client: ServerMessage
```

Relay shape:

```text
host <-> relay <-> joiner
```

Relay envelope:

```json
{
  "type": "client_to_host",
  "room": "rocket-salad-tiger",
  "player_id": 2,
  "message": { "type": "key_input", "sequence": 42, "key": { "type": "char", "value": "a" } }
}
```

Host-to-client envelope:

```json
{
  "type": "host_to_client",
  "room": "rocket-salad-tiger",
  "player_id": 2,
  "message": { "type": "race_snapshot", "snapshot": {} }
}
```

The inner `ClientMessage` and `ServerMessage` should remain the same where possible.

## Transport Refactor

Before implementing the relay, isolate the game protocol from raw TCP.

Suggested internal interface:

```rust
trait ClientTransport {
    fn send(&mut self, message: &ClientMessage) -> Result<()>;
    fn recv(&mut self) -> Result<Option<ServerMessage>>;
}

trait HostConnection {
    fn player_id(&self) -> PlayerId;
    fn send(&mut self, message: &ServerMessage) -> Result<()>;
    fn recv(&mut self) -> Result<Option<ClientMessage>>;
}
```

The exact trait shape can change, but the goal is stable:

- `src/net/protocol.rs` owns message structs and JSON encode/decode.
- TCP transport handles newline-delimited JSON.
- WebSocket transport handles one JSON message per text frame.
- Host game logic should not know whether a message came from TCP or relay.

## Relay Responsibilities

Minimum viable relay:

- Accept WebSocket connections.
- Support `create_room` from a host.
- Support `join_room` from joiners.
- Route messages by room and player id.
- Close rooms when the host disconnects.
- Reject joins when a room does not exist or is full.
- Apply idle room timeout.
- Limit message size.
- Log room lifecycle and disconnect causes.

The relay should not:

- Roll items.
- Validate typing.
- Own race state.
- Store mod packs.
- Persist accounts or stats.

## Security And Abuse Boundaries

Milestone 7 does not need accounts, but it does need basic safety:

- Room codes should be random enough to avoid easy guessing.
- Relay should reject oversized frames.
- Relay should cap players per room.
- Relay should reject clients whose TypeKart version differs from the room host.
- Relay should cap rooms per process.
- Relay should time out idle rooms.
- Relay should avoid logging full player input streams unless debug logging is explicitly enabled.

Encryption should come from `wss://` when deployed. Local development can use `ws://`.

## Latency Expectations

Typing games are sensitive to perceived latency, but TypeKart already uses server-authoritative snapshots.

Initial targets:

- Input command delivery: under 100 ms typical.
- Snapshot rate: keep current 20 Hz if relay bandwidth is acceptable.
- Graceful degradation: if relay traffic is too high, reduce snapshot rate before changing protocol shape.

Potential later improvements:

- Send snapshots only when state changes during lobby/results.
- Delta-compress race snapshots.
- Use binary serialization after JSON has proven insufficient.

## CLI Shape

Keep existing LAN commands:

```sh
cargo run -- host --name tom --bind 0.0.0.0:4000
cargo run -- join --name alex --server 192.168.1.20:4000
```

Add internet commands:

```sh
cargo run -- host-online --name tom --relay wss://relay.typekart.example
cargo run -- join-online --name alex --relay wss://relay.typekart.example --room rocket-salad-tiger
```

Useful development relay command:

```sh
cargo run -- relay --bind 127.0.0.1:8080
```

This keeps local TCP testing available while adding a dedicated path for internet play.

## Implementation Slices

### Slice 1: Transport Boundary

Status: implemented for newline-delimited JSON transport helpers.

Deliverables:

- Introduce transport abstractions or equivalent channel boundaries.
- Keep existing TCP host/join behavior unchanged.
- Add tests proving TCP protocol round trips still work.

Implementation notes:

- `src/net/transport.rs` owns JSON-line read/write helpers for `ClientMessage` and `ServerMessage`.
- Existing TCP host/join code now uses those helpers instead of encoding/decoding directly at each call site.
- This keeps the current LAN behavior unchanged while giving WebSocket transport a parallel framing surface.

Validation:

- Existing LAN tests pass.
- Manual local host/join commands still work.

### Slice 2: Relay Protocol

Status: implemented for room code and envelope data types.

Deliverables:

- Define relay envelope messages.
- Add room code generation.
- Add host create-room and joiner join-room flows.
- Add protocol tests for relay envelopes.

Implementation notes:

- `src/net/relay.rs` defines `RoomCode`, `RelayClientMessage`, and `RelayServerMessage`.
- Room codes use three easy-to-say words, such as `rocket-salad-tiger`.
- Room-code parsing is case-insensitive and accepts common separators such as spaces or hyphens.
- Relay envelopes wrap existing `ClientMessage` and `ServerMessage` values so the relay can route messages without owning race rules.

Validation:

- Envelope serialization round trips.
- Room code normalization and validation are covered.
- Invalid room joins, participant cleanup, and message-size limits are handled by the relay slice.

### Slice 3: Local Development Relay

Status: implemented for local relay process and in-memory room routing.

Deliverables:

- Add `relay` command.
- Implement a local WebSocket relay.
- Route opaque inner protocol messages between host and joiners.

Implementation notes:

- `src/net/relay_server.rs` runs a local WebSocket relay with room-scoped host and participant connections.
- `cargo run -- relay --bind 127.0.0.1:8080` starts the development relay.
- The relay creates room codes, forwards join requests to the host, routes participant input to the host, broadcasts host snapshots to participants, and closes rooms when the host disconnects.
- The relay can now be reached by the `host-online` and `join-online` loopback adapters from Slice 4.

Validation:

- `relay --help` displays the development relay command.
- Unit tests cover room creation, join forwarding, client-to-host routing, host broadcasts, and host disconnect cleanup.
- Same-machine host/join through the relay is available through Slice 4.

### Slice 4: Online Host And Join Commands

Status: implemented for local `ws://` relay usage through loopback adapters.

Deliverables:

- Add `host-online`.
- Add `join-online`.
- Reuse current network UI.
- Add clearer connection and room rejection errors.

Implementation notes:

- `src/net/online.rs` bridges the existing TCP game protocol over the WebSocket relay.
- `host-online` starts the normal authoritative host on loopback, starts an online bridge, creates a relay room, then opens the usual host UI.
- `join-online` starts a loopback proxy, connects that proxy to the relay room, then opens the usual join UI against the proxy.
- This intentionally avoids forking the game rules or renderer while proving the relay path end to end.
- Online adapters support plain `ws://` and TLS-backed `wss://` relay URLs.
- The Rust relay binary remains a plain WebSocket service; public TLS should terminate at a reverse proxy.

Validation:

- `host-online --help` and `join-online --help` expose the new commands.
- Existing TCP and game tests pass.
- Automated same-machine relay smoke test verifies a joiner receives the real host welcome through relay-backed adapters.
- Manual same-machine relay smoke test.
- Two-machine same-LAN relay test.
- Public relay test when hosting infrastructure exists.

### Slice 5: Deployment Notes

Status: implemented for relay hardening, reverse-proxy deployment guidance, and Docker deployment.

Deliverables:

- Document relay deployment options.
- Document expected ports and `ws://` vs `wss://`.
- Document operational limits and logging.
- Add an easy cloud deployment path.

Implementation notes:

- `docs/relay-deployment.md` documents local, LAN, and public reverse-proxy relay deployment.
- Public clients can use `wss://` relay URLs.
- The relay process should be run as a stateful in-memory service behind a supervisor.
- TLS termination is intentionally delegated to a reverse proxy such as Caddy.
- Relay runtime hardening includes configurable room limits, participants-per-room limits, message size limits, and idle room cleanup.
- `Dockerfile` builds a small relay runtime image for cloud infrastructure.

Validation:

- A clean checkout can run relay, host-online, and join-online using documented commands.
- Relay hardening unit tests cover room limits, joiner limits, and idle cleanup.
- Docker build should be verified before public deployment.
- Public deployment validation still requires a real domain and TLS reverse proxy.

## Testing Strategy

Unit tests:

- Relay envelope encode/decode.
- Room code validation.
- Room lifecycle transitions.
- Join rejection reasons.
- Message size limits.

Integration tests:

- Host creates a room.
- Joiner connects to room.
- Relay routes `ClientMessage` to host.
- Relay routes `ServerMessage` to joiner.
- Host disconnect closes room.
- Joiner disconnect notifies host.

Manual tests:

- Local relay on one machine.
- Relay on one machine, host/join from another.
- Public relay over `wss://`.
- High key-rate typing with multiple clients.

## Non-Goals

- Matchmaking queues.
- Accounts.
- Friend lists.
- Chat.
- Spectators.
- Reconnect/resume.
- Browser client.
- Custom relay discovery service.
- Full hosted authoritative game server.

## Open Questions

- Should the first relay be hosted as a long-running Rust binary, or should we target a managed WebSocket platform?
- Should room codes be relay-generated only, or can hosts request readable custom room names?
- Should mod hash mismatches reject joins before the race or only warn in the lobby?
- Do we need TLS support in the Rust relay itself, or should TLS terminate at a reverse proxy?
- What public hosting target do we want to support first?

## Recommendation For First Slice

Start with Slice 1: transport boundary.

Reasoning:

- It reduces risk before introducing hosted infrastructure.
- It keeps current LAN behavior intact.
- It makes TCP and WebSocket implementations peers instead of creating a one-off online path.
- It will expose exactly how much of `src/net/server.rs` and `src/net/client.rs` is tied to raw `TcpStream`.
