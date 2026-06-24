## ADDED Requirements

### Requirement: Mouse event capture via evdev (delta-based)
The system SHALL capture mouse movement events using evdev directly from `/dev/input/event*` devices, obtaining raw `EV_REL::REL_X/REL_Y` deltas that are NOT clamped by screen boundaries.

#### Scenario: Mouse moves within Server screen
- **WHEN** the mouse moves within the Server's screen boundaries
- **THEN** the system SHALL update the virtual cursor position by the delta
- **AND** the system SHALL pass the event through to the local UInput device (local injection)

#### Scenario: Mouse crosses to Client screen
- **WHEN** the virtual cursor's global X coordinate reaches or exceeds the Server's screen width
- **THEN** the system SHALL switch to REMOTE mode
- **AND** the system SHALL send a `BoundaryEnter` message to the Client with the entry position
- **AND** the system SHALL send `MouseDelta` messages for subsequent mouse movements
- **AND** the system SHALL suppress local injection (do NOT write to UInput)

#### Scenario: Mouse returns to Server screen
- **WHEN** the virtual cursor's global X coordinate drops below the Server's screen width while in REMOTE mode
- **THEN** the system SHALL switch to LOCAL mode
- **AND** the system SHALL send a `BoundaryLeave` message to the Client
- **AND** the system SHALL resume local injection via UInput

### Requirement: Keyboard and button event forwarding
The system SHALL capture keyboard and mouse button events via evdev and forward them to the Client when in REMOTE mode.

#### Scenario: Key press while in REMOTE mode
- **WHEN** a keyboard event occurs on the Server while in REMOTE mode
- **THEN** the system SHALL suppress local injection
- **AND** the system SHALL send a `KeyPress` message to the Client

#### Scenario: Key press while in LOCAL mode
- **WHEN** a keyboard event occurs on the Server while in LOCAL mode
- **THEN** the system SHALL pass the event through to the local UInput device

### Requirement: Input injection on Client (passive mode)
The system SHALL inject received mouse and keyboard events on the Client machine using `rdev::simulate`. The Client operates in pure passive mode with NO local input capture.

#### Scenario: Client receives MouseDelta
- **WHEN** the Client receives a `MouseDelta` message
- **THEN** the system SHALL update the Client's virtual cursor position by the delta
- **AND** the system SHALL clamp the position to the Client's screen bounds
- **AND** the system SHALL inject a `MouseMove` event at the virtual cursor position via `rdev::simulate`

#### Scenario: Client receives BoundaryEnter
- **WHEN** the Client receives a `BoundaryEnter` message
- **THEN** the system SHALL set the virtual cursor position to the entry coordinates
- **AND** the system SHALL inject a `MouseMove` event at the entry position

### Requirement: Virtual cursor management
The Server SHALL maintain a virtual cursor position in global coordinates, tracking the cumulative effect of all mouse deltas.

#### Scenario: Virtual cursor position tracking
- **WHEN** the Server receives an `EV_REL::REL_X` or `EV_REL::REL_Y` event
- **THEN** the system SHALL add the delta to the virtual cursor's global position
- **AND** the system SHALL clamp the position to the global coordinate system bounds

#### Scenario: Screen boundary detection via virtual cursor
- **WHEN** the virtual cursor's screen assignment changes (determined by `CoordinateSystem::screen_at_x`)
- **THEN** the system SHALL trigger a mode switch between LOCAL and REMOTE

### Requirement: evdev device grab and UInput pass-through
The system SHALL grab all input devices for exclusive access and create UInput virtual device copies for local event pass-through.

#### Scenario: Device grab on startup
- **WHEN** the Server starts input capture
- **THEN** the system SHALL open all `/dev/input/event*` character devices
- **AND** the system SHALL call `grab(GrabMode::Grab)` on each device
- **AND** the system SHALL create a `UInputDevice` copy for each physical device

#### Scenario: Local event pass-through
- **WHEN** the system is in LOCAL mode and receives an input event
- **THEN** the system SHALL write the original evdev event to the corresponding UInput device

#### Scenario: Remote event suppression
- **WHEN** the system is in REMOTE mode and receives an input event
- **THEN** the system SHALL NOT write the event to the UInput device
- **AND** the system SHALL forward the event to the Client via the network

### Requirement: Device hot-plug support
The system SHALL detect new input devices being plugged in and add them to the capture pool.

#### Scenario: New device plugged in
- **WHEN** a new character device appears in `/dev/input/`
- **THEN** the system SHALL detect it via inotify CREATE event
- **AND** the system SHALL open, grab, and create a UInput copy for the new device
