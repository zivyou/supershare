## Why

Today connecting a Client to a Server requires manually running `--gen-cert` to build a CA, generating a device cert signed by that CA, and copying three PEM files (`ca.pem`, `<device>.pem`, `<device>-key.pem`) onto each machine before editing config paths by hand. This is error-prone and a major barrier to first use. We want the Client to only need the Server's IP, with trust established automatically through a short one-time PIN.

## What Changes

- Add a **pairing handshake**: on first connection a Client and Server negotiate trust over an unauthenticated channel, authenticated by a short PIN the Server displays and the user types into the Client. The PIN is verified with a PAKE (SPAKE2), so even an active man-in-the-middle on the LAN cannot intercept pairing.
- The **Server acts as its own CA**: on first run it auto-generates a CA + a Server device cert (no manual `gen-cert`). During pairing it signs a device certificate for the Client and returns it plus the CA cert. The existing mTLS verify path is reused unchanged for all subsequent connections.
- Add a **trust store**: paired devices persist their provisioned cert/key/CA so re-connecting needs no PIN. The Server remembers paired clients; the Client remembers known servers keyed by IP.
- **GUI/CLI flow simplified**: Client UI requires only an IP (and PIN when prompted). Manual cert-path fields become optional/advanced. Server UI shows a "Pairing PIN" and a pairing-enable toggle.
- **`gen-cert` retained but de-emphasized** for advanced/manual deployments; it is no longer required for normal use. **BREAKING**: normal connection no longer expects user-supplied cert paths — config grows a pairing trust store and cert paths become optional.

## Capabilities

### New Capabilities
- `device-pairing`: First-contact pairing protocol (PIN display + entry, PAKE-authenticated channel, cert signing/provisioning, trust persistence, re-pair/forget).

### Modified Capabilities
- `network-transport`: mTLS bootstrap changes — Server is its own auto-generated CA; certs are provisioned via pairing rather than pre-shared files; defines how pairing transitions into the existing mTLS connection.
- `config-ui`: Client connection requires only an IP; pairing PIN entry/display in GUI; cert paths become optional/advanced; CLI gains pairing-oriented flags.

## Impact

- **Code**: `src/certgen.rs` (CA auto-gen + sign-on-demand), `crates/ss-network` (new pairing module, server/client connect flow, trust store), `crates/ss-core` (new pairing protocol `Message` variants + `AppConfig` trust store fields), `crates/ss-ui` (PIN UI, IP-only connect, new `AppCommand` variants), `src/main.rs` (pairing CLI flags, headless pairing flow).
- **Dependencies**: add a PAKE crate (e.g. `spake2`) for PIN-authenticated key exchange.
- **Config/data**: new on-disk trust store (provisioned certs + paired-device records) under the existing config dir.
- **Security**: pairing is the new trust root; PIN entropy, PIN rotation/expiry, and replay protection must be specified carefully.
- **Backward compatibility**: existing manually-generated certs continue to work via optional cert paths; pairing is additive.
