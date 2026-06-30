## MODIFIED Requirements

### Requirement: CLI subcommands
The system SHALL support the following CLI options via clap. When no arguments are provided, the system SHALL open the GUI. Certificate paths SHALL be optional; when omitted the system SHALL use pairing-provisioned trust material.

#### Scenario: Default behavior (no arguments)
- **WHEN** the user runs `supershare` with no arguments
- **THEN** the system SHALL open the GUI configuration window

#### Scenario: Start as Server (headless)
- **WHEN** the user runs `supershare --server [--port 9876]`
- **THEN** the system SHALL start in headless Server mode
- **AND** the system SHALL auto-generate a CA and Server certificate if none are configured or present

#### Scenario: Start as Server with explicit certificates
- **WHEN** the user runs `supershare --server [--port 9876] --cert cert.pem --key key.pem --ca ca.pem`
- **THEN** the system SHALL start in headless Server mode using the specified certificates

#### Scenario: Pair a Client (headless)
- **WHEN** the user runs `supershare --client --server 192.168.1.100 --pair`
- **AND** no trust material exists for that Server
- **THEN** the system SHALL prompt for the pairing PIN on stdin
- **AND** the system SHALL complete pairing and persist the provisioned certificate before connecting

#### Scenario: Start as Client to a known Server (headless)
- **WHEN** the user runs `supershare --client --server 192.168.1.100`
- **AND** trust material already exists for that Server
- **THEN** the system SHALL connect using the persisted certificate without prompting for a PIN

#### Scenario: Start as Client with explicit certificates
- **WHEN** the user runs `supershare --client --server 192.168.1.100:9876 --cert cert.pem --key key.pem --ca ca.pem`
- **THEN** the system SHALL connect using the specified certificates without pairing

#### Scenario: Generate certificates
- **WHEN** the user runs `supershare gen-cert [--output ./certs]`
- **THEN** the system SHALL generate CA and device certificates

## ADDED Requirements

### Requirement: IP-only client connection
The Client GUI SHALL allow connecting to a Server by entering only the Server's IP address, without requiring certificate paths.

#### Scenario: Connect with only an IP
- **WHEN** the user enters a Server IP and triggers connect
- **AND** trust material already exists for that Server
- **THEN** the system SHALL connect using the persisted certificate
- **AND** the system SHALL NOT require the user to enter certificate paths

#### Scenario: Advanced certificate override
- **WHEN** the user opens advanced settings and supplies certificate, key, and CA paths
- **THEN** the system SHALL use those certificates instead of pairing-provisioned material

### Requirement: Pairing PIN in the GUI
The GUI SHALL surface the pairing PIN on the Server and prompt for it on the Client.

#### Scenario: Server displays the PIN
- **WHEN** pairing is enabled on the Server
- **THEN** the Server GUI SHALL display the current pairing PIN
- **AND** the Server GUI SHALL update the displayed PIN when it rotates

#### Scenario: Client prompts for the PIN
- **WHEN** the Client connects to a Server for which no trust material exists
- **THEN** the Client GUI SHALL prompt the user to enter the pairing PIN
- **AND** the system SHALL begin pairing once the PIN is submitted

#### Scenario: Pairing failure feedback
- **WHEN** pairing fails (wrong PIN, expired PIN, or lockout)
- **THEN** the Client GUI SHALL display an error indicating the reason
- **AND** the user SHALL be able to retry
