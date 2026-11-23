//! Asynchronous media decoding, disk cover cache, and video playback for BigPicture.

/// Target dimensions for game cover artwork in the launcher grid.
pub const COVER_WIDTH: u32 = 600;
pub const COVER_HEIGHT: u32 = 900;

/// Decoded RGBA8 image frame ready for upload to the UI renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaFrame {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl RgbaFrame {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Option<Self> {
        let expected = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        if pixels.len() != expected {
            return None;
        }
        Some(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_rgba_buffer_length() {
        assert!(RgbaFrame::new(2, 2, vec![0; 16]).is_some());
        assert!(RgbaFrame::new(2, 2, vec![0; 15]).is_none());
    }
}
