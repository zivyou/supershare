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
    fn test_screen_at_x() {
        let mut cs = CoordinateSystem::new(1920, 1080);
        cs.add_screen(1, "right".to_string(), 1920, 1080);

        // Server screen
        assert_eq!(cs.screen_id_at_x(960.0), Some(0));
        assert_eq!(cs.screen_id_at_x(1919.0), Some(0));

        // Client screen
        assert_eq!(cs.screen_id_at_x(1920.0), Some(1));
        assert_eq!(cs.screen_id_at_x(3839.0), Some(1));
    }
}
