## ADDED Requirements

### Requirement: Text clipboard synchronization
The system SHALL synchronize text clipboard content across all connected machines.

#### Scenario: Text copied on Server
- **WHEN** the user copies text on the Server machine
- **THEN** the system SHALL detect the clipboard change within 500ms
- **AND** the system SHALL send the text content to all connected Clients via the data channel
- **AND** each Client SHALL write the text to its local clipboard

#### Scenario: Text copied on Client
- **WHEN** the user copies text on a Client machine
- **THEN** the Client SHALL detect the clipboard change within 500ms
- **AND** the Client SHALL send the text content to the Server via the data channel
- **AND** the Server SHALL broadcast the text to all other connected Clients

### Requirement: Image clipboard synchronization
The system SHALL synchronize image clipboard content across all connected machines.

#### Scenario: Image copied on Server
- **WHEN** the user copies an image (e.g., screenshot) on the Server machine
- **THEN** the system SHALL read the image as RGBA pixel data via arboard
- **AND** the system SHALL compute a blake3 hash of the pixel data
- **AND** if the hash differs from the previous clipboard content, the system SHALL compress the data with zstd (level 3)
- **AND** the system SHALL send `ClipboardBegin` + `ClipboardChunk` (64KB each) + `ClipboardEnd` messages via the data channel
- **AND** each Client SHALL reassemble, decompress, and write the image to its local clipboard

#### Scenario: Large image transfer
- **WHEN** an image larger than 10 MB (compressed) is copied
- **THEN** the system SHALL reject the transfer and log a warning
- **AND** the clipboard on remote machines SHALL remain unchanged

### Requirement: Clipboard change detection
The system SHALL detect clipboard changes by polling the clipboard content at regular intervals.

#### Scenario: Polling interval
- **WHEN** the system is running
- **THEN** the system SHALL poll the clipboard every 200ms
- **AND** the system SHALL compare the current content hash with the previous hash to detect changes

#### Scenario: No change detected
- **WHEN** the clipboard content has not changed since the last poll
- **THEN** the system SHALL not send any clipboard messages

### Requirement: Clipboard loop prevention
The system SHALL prevent infinite clipboard synchronization loops when content is received from a remote machine and written to the local clipboard.

#### Scenario: Suppression after remote write
- **WHEN** the system writes clipboard content received from a remote machine
- **THEN** the system SHALL set a suppression flag for 1 second
- **AND** during this period, local clipboard changes SHALL be ignored (not forwarded)

#### Scenario: Hash-based deduplication
- **WHEN** the system detects a local clipboard change
- **AND** the content hash matches the last received remote clipboard hash
- **THEN** the system SHALL NOT forward the change (it originated from the network)

### Requirement: Clipboard content format support
The system SHALL support text (UTF-8) and image (RGBA pixel data) clipboard formats.

#### Scenario: Text format
- **WHEN** clipboard content is text
- **THEN** the system SHALL transmit it as UTF-8 encoded bytes
- **AND** the message type SHALL be `ClipboardData` with format `0x01` (text)

#### Scenario: Image format
- **WHEN** clipboard content is an image
- **THEN** the system SHALL transmit the raw RGBA pixel data after zstd compression
- **AND** the message type SHALL be `ClipboardBegin` with format `0x02` (image)
- **AND** the `ClipboardBegin` message SHALL include width, height, and total compressed size
