pub mod capture;
pub mod inject;
pub mod boundary;
pub mod virtual_cursor;
pub mod warp_capture;

#[cfg(target_os = "linux")]
pub mod evdev_capture;
