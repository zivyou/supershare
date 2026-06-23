## MODIFIED Requirements

### Requirement: CLI subcommands
The system SHALL support the following CLI options via clap. When no arguments are provided, the system SHALL open the GUI.

#### Scenario: Default behavior (no arguments)
- **WHEN** the user runs `supershare` with no arguments
- **THEN** the system SHALL open the GUI configuration window

#### Scenario: Start as Server (headless)
- **WHEN** the user runs `supershare --server [--port 9876] --cert cert.pem --key key.pem --ca ca.pem`
- **THEN** the system SHALL start in headless Server mode with the specified configuration

#### Scenario: Start as Client (headless)
- **WHEN** the user runs `supershare --client --server 192.168.1.100:9876 --cert cert.pem --key key.pem --ca ca.pem`
- **THEN** the system SHALL start in headless Client mode and connect to the specified Server

#### Scenario: Generate certificates
- **WHEN** the user runs `supershare gen-cert [--output ./certs]`
- **THEN** the system SHALL generate CA and device certificates
