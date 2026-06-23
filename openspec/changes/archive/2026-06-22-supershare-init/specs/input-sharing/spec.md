## ADDED Requirements

### Requirement: Mouse event capture and forwarding
The system SHALL capture mouse move, button, and scroll events on the Server machine and forward them to the appropriate Client machine via the control channel.

#### Scenario: Mouse moves within Server screen
- **WHEN** the mouse moves within the Server's screen boundaries
- **THEN** the system SHALL process the event locally (no forwarding)

#### Scenario: Mouse crosses to Client screen
- **WHEN** the mouse reaches the right edge of the Server's screen (within the boundary zone of 5 pixels)
- **THEN** the system SHALL send a `BoundaryEnter` message to the target Client with the entry position
- **AND** the system SHALL suppress local input processing on the Server
- **AND** the Client SHALL begin injecting mouse events at the specified entry position

#### Scenario: Mouse returns to Server screen
- **WHEN** the mouse reaches the left edge of the Client's screen (within the boundary zone)
- **THEN** the Client SHALL send a `BoundaryLeave` message to the Server
- **AND** the Server SHALL resume local input capture
- **AND** the Client SHALL stop injecting mouse events

### Requirement: Keyboard event capture and forwarding
The system SHALL capture keyboard press/release events on the Server machine and forward them to the currently active Client machine via the control channel.

#### Scenario: Key press while mouse is on Client screen
- **WHEN** a keyboard event occurs on the Server while the mouse cursor is on a Client's screen
- **THEN** the system SHALL forward the keyboard event to that Client via the control channel
- **AND** the Client SHALL inject the keyboard event locally

#### Scenario: Key press while mouse is on Server screen
- **WHEN** a keyboard event occurs on the Server while the mouse cursor is on the Server's screen
- **THEN** the system SHALL process the event locally (no forwarding)

### Requirement: Input injection on Client
The system SHALL inject received mouse and keyboard events on the Client machine using platform-native APIs.

#### Scenario: Inject mouse move on Linux X11
- **WHEN** the Client receives a `MouseMove` message on Linux with X11
- **THEN** the system SHALL use rdev to simulate the mouse movement

#### Scenario: Inject mouse move on Linux Wayland
- **WHEN** the Client receives a `MouseMove` message on Linux with Wayland
- **THEN** the system SHALL use uinput virtual device to simulate the mouse movement

#### Scenario: Inject mouse move on Windows
- **WHEN** the Client receives a `MouseMove` message on Windows
- **THEN** the system SHALL use rdev (SendInput API) to simulate the mouse movement

### Requirement: Screen boundary detection
The system SHALL detect when the mouse cursor crosses screen boundaries and trigger the appropriate mode switch.

#### Scenario: Boundary zone detection
- **WHEN** the mouse cursor position is within 5 pixels of a screen edge
- **THEN** the system SHALL treat this as a boundary crossing event

#### Scenario: Multi-client horizontal layout
- **WHEN** Server screen is 1920px wide and Client B is configured to the right
- **THEN** the logical coordinate system SHALL map Client B's screen starting at x=1920
- **AND** moving the mouse past x=1915 (Server right edge) SHALL trigger entry into Client B at x=0

### Requirement: Mouse position synchronization
The Client SHALL report mouse position back to the Server while the mouse is on the Client's screen, so the Server maintains an accurate global coordinate system.

#### Scenario: Client reports mouse position
- **WHEN** the mouse is on a Client's screen and moves
- **THEN** the Client SHALL send `MouseMove` messages to the Server via the control channel
- **AND** the Server SHALL update its global coordinate tracking
