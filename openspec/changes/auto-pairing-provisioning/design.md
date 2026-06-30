## Context

SuperShare uses mTLS (rustls) for all Server↔Client traffic: both sides present certs signed by a shared CA. Today the user must manually run `--gen-cert` to create a CA, sign a device cert per machine, copy three PEM files to each host, and edit cert/key/ca paths in `config.toml` (`ServerConfig`/`ClientConfig` in `crates/ss-core/src/config.rs`). The TLS verify path is in `crates/ss-network/src/tls.rs` (`build_server_config`/`build_client_config`), cert generation in `src/certgen.rs`.

The goal is to keep the proven mTLS data path completely intact, but replace the manual cert-distribution step with an automated, PIN-authenticated pairing exchange. The user-confirmed decisions are: **Server is its own CA** (auto-generated, signs client certs on pairing) and **PAKE PIN** (SPAKE2-authenticated pairing channel so a LAN MITM cannot intercept the first contact).

Constraints from the existing codebase:
- Tokio async networking; GUI on main thread, runtime on a background thread; `std::sync::RwLock` for `SharedAppState`, `AppCommand` mpsc for UI→backend.
- Frame format `[Magic:2][Type:1][Len:4][Payload]` with bincode; `Message` enum + `MessageType` u8 tags in `protocol.rs`.
- Config persists to `dirs::config_dir()/supershare/config.toml`.

## Goals / Non-Goals

**Goals:**
- Client connects given only the Server IP; trust is bootstrapped automatically.
- A short PIN (Server-displayed, Client-entered) authenticates first contact against an active MITM.
- After pairing, reconnects are silent (no PIN) using persisted provisioned certs.
- Reuse the existing mTLS connect/verify code unchanged once certs exist.
- Server needs no manual `gen-cert` for normal use.

**Non-Goals:**
- Discovery/auto-detect of servers (mDNS/Bonjour) — IP is still entered manually.
- Replacing or rewriting the mTLS data path or the dual-channel architecture.
- Multi-CA / external PKI integration; the Server's CA is the sole trust root for paired devices.
- Cross-internet/NAT-traversal pairing — LAN-oriented.

## Decisions

### D1: Dedicated pairing port + protocol, separate from control/data
A new **pairing listener** (default control_port − 1, e.g. 9875) accepts plain-TLS-less (or anonymous-TLS) connections used only for the SPAKE2 exchange and cert provisioning. Rationale: the control/data ports require client certs the unpaired client doesn't have yet; mixing pairing into those listeners would force loosening their mTLS verifier. A separate port keeps the mTLS verifier strict and the pairing surface small and explicit.
- *Alternative considered*: reuse control port with a pre-handshake "pairing mode" before requiring client cert. Rejected — complicates the rustls verifier (would need a switchable client-cert-optional acceptor) and risks weakening the authenticated path.

### D2: SPAKE2 over a TCP channel; PIN is the shared low-entropy password
Pairing runs SPAKE2 (e.g. the `spake2` crate) with the PIN as the password. Both sides derive a shared session key only if the PINs match; a MITM who doesn't know the PIN cannot derive the key and cannot complete the exchange. All provisioning payloads (CSR/cert/CA) are then sent encrypted+authenticated under a key derived from the SPAKE2 output (an AEAD such as ChaCha20-Poly1305 keyed by HKDF of the session key).
- *Alternative considered*: TLS-PSK. Rejected — rustls PSK support is limited and a low-entropy PSK over plain TLS is vulnerable to offline dictionary attack; SPAKE2 is purpose-built for low-entropy secrets.

### D3: PIN generation and lifecycle
Server generates a fresh numeric PIN (6 digits) when pairing is enabled, displays it in the GUI / prints it in headless mode, and rotates it on a timeout (e.g. 3 minutes) and after each successful pairing. Rationale: 6 digits is usable to type; short TTL + online-only verification (SPAKE2 gives no offline guessing) keeps brute force infeasible. Server rate-limits/locks pairing after N failed attempts.

