use ss_core::protocol::BOUNDARY_ZONE_PX;

/// Describes a screen in the global coordinate system
#[derive(Debug, Clone)]
pub struct ScreenInfo {
    /// Unique screen ID (0 = server, 1+ = clients)
    pub id: u8,
    /// Device name
    pub name: String,
    /// Screen width in pixels
    pub width: u32,
    /// Screen height in pixels
    pub height: u32,
    /// X offset in the global coordinate system
    pub offset_x: u32,
}

/// Global coordinate system manager for horizontal screen layout
pub struct CoordinateSystem {
    /// All screens ordered left to right
    pub screens: Vec<ScreenInfo>,
}

impl CoordinateSystem {
    /// Create a new coordinate system with the server's screen as screen 0
    pub fn new(server_width: u32, server_height: u32) -> Self {
        Self {
            screens: vec![ScreenInfo {
                id: 0,
                name: "server".to_string(),
                width: server_width,
                height: server_height,
                offset_x: 0,
            }],
        }
    }

    /// Add a client screen to the right of all existing screens
    pub fn add_screen(&mut self, id: u8, name: String, width: u32, height: u32) {
        let offset_x = self
            .screens
            .iter()
            .map(|s| s.offset_x + s.width)
            .max()
            .unwrap_or(0);
        self.screens.push(ScreenInfo {
            id,
            name,
            width,
            height,
            offset_x,
        });
    }

    /// Remove a screen by ID
    pub fn remove_screen(&mut self, id: u8) {
        self.screens.retain(|s| s.id != id);
        // Recalculate offsets
        let mut offset = 0u32;
        for screen in &mut self.screens {
            screen.offset_x = offset;
            offset += screen.width;
        }
    }

    /// Find which screen contains the given global x coordinate
    pub fn screen_at_x(&self, global_x: f32) -> Option<&ScreenInfo> {
        let x = global_x as u32;
        self.screens
            .iter()
            .find(|s| x >= s.offset_x && x < s.offset_x + s.width)
    }

    /// Get the screen ID for a global x coordinate
    pub fn screen_id_at_x(&self, global_x: f32) -> Option<u8> {
        self.screen_at_x(global_x).map(|s| s.id)
    }

    /// Check if a mouse position is in the boundary zone (within BOUNDARY_ZONE_PX of an edge)
    /// Returns Some((target_screen_id, enter_x, enter_y)) if crossing, None otherwise
    pub fn check_boundary(&self, screen_id: u8, local_x: f32, local_y: f32) -> Option<(u8, f32, f32)> {
        let screen = self.screens.iter().find(|s| s.id == screen_id)?;

        // Check right edge
        if local_x >= (screen.width as f32 - BOUNDARY_ZONE_PX as f32) {
            // Find the screen to the right
            let right_screen = self.screens.iter().find(|s| s.offset_x == screen.offset_x + screen.width)?;
            return Some((right_screen.id, 0.0, local_y));
        }

        // Check left edge
        if local_x <= BOUNDARY_ZONE_PX as f32 {
            // Find the screen to the left
            let left_screen = self
                .screens
                .iter()
                .find(|s| s.offset_x + s.width == screen.offset_x)?;
            return Some((left_screen.id, left_screen.width as f32 - 1.0, local_y));
        }

        None
    }

    /// Convert local screen coordinates to global coordinates
    pub fn local_to_global(&self, screen_id: u8, local_x: f32, local_y: f32) -> (f32, f32) {
        if let Some(screen) = self.screens.iter().find(|s| s.id == screen_id) {
            (screen.offset_x as f32 + local_x, local_y)
        } else {
            (local_x, local_y)
        }
    }

    /// Convert global coordinates to local screen coordinates
    pub fn global_to_local(&self, global_x: f32, global_y: f32) -> Option<(u8, f32, f32)> {
        let screen = self.screen_at_x(global_x)?;
        let local_x = global_x - screen.offset_x as f32;
        Some((screen.id, local_x, global_y))
    }

    /// Get the total width of all screens
    pub fn total_width(&self) -> u32 {
        self.screens
            .iter()
            .map(|s| s.offset_x + s.width)
            .max()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_horizontal_layout() {
        let mut cs = CoordinateSystem::new(1920, 1080);
        cs.add_screen(1, "laptop".to_string(), 2560, 1440);

        assert_eq!(cs.screens.len(), 2);
        assert_eq!(cs.screens[0].offset_x, 0);
        assert_eq!(cs.screens[1].offset_x, 1920);
        assert_eq!(cs.total_width(), 4480);
    }

    #[test]
    fn test_boundary_right_edge() {
        let mut cs = CoordinateSystem::new(1920, 1080);
        cs.add_screen(1, "right".to_string(), 1920, 1080);

        // Mouse at right edge of screen 0
        let result = cs.check_boundary(0, 1918.0, 500.0);
        assert!(result.is_some());
        let (target, enter_x, enter_y) = result.unwrap();
        assert_eq!(target, 1);
        assert_eq!(enter_x, 0.0);
        assert_eq!(enter_y, 500.0);
    }

    #[test]
    fn test_boundary_left_edge() {
        let mut cs = CoordinateSystem::new(1920, 1080);
        cs.add_screen(1, "right".to_string(), 1920, 1080);

        // Mouse at left edge of screen 1
        let result = cs.check_boundary(1, 2.0, 500.0);
        assert!(result.is_some());
        let (target, enter_x, enter_y) = result.unwrap();
        assert_eq!(target, 0);
        assert_eq!(enter_x, 1919.0);
        assert_eq!(enter_y, 500.0);
    }

    #[test]
    fn test_no_boundary() {
        let mut cs = CoordinateSystem::new(1920, 1080);
        cs.add_screen(1, "right".to_string(), 1920, 1080);

        // Mouse in middle of screen
        let result = cs.check_boundary(0, 960.0, 500.0);
        assert!(result.is_none());
    }
}
