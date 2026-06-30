## 1. Dependencies & crypto primitives

- [x] 1.1 Add `spake2` (PAKE), an AEAD (`chacha20poly1305`), and `hkdf`/`sha2` to `ss-network` Cargo.toml
- [x] 1.2 Add a `pairing::crypto` module in `ss-network` wrapping SPAKE2 start/finish and HKDF→AEAD key derivation
- [x] 1.3 Unit-test the crypto module: matching PINs derive equal keys; mismatched PINs do not; AEAD round-trip and tamper-detection

## 2. Protocol additions (ss-core)

- [x] 2.1 Add pairing `Message` variants (`PairRequest`, `PairSpake`, `PairConfirm`, `PairResult`, `PairError`) with opaque `Vec<u8>` payloads
- [x] 2.2 Add matching `MessageType` u8 tags and update `msg_type()` and `TryFrom<u8>`
- [x] 2.3 Add a unit test serializing/deserializing each new variant through the frame format

## 3. Server-as-CA auto-generation (certgen + config)

- [x] 3.1 Add `ensure_server_ca()` to `certgen.rs`: generate CA + Server device cert into the config dir if absent, restrictive perms on the CA key
- [x] 3.2 Add `sign_client_cert(public_key, device_name)` to `certgen.rs` that signs a client cert with the in-memory CA (no temp files)
- [x] 3.3 Wire server startup to call `ensure_server_ca()` when no cert paths are configured

## 4. Trust store (ss-core config)

- [x] 4.1 Add `PairedClient { name, cert_fingerprint, paired_at }` and `paired_clients: Vec<_>` to `ServerConfig`; make cert paths optional with sensible config-dir defaults
- [x] 4.2 Add `KnownServer { address, cert_path, key_path, ca_path, server_fingerprint }` and `known_servers: Vec<_>` to `ClientConfig`
- [x] 4.3 Add helpers to look up / insert / remove a known server by address and a paired client by fingerprint
- [x] 4.4 Define the on-disk trust layout (`supershare/trust/`) and a helper to write/read provisioned PEMs
- [x] 4.5 Unit-test trust-store round-trip (save → load) and lookup/forget semantics

## 5. Pairing protocol — Server side (ss-network)

- [x] 5.1 Add a `pairing` module with a pairing listener bound to the pairing port (default control_port − 1), gated by a pairing-enabled flag
- [x] 5.2 Implement PIN generation, display hook, TTL rotation, rotate-after-success, and failed-attempt lockout
- [x] 5.3 Implement the server pairing handshake: SPAKE2 exchange → on success decrypt provisioning request → sign client cert → return `{client_cert, ca_cert}` encrypted; record `PairedClient`
- [x] 5.4 Reject mismatched PIN / tampered payloads with `PairError`; ensure no cert is issued on failure
- [x] 5.5 Start the pairing listener from `server::start` alongside control/data listeners; shut it down with the server

## 6. Pairing protocol — Client side (ss-network)

- [x] 6.1 Implement `pair_with_server(address, pin)`: connect to the pairing port, run SPAKE2, generate local keypair, send provisioning request, receive + persist cert/key/CA, record `KnownServer`
- [x] 6.2 Add trust resolution to `client::connect`: if a known server (or explicit cert paths) exists, connect via existing mTLS path; otherwise signal that pairing is required
- [x] 6.3 Ensure post-pairing connect reuses the existing unchanged `connect()` mTLS flow
- [x] 6.4 Surface pairing errors (wrong/expired PIN, lockout) as typed errors for the UI/CLI

## 7. GUI integration (ss-ui)

- [x] 7.1 Add `AppCommand` variants for `EnablePairing`/`DisablePairing` (server) and `PairAndConnect { address, pin }` / pairing-required signaling (client)
- [x] 7.2 Add `SharedAppState` fields for current pairing PIN (server) and pairing status/prompt (client)
- [x] 7.3 Server UI: pairing-enable toggle + live PIN display
- [x] 7.4 Client UI: IP-only connect; PIN prompt dialog when pairing is required; error/retry feedback
- [x] 7.5 Move cert path inputs into an "Advanced" section, optional and overriding pairing

## 8. CLI / headless integration (src/main.rs)

- [x] 8.1 Make `--cert/--key/--ca` optional for both server and client modes
- [x] 8.2 Add `--pair` to client mode: when no trust exists, prompt for PIN on stdin, pair, then connect
- [x] 8.3 Headless client auto-uses persisted trust for a known server without prompting
- [x] 8.4 Keep `gen-cert` working and document it as optional/advanced

## 9. Testing & docs

- [x] 9.1 End-to-end test: fresh client pairs with PIN → cert provisioned → control+data channels establish
- [x] 9.2 End-to-end test: paired client reconnects silently (no PIN); forget-server forces re-pair
- [x] 9.3 Negative tests: wrong PIN, expired PIN, lockout, MITM cannot complete pairing
- [x] 9.4 Update README/CLAUDE.md and docs to describe pairing as the default flow and `gen-cert` as advanced
