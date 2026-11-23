//! Operating-system display management and window/process control abstractions.

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SystemError {
    #[error("operation not supported on this platform/compositor: {0}")]
    Unsupported(&'static str),
    #[error("system operation failed: {0}")]
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayMode {
    width: u32,
    height: u32,
    refresh_hz: u32,
}

impl DisplayMode {
    pub fn new(width: u32, height: u32, refresh_hz: u32) -> Self {
        Self {
            width,
            height,
            refresh_hz,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn refresh_hz(&self) -> u32 {
        self.refresh_hz
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_mode_accessors() {
        let mode = DisplayMode::new(1920, 1080, 120);
        assert_eq!(mode.width(), 1920);
        assert_eq!(mode.height(), 1080);
        assert_eq!(mode.refresh_hz(), 120);
    }
}
