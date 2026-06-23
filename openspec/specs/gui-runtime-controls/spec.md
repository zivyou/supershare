## ADDED Requirements

### Requirement: Server start/stop control in GUI
The GUI SHALL provide buttons to start and stop the Server, and display the current running status.

#### Scenario: Start server
- **WHEN** the user clicks the "Start Server" button on the Server tab
- **AND** the TLS certificate, key, and CA paths are configured
- **THEN** the system SHALL start the Server on the configured ports
- **AND** the button SHALL change to "Stop Server"
- **AND** the status SHALL display "Running (port XXXX)"

#### Scenario: Stop server
- **WHEN** the user clicks the "Stop Server" button
- **THEN** the system SHALL stop the Server and disconnect all clients
- **AND** the button SHALL change to "Start Server"
- **AND** the status SHALL display "Stopped"

#### Scenario: Start server with missing config
- **WHEN** the user clicks "Start Server" but certificate, key, or CA path is not configured
- **THEN** the system SHALL display an error message indicating which paths are missing

### Requirement: Client connect/disconnect control in GUI
The GUI SHALL provide buttons to connect and disconnect the Client, and display the current connection status.

#### Scenario: Connect client
- **WHEN** the user clicks the "Connect" button on the Client tab
- **AND** the server address, TLS certificate, key, and CA paths are configured
- **THEN** the system SHALL connect to the specified Server
- **AND** the button SHALL change to "Disconnect"
- **AND** the status SHALL display "Connected to XXXX"

#### Scenario: Disconnect client
- **WHEN** the user clicks the "Disconnect" button
- **THEN** the system SHALL disconnect from the Server
- **AND** the button SHALL change to "Connect"
- **AND** the status SHALL display "Disconnected"

#### Scenario: Connection failure
- **WHEN** the Client attempts to connect but the Server is unreachable
- **THEN** the system SHALL display an error message
- **AND** the status SHALL display "Connection failed"

### Requirement: Real-time connected clients list
The Server tab SHALL display a real-time list of currently connected clients.

#### Scenario: Client connects
- **WHEN** a new Client connects to the Server
- **THEN** the client list SHALL update to include the new client with its name
- **AND** the list SHALL update within 1 second

#### Scenario: Client disconnects
- **WHEN** a Client disconnects from the Server
- **THEN** the client list SHALL update to remove the disconnected client
- **AND** the list SHALL update within 1 second

#### Scenario: No clients connected
- **WHEN** no Clients are connected to the Server
- **THEN** the client list SHALL display "No clients connected"

### Requirement: Client connection status display
The Client tab SHALL display the current connection status including the Server address and screen info.

#### Scenario: Connected to server
- **WHEN** the Client is connected to a Server
- **THEN** the Client tab SHALL display the Server address
- **AND** the Client tab SHALL display the Server's screen resolution

#### Scenario: Disconnected
- **WHEN** the Client is not connected
- **THEN** the Client tab SHALL display "Not connected"

### Requirement: Shared application state
The system SHALL maintain a shared state between the UI and backend runtime.

#### Scenario: State updates from backend
- **WHEN** the backend detects a client connection or disconnection
- **THEN** the SharedAppState SHALL be updated
- **AND** the UI SHALL reflect the change on the next frame

#### Scenario: Commands from UI to backend
- **WHEN** the user clicks a control button (Start/Stop/Connect/Disconnect)
- **THEN** the system SHALL send the corresponding command via the AppCommand channel
- **AND** the backend SHALL execute the command asynchronously

### Requirement: Application exit on window close
The application SHALL exit completely when the user closes the GUI window.

#### Scenario: Window close
- **WHEN** the user closes the GUI window
- **THEN** the application SHALL stop any running Server or Client
- **AND** the application SHALL exit the process
