## ADDED Requirements

### Requirement: egui configuration window
The system SHALL provide an egui-based configuration window for managing Server and Client settings.

#### Scenario: Open configuration window
- **WHEN** the user runs `supershare gui` or double-clicks the system tray icon
- **THEN** the system SHALL display the egui configuration window

#### Scenario: Server mode configuration
- **WHEN** the user selects Server mode in the configuration window
- **THEN** the system SHALL display fields for: listen port (default 9876), TLS certificate path, TLS key path
- **AND** the system SHALL display a list of connected clients with their name, IP, resolution, and screen position

#### Scenario: Client mode configuration
- **WHEN** the user selects Client mode in the configuration window
- **THEN** the system SHALL display fields for: Server IP:port, TLS certificate path, device name
- **AND** the system SHALL display connection status (connected/disconnected)

### Requirement: System tray integration
The system SHALL display a system tray icon when running in Server or Client mode.

#### Scenario: Tray icon display
- **WHEN** the application starts in Server or Client mode
- **THEN** the system SHALL display a system tray icon indicating the current mode

#### Scenario: Tray right-click menu
- **WHEN** the user right-clicks the system tray icon
- **THEN** the system SHALL display a menu with: "Open Settings", "Connection Status", and "Quit" options

#### Scenario: Tray double-click
- **WHEN** the user double-clicks the system tray icon
- **THEN** the system SHALL show/bring to front the configuration window

#### Scenario: Window close to tray
- **WHEN** the user closes the configuration window
- **THEN** the application SHALL minimize to the system tray (not exit)
- **AND** the background service SHALL continue running

### Requirement: Clipboard sync settings
The system SHALL allow users to configure clipboard synchronization options.

#### Scenario: Enable/disable clipboard sync
- **WHEN** the user toggles clipboard sync in settings
- **THEN** the system SHALL enable or disable clipboard synchronization accordingly

#### Scenario: Configure image size limit
- **WHEN** the user sets the maximum image transfer size
- **THEN** the system SHALL enforce this limit (default: 10 MB)
- **AND** images exceeding the limit SHALL be rejected with a log warning

#### Scenario: Select content types
- **WHEN** the user configures clipboard settings
- **THEN** the system SHALL allow toggling text sync and image sync independently

### Requirement: Certificate management
The system SHALL provide tools for generating and managing TLS certificates.

#### Scenario: Generate certificates via CLI
- **WHEN** the user runs `supershare gen-cert --output ./certs`
- **THEN** the system SHALL generate a self-signed CA certificate and key
- **AND** the system SHALL generate a device certificate and key signed by the CA
- **AND** the system SHALL save all files to the specified output directory

#### Scenario: Certificate path configuration
- **WHEN** the user configures TLS settings in the UI
- **THEN** the system SHALL provide file selection dialogs for certificate and key paths
- **AND** the system SHALL validate that the files exist and are valid PEM format

### Requirement: CLI subcommands
The system SHALL support the following CLI subcommands via clap.

#### Scenario: Start as Server
- **WHEN** the user runs `supershare server [--port 9876] --cert cert.pem --key key.pem`
- **THEN** the system SHALL start in Server mode with the specified configuration

#### Scenario: Start as Client
- **WHEN** the user runs `supershare client --server 192.168.1.100:9876 --cert cert.pem --key key.pem --ca ca.pem`
- **THEN** the system SHALL start in Client mode and connect to the specified Server

#### Scenario: Open GUI
- **WHEN** the user runs `supershare gui`
- **THEN** the system SHALL open the egui configuration window

#### Scenario: Generate certificates
- **WHEN** the user runs `supershare gen-cert [--output ./certs]`
- **THEN** the system SHALL generate CA and device certificates

### Requirement: Configuration persistence
The system SHALL persist configuration to a TOML file in the user's config directory.

#### Scenario: Save configuration
- **WHEN** the user changes settings in the UI
- **THEN** the system SHALL save the configuration to `~/.config/supershare/config.toml` (Linux) or `%APPDATA%\supershare\config.toml` (Windows)

#### Scenario: Load configuration on startup
- **WHEN** the application starts
- **THEN** the system SHALL load the configuration file if it exists
- **AND** apply saved settings as defaults