### D4: Server-as-CA, client cert provisioned during pairing
On first run the Server auto-generates `ca.pem`/`ca-key.pem` and a server device cert (reusing `certgen.rs` functions) into its config dir if absent. During pairing the Client generates a keypair locally, sends a CSR-equivalent (its public key + desired device name) over the SPAKE2-encrypted channel; the Server signs a device cert (SANs include the client name) and returns `{client_cert, ca_cert}`. The Client persists `client_cert`, its own `client_key`, and `ca_cert`. The Server records the paired client (name + cert fingerprint).
- *Alternative considered*: client self-signs and both pin fingerprints (no CA). Rejected per user decision — would require rewriting the CA-based verifier in `tls.rs`.

### D5: Trust store persistence
Extend `AppConfig`:
- Server: auto CA/cert paths (defaulted into config dir) + `paired_clients: Vec<PairedClient { name, cert_fingerprint, paired_at }>`.
- Client: `known_servers: Vec<KnownServer { address, cert_path, key_path, ca_path, server_fingerprint }>` keyed by address.
Provisioned PEMs are written under the config dir (e.g. `supershare/trust/`). On connect, the Client looks up the server address in `known_servers`; if found it goes straight to mTLS connect, otherwise it triggers pairing. Existing manual cert paths remain honored as an override.

### D6: Connect flow integration
`client::connect` gains a pre-step: resolve trust for the target address. If untrusted → run pairing (prompt PIN via a new `AppCommand`/callback) → persist → then proceed into the existing `connect()` unchanged. Headless client gets a `--pair` path that prompts for the PIN on stdin. The Server runs the pairing listener alongside control/data listeners in `server::start`, gated by a pairing-enabled flag.

### D7: Protocol additions
New `Message` variants + `MessageType` tags for the pairing exchange (e.g. `PairRequest`, `PairSpake { msg }`, `PairConfirm`, `PairResult { client_cert, ca_cert }`, `PairError { reason }`). These travel only on the pairing channel. The PAKE bytes and provisioning payload are carried as opaque `Vec<u8>` fields to keep `ss-core` free of crypto deps (crypto lives in `ss-network`).

## Risks / Trade-offs

- **Low-entropy PIN brute force** → SPAKE2 makes guessing online-only (one guess per exchange); Server rate-limits and rotates/expires the PIN, and locks after N failures.
- **New crypto dependencies (`spake2`, an AEAD, HKDF)** → keep them confined to `ss-network`; pin versions; cover the exchange with unit tests including mismatched-PIN and tampered-message cases.
- **Pairing port adds attack surface** → it's only open when pairing is explicitly enabled; it exposes no capability without a valid SPAKE2 completion; close it when not pairing.
- **Server CA key compromise = full trust compromise** → store CA key with restrictive file permissions in the config dir; document that re-keying invalidates all paired clients.
- **Address-keyed client trust store** → IP changes (DHCP) cause a re-pair; mitigate by also storing/serving on server fingerprint so a known fingerprint at a new IP can re-bind without a new PIN (optional follow-up).
- **Backward compatibility** → existing configs with explicit cert paths still work; pairing is additive and only triggers when no trust exists for the target.

## Migration Plan

1. Ship pairing as additive; existing manual-cert configs are untouched and take precedence when present.
2. First run with no CA auto-generates one; no user action needed.
3. `gen-cert` remains for advanced/manual setups (documented as optional).
4. Rollback: disable the pairing listener (feature flag/config); manual cert flow is unaffected.

## Open Questions

- PIN length/format: 6 numeric digits vs. shorter word-based code — default to 6 digits unless usability testing says otherwise.
- Should the Client also display a fingerprint for the user to optionally cross-check, as defense-in-depth on top of SPAKE2? (Lean: no, SPAKE2 suffices; revisit if requested.)
- Server fingerprint re-bind on IP change (D-store note) — include now or defer to a follow-up change?
