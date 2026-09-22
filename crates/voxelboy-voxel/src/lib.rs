//! Voxel conversion that works with the minimum core capability: a video frame.

#![forbid(unsafe_code)]

use std::collections::VecDeque;

use voxelboy_core_api::{CoreError, PixelFormat, VideoFrame};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rgba8 {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Voxel {
    pub position: [f32; 3],
    pub size: [f32; 3],
    pub color: Rgba8,
}

#[derive(Clone, Copy, Debug)]
pub struct FramebufferProfile {
    /// Exact color treated as empty space. `None` keeps every pixel.
    pub background: Option<Rgba8>,
    /// Controls whether matching object pixels are also removed.
    pub background_policy: BackgroundPolicy,
    pub pixel_size: f32,
    pub minimum_depth: f32,
    pub depth_range: f32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BackgroundPolicy {
    /// Remove every pixel matching the configured background color.
    #[default]
    AllMatching,
    /// Remove matches only when connected to an edge of the video frame.
    BorderConnected,
}

impl Default for FramebufferProfile {
    fn default() -> Self {
        Self {
            background: None,
            background_policy: BackgroundPolicy::default(),
            pixel_size: 1.0,
            minimum_depth: 0.25,
            depth_range: 1.0,
        }
    }
}

/// Converts each visible source pixel into one voxel. Mesh merging belongs to
/// the renderer and can be optimized without changing this stable output.
///
/// # Errors
///
/// Returns [`CoreError::InvalidFrame`] if the source frame is malformed or its
/// dimensions cannot be represented safely.
#[allow(clippy::cast_precision_loss)]
pub fn voxelize_frame(
    frame: VideoFrame<'_>,
    profile: FramebufferProfile,
) -> Result<Vec<Voxel>, CoreError> {
    frame.validate()?;
    let capacity = usize::try_from(frame.width)
        .ok()
        .and_then(|width| {
            usize::try_from(frame.height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .ok_or_else(|| CoreError::InvalidFrame("voxel count overflow".into()))?;
    let mut voxels = Vec::with_capacity(capacity);
    let border_background = match (profile.background, profile.background_policy) {
        (Some(background), BackgroundPolicy::BorderConnected) => {
            Some(border_connected_background(frame, background, capacity))
        }
        _ => None,
    };

    for y in 0..frame.height {
        for x in 0..frame.width {
            let color = read_pixel(frame, x, y);
            let index = usize::try_from(y).unwrap_or(0) * usize::try_from(frame.width).unwrap_or(0)
                + usize::try_from(x).unwrap_or(0);
            let is_background = match profile.background_policy {
                BackgroundPolicy::AllMatching => profile.background == Some(color),
                BackgroundPolicy::BorderConnected => border_background
                    .as_ref()
                    .is_some_and(|background| background[index]),
            };
            if color.alpha == 0 || is_background {
                continue;
            }
            let luminance = (0.2126 * f32::from(color.red)
                + 0.7152 * f32::from(color.green)
                + 0.0722 * f32::from(color.blue))
                / 255.0;
            let depth = profile.minimum_depth + profile.depth_range * (1.0 - luminance);
            voxels.push(Voxel {
                position: [
                    x as f32 * profile.pixel_size,
                    -(y as f32) * profile.pixel_size,
                    0.0,
                ],
                size: [profile.pixel_size, profile.pixel_size, depth],
                color,
            });
        }
    }

    Ok(voxels)
}

fn border_connected_background(
    frame: VideoFrame<'_>,
    background: Rgba8,
    pixel_count: usize,
) -> Vec<bool> {
    let width = usize::try_from(frame.width).unwrap_or(0);
    let height = usize::try_from(frame.height).unwrap_or(0);
    let mut removed = vec![false; pixel_count];
    let mut pending = VecDeque::new();

    for x in 0..width {
        enqueue_background(frame, background, x, 0, width, &mut removed, &mut pending);
        if height > 1 {
            enqueue_background(
                frame,
                background,
                x,
                height - 1,
                width,
                &mut removed,
                &mut pending,
            );
        }
    }
    for y in 1..height.saturating_sub(1) {
        enqueue_background(frame, background, 0, y, width, &mut removed, &mut pending);
        if width > 1 {
            enqueue_background(
                frame,
                background,
                width - 1,
                y,
                width,
                &mut removed,
                &mut pending,
            );
        }
    }

    while let Some(index) = pending.pop_front() {
        let x = index % width;
        let y = index / width;
        if x > 0 {
            enqueue_background(
                frame,
                background,
                x - 1,
                y,
                width,
                &mut removed,
                &mut pending,
            );
        }
        if x + 1 < width {
            enqueue_background(
                frame,
                background,
                x + 1,
                y,
                width,
                &mut removed,
                &mut pending,
            );
        }
        if y > 0 {
            enqueue_background(
                frame,
                background,
                x,
                y - 1,
                width,
                &mut removed,
                &mut pending,
            );
        }
        if y + 1 < height {
            enqueue_background(
                frame,
                background,
                x,
                y + 1,
                width,
                &mut removed,
                &mut pending,
            );
        }
    }
    removed
}

#[allow(clippy::too_many_arguments)]
fn enqueue_background(
    frame: VideoFrame<'_>,
    background: Rgba8,
    x: usize,
    y: usize,
    width: usize,
    removed: &mut [bool],
    pending: &mut VecDeque<usize>,
) {
    let index = y * width + x;
    if removed[index] {
        return;
    }
    let Ok(x) = u32::try_from(x) else {
        return;
    };
    let Ok(y) = u32::try_from(y) else {
        return;
    };
    if read_pixel(frame, x, y) == background {
        removed[index] = true;
        pending.push_back(index);
    }
}

fn read_pixel(frame: VideoFrame<'_>, x: u32, y: u32) -> Rgba8 {
    let bytes_per_pixel = frame.format.bytes_per_pixel();
    let offset = usize::try_from(y).unwrap_or(0) * frame.pitch
        + usize::try_from(x).unwrap_or(0) * bytes_per_pixel;
    match frame.format {
        PixelFormat::Rgba8888 => Rgba8 {
            red: frame.pixels[offset],
            green: frame.pixels[offset + 1],
            blue: frame.pixels[offset + 2],
            alpha: frame.pixels[offset + 3],
        },
        PixelFormat::Bgra8888 => Rgba8 {
            red: frame.pixels[offset + 2],
            green: frame.pixels[offset + 1],
            blue: frame.pixels[offset],
            alpha: frame.pixels[offset + 3],
        },
        PixelFormat::Rgb565 => {
            let packed = u16::from_le_bytes([frame.pixels[offset], frame.pixels[offset + 1]]);
            let red = ((packed >> 11) & 0x1f) as u8;
            let green = ((packed >> 5) & 0x3f) as u8;
            let blue = (packed & 0x1f) as u8;
            Rgba8 {
                red: (red << 3) | (red >> 2),
                green: (green << 2) | (green >> 4),
                blue: (blue << 3) | (blue >> 2),
                alpha: u8::MAX,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BackgroundPolicy, FramebufferProfile, Rgba8, voxelize_frame};
    use voxelboy_core_api::{PixelFormat, VideoFrame};

    #[test]
    fn removes_profile_background_and_extrudes_foreground() {
        let pixels = [255, 255, 255, 255, 0, 0, 0, 255];
        let frame = VideoFrame {
            pixels: &pixels,
            width: 2,
            height: 1,
            pitch: 8,
            format: PixelFormat::Rgba8888,
        };
        let profile = FramebufferProfile {
            background: Some(Rgba8 {
                red: 255,
                green: 255,
                blue: 255,
                alpha: 255,
            }),
            ..FramebufferProfile::default()
        };

        let voxels = voxelize_frame(frame, profile).expect("valid frame");
        assert_eq!(voxels.len(), 1);
        assert!((voxels[0].position[0] - 1.0).abs() < f32::EPSILON);
        assert!(voxels[0].position[1].abs() < f32::EPSILON);
        assert!(voxels[0].position[2].abs() < f32::EPSILON);
        assert!((voxels[0].size[2] - 1.25).abs() < f32::EPSILON);
    }

    #[test]
    fn decodes_rgb565() {
        let pixels = [0x00, 0xf8];
        let frame = VideoFrame {
            pixels: &pixels,
            width: 1,
            height: 1,
            pitch: 2,
            format: PixelFormat::Rgb565,
        };

        let voxels = voxelize_frame(frame, FramebufferProfile::default()).expect("valid frame");
        assert_eq!(voxels[0].color.red, 255);
        assert_eq!(voxels[0].color.green, 0);
        assert_eq!(voxels[0].color.blue, 0);
    }

    #[test]
    fn border_background_preserves_enclosed_matching_pixels() {
        let white = [255, 255, 255, 255];
        let black = [0, 0, 0, 255];
        let mut pixels = Vec::new();
        for y in 0..5 {
            for x in 0..5 {
                let is_outer_background = x == 0 || x == 4 || y == 0 || y == 4;
                let color = if is_outer_background || (x == 2 && y == 2) {
                    white
                } else {
                    black
                };
                pixels.extend_from_slice(&color);
            }
        }
        let frame = VideoFrame {
            pixels: &pixels,
            width: 5,
            height: 5,
            pitch: 20,
            format: PixelFormat::Rgba8888,
        };
        let profile = FramebufferProfile {
            background: Some(Rgba8 {
                red: 255,
                green: 255,
                blue: 255,
                alpha: 255,
            }),
            background_policy: BackgroundPolicy::BorderConnected,
            ..FramebufferProfile::default()
        };

        let voxels = voxelize_frame(frame, profile).expect("valid frame");
        assert_eq!(voxels.len(), 9);
        assert_eq!(
            voxels.iter().filter(|voxel| voxel.color.red == 255).count(),
            1
        );
    }
}
