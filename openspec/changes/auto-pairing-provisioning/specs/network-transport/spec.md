## MODIFIED Requirements

### Requirement: TLS encryption
All network communication SHALL be encrypted using TLS 1.3 via rustls. The Server SHALL act as its own Certificate Authority, auto-generating a CA and Server certificate on first run when none is configured.

#### Scenario: TLS handshake
- **WHEN** a Client connects to the Server
- **THEN** the connection SHALL be upgraded to TLS before any application data is exchanged
- **AND** the TLS handshake SHALL use rustls with the Server's CA certificate

#### Scenario: Data in transit protection
- **WHEN** mouse events, keyboard events, or clipboard data are transmitted
- **THEN** all data SHALL be encrypted within the TLS tunnel

#### Scenario: Auto-generated CA on first run
- **WHEN** the Server starts and no CA certificate/key is configured or present in the config directory
- **THEN** the Server SHALL generate a CA certificate, a CA key, and a Server device certificate signed by that CA
- **AND** the Server SHALL persist them to the config directory for reuse on subsequent runs

### Requirement: Mutual TLS authentication
The system SHALL use mTLS where both Server and Client present certificates signed by the Server's CA. Client certificates SHALL be obtained through the pairing flow rather than required to be pre-shared manually.

#### Scenario: Client authentication
- **WHEN** a Client connects to the Server on the control or data channel
- **THEN** the Server SHALL verify the Client's certificate against the trusted CA
- **AND** connections with invalid or missing client certificates SHALL be rejected

#### Scenario: Server authentication
- **WHEN** a Client connects to the Server
- **THEN** the Client SHALL verify the Server's certificate against the trusted CA
- **AND** connections with invalid server certificates SHALL be rejected by the Client

#### Scenario: Certificates obtained via pairing
- **WHEN** a Client has no certificate trusted by the Server for the target address
- **THEN** the Client SHALL obtain a CA-signed certificate through the pairing flow before establishing the mTLS control and data channels

#### Scenario: Manual certificates still honored
- **WHEN** explicit certificate, key, and CA paths are configured for a connection
- **THEN** the system SHALL use those certificates directly without triggering pairing
