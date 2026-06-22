## ADDED Requirements

### Requirement: Dual-channel network architecture
The system SHALL maintain two separate TCP connections between Server and Client: a control channel and a data channel.

#### Scenario: Control channel establishment
- **WHEN** a Client connects to the Server
- **THEN** the system SHALL establish the control channel on port 9876 (default)
- **AND** the control channel SHALL carry mouse events, keyboard events, boundary messages, heartbeats, and screen configuration

#### Scenario: Data channel establishment
- **WHEN** a Client connects to the Server
- **THEN** the system SHALL establish the data channel on port 9877 (default)
- **AND** the data channel SHALL carry clipboard data (text and image chunks)

### Requirement: TLS encryption
All network communication SHALL be encrypted using TLS 1.3 via rustls.

#### Scenario: TLS handshake
- **WHEN** a Client connects to the Server
- **THEN** the connection SHALL be upgraded to TLS before any application data is exchanged
- **AND** the TLS handshake SHALL use rustls with a self-signed CA certificate

#### Scenario: Data in transit protection
- **WHEN** mouse events, keyboard events, or clipboard data are transmitted
- **THEN** all data SHALL be encrypted within the TLS tunnel

### Requirement: Mutual TLS authentication
The system SHALL use mTLS where both Server and Client present certificates signed by the same CA.

#### Scenario: Client authentication
- **WHEN** a Client connects to the Server
- **THEN** the Server SHALL verify the Client's certificate against the trusted CA
- **AND** connections with invalid or missing client certificates SHALL be rejected

#### Scenario: Server authentication
- **WHEN** a Client connects to the Server
- **THEN** the Client SHALL verify the Server's certificate against the trusted CA
- **AND** connections with invalid server certificates SHALL be rejected by the Client

### Requirement: Connection handshake
After TLS establishment, the system SHALL perform an application-level handshake to exchange device information.

#### Scenario: Successful handshake
- **WHEN** TLS is established between Server and Client
- **THEN** the Client SHALL send a `Handshake` message containing protocol version and device name
- **AND** the Server SHALL respond with a `ScreenConfig` message containing the Server's screen dimensions
- **AND** the connection SHALL be marked as active

#### Scenario: Version mismatch
- **WHEN** the Client's protocol version does not match the Server's
- **THEN** the Server SHALL reject the connection with an error message

### Requirement: Heartbeat and connection monitoring
The system SHALL use heartbeat messages to detect connection health.

#### Scenario: Heartbeat sending
- **WHEN** a connection is active
- **THEN** both Server and Client SHALL send `Heartbeat` messages every 5 seconds

#### Scenario: Connection timeout
- **WHEN** no message is received from the peer for 15 seconds
- **THEN** the connection SHALL be declared dead
- **AND** the system SHALL attempt automatic reconnection with exponential backoff (1s, 2s, 4s, max 30s)

### Requirement: Message framing protocol
All messages SHALL use a binary frame format: `[Magic: 2 bytes (0x5353)][Type: 1 byte][Length: 4 bytes (LE)][Payload: variable]`.

#### Scenario: Frame parsing
- **WHEN** data is received on a channel
- **THEN** the system SHALL parse the frame header first
- **AND** buffer until `Length` bytes of payload are available
- **AND** deserialize the payload using bincode according to the message type

#### Scenario: Invalid frame handling
- **WHEN** a frame with invalid magic bytes or unknown message type is received
- **THEN** the system SHALL log an error and close the connection

### Requirement: Event priority on control channel
The control channel SHALL prioritize input events (mouse/keyboard) over other control messages to minimize latency.

#### Scenario: Input event priority
- **WHEN** a mouse move event and a heartbeat are both queued
- **THEN** the mouse move event SHALL be sent before the heartbeat
