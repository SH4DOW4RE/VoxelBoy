//! Stable, core-neutral types shared by `VoxelBoy` emulator adapters.

#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// Hardware families understood by the frontend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum System {
    GameBoy,
    GameBoyColor,
    GameBoyAdvance,
    SuperGameBoy,
}

/// Pixel layout of a core-owned video buffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PixelFormat {
    Rgba8888,
    Bgra8888,
    Rgb565,
}

impl PixelFormat {
    #[must_use]
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgba8888 | Self::Bgra8888 => 4,
            Self::Rgb565 => 2,
        }
    }
}

/// A view of the most recently completed video frame.
#[derive(Clone, Copy, Debug)]
pub struct VideoFrame<'a> {
    pub pixels: &'a [u8],
    pub width: u32,
    pub height: u32,
    /// Byte distance between the start of adjacent rows.
    pub pitch: usize,
    pub format: PixelFormat,
}

impl VideoFrame<'_> {
    /// Checks dimensions before a renderer reads the borrowed buffer.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::InvalidFrame`] when dimensions overflow, the pitch
    /// is invalid, or the pixel buffer does not contain every declared row.
    pub fn validate(&self) -> Result<(), CoreError> {
        let row_bytes = usize::try_from(self.width)
            .ok()
            .and_then(|width| width.checked_mul(self.format.bytes_per_pixel()))
            .ok_or_else(|| CoreError::InvalidFrame("row size overflow".into()))?;
        if self.width == 0 || self.height == 0 {
            return Err(CoreError::InvalidFrame("zero-sized frame".into()));
        }
        if self.pitch < row_bytes {
            return Err(CoreError::InvalidFrame(
                "pitch is shorter than one row".into(),
            ));
        }
        let required = self
            .pitch
            .checked_mul(usize::try_from(self.height).unwrap_or(usize::MAX))
            .ok_or_else(|| CoreError::InvalidFrame("buffer size overflow".into()))?;
        if self.pixels.len() < required {
            return Err(CoreError::InvalidFrame("pixel buffer is truncated".into()));
        }
        Ok(())
    }
}

/// Input uses named buttons rather than a core-specific bit layout.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InputState {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub a: bool,
    pub b: bool,
    pub start: bool,
    pub select: bool,
    pub shoulder_left: bool,
    pub shoulder_right: bool,
}

/// Optional features are negotiated rather than assumed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Capabilities {
    pub save_states: bool,
    pub persistent_memory: bool,
    pub semantic_scene: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoreDescriptor {
    pub id: String,
    pub display_name: String,
    pub version: String,
    pub systems: Vec<System>,
    pub capabilities: Capabilities,
}

/// Stable semantic categories shared across hardware-specific adapters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SceneRole {
    Background,
    Window,
    Sprite,
    Bitmap,
}

/// A hardware adapter may supply these primitives after a frame completes.
#[derive(Clone, Debug, PartialEq)]
pub struct ScenePrimitive {
    /// Stable within a loaded game when the adapter can provide one.
    pub id: Option<u64>,
    pub role: SceneRole,
    pub draw_order: i32,
    pub position: [f32; 2],
    pub size: [f32; 2],
    pub visible: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SemanticScene {
    pub primitives: Vec<ScenePrimitive>,
}

/// All backends, including dynamic-core adapters, implement this interface.
pub trait EmulatorCore {
    fn descriptor(&self) -> &CoreDescriptor;

    /// Loads a game and reports the detected hardware.
    ///
    /// # Errors
    ///
    /// Returns an error when the image is invalid or unsupported by the core.
    fn load_game(&mut self, rom: &[u8]) -> Result<System, CoreError>;

    fn unload_game(&mut self);

    /// Restarts the loaded game.
    ///
    /// # Errors
    ///
    /// Returns an error when no game is loaded or the backend cannot reset.
    fn reset(&mut self) -> Result<(), CoreError>;

    /// Advances emulation by one video frame.
    ///
    /// # Errors
    ///
    /// Returns an error when no game is loaded or the backend fails.
    fn run_frame(&mut self, input: InputState) -> Result<(), CoreError>;

    fn video_frame(&self) -> Option<VideoFrame<'_>>;
    fn audio_samples(&self) -> &[i16];

    fn semantic_scene(&self) -> Option<&SemanticScene> {
        None
    }

    /// Serializes the current emulator state.
    ///
    /// # Errors
    ///
    /// Returns an error if save states are unsupported or serialization fails.
    fn save_state(&self) -> Result<Vec<u8>, CoreError> {
        Err(CoreError::Unsupported("save states"))
    }

    /// Restores a previously serialized emulator state.
    ///
    /// # Errors
    ///
    /// Returns an error if save states are unsupported or the data is invalid.
    fn load_state(&mut self, _state: &[u8]) -> Result<(), CoreError> {
        Err(CoreError::Unsupported("save states"))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoreError {
    InvalidFrame(String),
    InvalidRom(String),
    NotLoaded,
    Unsupported(&'static str),
    Backend(String),
}

impl Display for CoreError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFrame(message) => write!(formatter, "invalid video frame: {message}"),
            Self::InvalidRom(message) => write!(formatter, "invalid ROM: {message}"),
            Self::NotLoaded => formatter.write_str("no game is loaded"),
            Self::Unsupported(feature) => write!(formatter, "unsupported feature: {feature}"),
            Self::Backend(message) => write!(formatter, "core error: {message}"),
        }
    }
}

impl Error for CoreError {}

#[cfg(test)]
mod tests {
    use super::{PixelFormat, VideoFrame};

    #[test]
    fn validates_pitched_frame() {
        let pixels = [0_u8; 24];
        let frame = VideoFrame {
            pixels: &pixels,
            width: 2,
            height: 2,
            pitch: 12,
            format: PixelFormat::Rgba8888,
        };
        assert!(frame.validate().is_ok());
    }

    #[test]
    fn rejects_truncated_frame() {
        let pixels = [0_u8; 15];
        let frame = VideoFrame {
            pixels: &pixels,
            width: 2,
            height: 2,
            pitch: 8,
            format: PixelFormat::Rgba8888,
        };
        assert!(frame.validate().is_err());
    }
}
