## ADDED Requirements

### Requirement: Pairing channel
The system SHALL provide a dedicated pairing channel, separate from the control and data channels, used only to bootstrap trust between an unpaired Client and a Server.

#### Scenario: Pairing listener enabled
- **WHEN** pairing is enabled on the Server
- **THEN** the Server SHALL listen for pairing connections on a dedicated pairing port (default control_port − 1)
- **AND** the pairing channel SHALL NOT require a client certificate

#### Scenario: Pairing listener disabled
- **WHEN** pairing is not enabled on the Server
- **THEN** the Server SHALL NOT accept pairing connections
- **AND** the pairing port SHALL be closed

### Requirement: PIN-authenticated pairing
The system SHALL authenticate first-contact pairing using a short PIN via a Password-Authenticated Key Exchange (SPAKE2) so that an active man-in-the-middle without the PIN cannot complete pairing.

#### Scenario: PIN display
- **WHEN** pairing is enabled on the Server
- **THEN** the Server SHALL generate a numeric PIN
- **AND** the Server SHALL display the PIN to the user (GUI) or print it (headless)

#### Scenario: Matching PIN succeeds
- **WHEN** the Client performs the SPAKE2 exchange using a PIN that matches the Server's PIN
- **THEN** both sides SHALL derive a shared session key
- **AND** the pairing SHALL proceed to certificate provisioning

#### Scenario: Mismatched PIN fails
- **WHEN** the Client performs the SPAKE2 exchange using a PIN that does not match the Server's PIN
- **THEN** the session keys SHALL NOT match
- **AND** the Server SHALL reject the pairing with a `PairError`
- **AND** no certificate SHALL be provisioned

#### Scenario: Man-in-the-middle without PIN
- **WHEN** an attacker intercepts the pairing channel without knowing the PIN
- **THEN** the attacker SHALL be unable to derive the shared session key
- **AND** the attacker SHALL be unable to decrypt or forge provisioning payloads

### Requirement: PIN lifecycle and brute-force protection
The system SHALL limit the lifetime and guessability of the pairing PIN.

#### Scenario: PIN expiry
- **WHEN** the PIN has been displayed for longer than its time-to-live
- **THEN** the Server SHALL rotate to a new PIN
- **AND** pairing attempts using the old PIN SHALL fail

#### Scenario: PIN rotation after success
- **WHEN** a pairing completes successfully
- **THEN** the Server SHALL rotate to a new PIN before accepting another pairing

#### Scenario: Failed-attempt lockout
- **WHEN** the number of failed pairing attempts exceeds the configured limit
- **THEN** the Server SHALL temporarily reject further pairing attempts

### Requirement: Certificate provisioning during pairing
After PIN authentication, the system SHALL provision the Client with a CA-signed certificate over the encrypted pairing channel.

#### Scenario: Client requests a certificate
- **WHEN** the SPAKE2 session key is established
- **THEN** the Client SHALL generate a local keypair
- **AND** the Client SHALL send its public key and desired device name to the Server, encrypted under the session key

#### Scenario: Server signs and returns the certificate
- **WHEN** the Server receives a valid provisioning request
- **THEN** the Server SHALL sign a device certificate for the Client using the Server's CA
- **AND** the Server SHALL return the signed Client certificate and the CA certificate, encrypted under the session key

#### Scenario: Client persists provisioned trust material
- **WHEN** the Client receives its signed certificate and the CA certificate
- **THEN** the Client SHALL persist the certificate, its private key, and the CA certificate to the trust store
- **AND** the Client SHALL record the Server under its address in the known-servers list

#### Scenario: Server records the paired client
- **WHEN** the Server signs a certificate for a Client
- **THEN** the Server SHALL record the Client's name and certificate fingerprint in its paired-clients list

### Requirement: Silent reconnection after pairing
The system SHALL allow a previously paired Client to reconnect without a PIN.

#### Scenario: Reconnect to a known server
- **WHEN** the Client connects to a Server address present in its known-servers list
- **THEN** the Client SHALL use the persisted certificate, key, and CA to establish mTLS directly
- **AND** the Client SHALL NOT prompt for a PIN

#### Scenario: Connect to an unknown server triggers pairing
- **WHEN** the Client connects to a Server address not present in its known-servers list
- **AND** no manual certificate paths are configured for that connection
- **THEN** the Client SHALL initiate the pairing flow

### Requirement: Forget and re-pair
The system SHALL allow removing established trust so devices can be re-paired.

#### Scenario: Client forgets a server
- **WHEN** the user removes a Server from the Client's known-servers list
- **THEN** the next connection to that Server SHALL trigger pairing again

#### Scenario: Server revokes a client
- **WHEN** the user removes a Client from the Server's paired-clients list
- **THEN** that Client's certificate fingerprint SHALL no longer be recorded as paired
- **AND** the Client SHALL need to pair again to be re-recorded
