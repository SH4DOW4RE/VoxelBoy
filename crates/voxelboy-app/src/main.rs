use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::{Parser, ValueEnum};
use pixels::{Pixels, SurfaceTexture};
use voxelboy_core_api::{EmulatorCore, GameImage, InputState, System, VideoFrame};
use voxelboy_core_libretro::LibretroCore;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

const DEFAULT_FRAMES_PER_SECOND: f64 = 60.0;
const INITIAL_SCALE: u32 = 4;
const MAX_CATCH_UP_FRAMES: usize = 4;

#[derive(Debug, Parser)]
#[command(version, about = "Game Boy-family emulation with voxel rendering")]
struct Arguments {
    /// Path to a libretro core dynamic library (.dll, .so, or .dylib).
    #[arg(long)]
    core: PathBuf,

    /// Path to a game ROM.
    #[arg(long)]
    rom: PathBuf,

    /// Override automatic system detection.
    #[arg(long, value_enum, default_value_t = SystemArgument::Auto)]
    system: SystemArgument,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum SystemArgument {
    #[default]
    Auto,
    Gb,
    Gbc,
    Gba,
    Sgb,
}

impl SystemArgument {
    fn resolve(self, path: &Path, rom: &[u8]) -> Result<System, String> {
        match self {
            Self::Auto => detect_system(path, rom),
            Self::Gb => Ok(System::GameBoy),
            Self::Gbc => Ok(System::GameBoyColor),
            Self::Gba => Ok(System::GameBoyAdvance),
            Self::Sgb => Ok(System::SuperGameBoy),
        }
    }
}

struct DesktopApp {
    core: LibretroCore,
    rom_name: String,
    input: InputState,
    window: Option<Arc<Window>>,
    pixels: Option<Pixels<'static>>,
    frame_width: u32,
    frame_height: u32,
    frame_period: Duration,
    next_frame: Instant,
    paused: bool,
    fatal_error: Option<String>,
}

impl DesktopApp {
    fn new(mut core: LibretroCore, rom_name: String) -> Result<Self, Box<dyn Error>> {
        core.run_frame(InputState::default())?;
        let frame = core
            .video_frame()
            .ok_or("core did not produce a video frame")?;
        frame.validate()?;
        let frame_width = frame.width;
        let frame_height = frame.height;
        let frames_per_second = core
            .system_timing()
            .map(|timing| timing.frames_per_second)
            .filter(|fps| fps.is_finite() && *fps > 0.0)
            .unwrap_or(DEFAULT_FRAMES_PER_SECOND);

        Ok(Self {
            core,
            rom_name,
            input: InputState::default(),
            window: None,
            pixels: None,
            frame_width,
            frame_height,
            frame_period: Duration::from_secs_f64(1.0 / frames_per_second),
            next_frame: Instant::now(),
            paused: false,
            fatal_error: None,
        })
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: &impl ToString) {
        self.fatal_error = Some(error.to_string());
        event_loop.exit();
    }

    fn create_window(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let logical_width = self.frame_width.saturating_mul(INITIAL_SCALE);
        let logical_height = self.frame_height.saturating_mul(INITIAL_SCALE);
        let attributes = Window::default_attributes()
            .with_title(format!(
                "VoxelBoy — {} — {}",
                self.rom_name,
                self.core.descriptor().display_name
            ))
            .with_inner_size(LogicalSize::new(logical_width, logical_height))
            .with_min_inner_size(LogicalSize::new(self.frame_width, self.frame_height));
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(|error| format!("could not create window: {error}"))?,
        );
        let size = window.inner_size();
        let surface = SurfaceTexture::new(size.width, size.height, Arc::clone(&window));
        let pixels = Pixels::new(self.frame_width, self.frame_height, surface)
            .map_err(|error| format!("could not initialize renderer: {error}"))?;
        self.window = Some(window);
        self.pixels = Some(pixels);
        Ok(())
    }

    fn run_due_frames(&mut self) -> Result<bool, String> {
        if self.paused {
            return Ok(false);
        }
        let now = Instant::now();
        let mut frames_run = 0;
        while now >= self.next_frame && frames_run < MAX_CATCH_UP_FRAMES {
            self.core
                .run_frame(self.input)
                .map_err(|error| error.to_string())?;
            self.next_frame += self.frame_period;
            frames_run += 1;
        }
        if frames_run == MAX_CATCH_UP_FRAMES && now >= self.next_frame {
            self.next_frame = now + self.frame_period;
        }
        Ok(frames_run != 0)
    }

    fn draw(&mut self) -> Result<(), String> {
        let frame = self
            .core
            .video_frame()
            .ok_or_else(|| "core did not produce a video frame".to_owned())?;
        frame.validate().map_err(|error| error.to_string())?;
        let pixels = self
            .pixels
            .as_mut()
            .ok_or_else(|| "renderer is not initialized".to_owned())?;
        if frame.width != self.frame_width || frame.height != self.frame_height {
            pixels
                .resize_buffer(frame.width, frame.height)
                .map_err(|error| format!("could not resize frame buffer: {error}"))?;
            self.frame_width = frame.width;
            self.frame_height = frame.height;
        }
        copy_frame(frame, pixels.frame_mut())?;
        pixels
            .render()
            .map_err(|error| format!("rendering failed: {error}"))
    }

    fn handle_key(&mut self, event_loop: &ActiveEventLoop, event: &KeyEvent) {
        let PhysicalKey::Code(code) = event.physical_key else {
            return;
        };
        let pressed = event.state == ElementState::Pressed;
        match code {
            KeyCode::Escape if pressed => event_loop.exit(),
            KeyCode::Space if pressed && !event.repeat => {
                self.paused = !self.paused;
                self.next_frame = Instant::now();
            }
            KeyCode::F2 if pressed && !event.repeat => {
                if let Err(error) = self.core.reset() {
                    self.fail(event_loop, &error);
                }
            }
            KeyCode::ArrowUp => self.input.up = pressed,
            KeyCode::ArrowDown => self.input.down = pressed,
            KeyCode::ArrowLeft => self.input.left = pressed,
            KeyCode::ArrowRight => self.input.right = pressed,
            KeyCode::KeyZ => self.input.a = pressed,
            KeyCode::KeyX => self.input.b = pressed,
            KeyCode::Enter => self.input.start = pressed,
            KeyCode::Backspace => self.input.select = pressed,
            KeyCode::KeyA => self.input.shoulder_left = pressed,
            KeyCode::KeyS => self.input.shoulder_right = pressed,
            _ => {}
        }
    }
}

impl ApplicationHandler for DesktopApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none()
            && let Err(error) = self.create_window(event_loop)
        {
            self.fail(event_loop, &error);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window.as_ref().map(|window| window.id()) != Some(window_id) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Focused(false) => self.input = InputState::default(),
            WindowEvent::KeyboardInput { event, .. } => self.handle_key(event_loop, &event),
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                if let Some(pixels) = self.pixels.as_mut()
                    && let Err(error) = pixels.resize_surface(size.width, size.height)
                {
                    self.fail(event_loop, &error);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.draw() {
                    self.fail(event_loop, &error);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        match self.run_due_frames() {
            Ok(true) => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            Ok(false) => {}
            Err(error) => {
                self.fail(event_loop, &error);
                return;
            }
        }
        if self.paused {
            event_loop.set_control_flow(ControlFlow::Wait);
        } else {
            event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(error) = &self.fatal_error {
            eprintln!("VoxelBoy stopped: {error}");
        }
    }
}

fn copy_frame(frame: VideoFrame<'_>, destination: &mut [u8]) -> Result<(), String> {
    let row_bytes = usize::try_from(frame.width)
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or_else(|| "frame row size overflow".to_owned())?;
    let height = usize::try_from(frame.height).map_err(|_| "frame height is too large")?;
    let required = row_bytes
        .checked_mul(height)
        .ok_or_else(|| "frame size overflow".to_owned())?;
    if destination.len() != required {
        return Err("renderer buffer has the wrong size".into());
    }
    for row in 0..height {
        let source_start = row * frame.pitch;
        let destination_start = row * row_bytes;
        destination[destination_start..destination_start + row_bytes]
            .copy_from_slice(&frame.pixels[source_start..source_start + row_bytes]);
    }
    Ok(())
}

fn detect_system(path: &Path, rom: &[u8]) -> Result<System, String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "gba" => Ok(System::GameBoyAdvance),
        "gbc" => Ok(System::GameBoyColor),
        "sgb" => Ok(System::SuperGameBoy),
        "gb" => {
            let color_flag = rom.get(0x143).copied().unwrap_or_default();
            if color_flag == 0x80 || color_flag == 0xc0 {
                Ok(System::GameBoyColor)
            } else {
                Ok(System::GameBoy)
            }
        }
        _ => Err(format!(
            "cannot detect system from extension '.{extension}'; use --system"
        )),
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let arguments = Arguments::parse();
    let rom = fs::read(&arguments.rom)?;
    let system = arguments
        .system
        .resolve(&arguments.rom, &rom)
        .map_err(|message| format!("invalid ROM selection: {message}"))?;
    let mut core = LibretroCore::load(&arguments.core, vec![system])?;
    core.load_game(GameImage {
        data: &rom,
        path: Some(&arguments.rom),
        system,
    })?;

    let rom_name = arguments
        .rom
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Unknown game")
        .to_owned();
    let mut app = DesktopApp::new(core, rom_name)?;
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut app)?;
    if let Some(error) = app.fatal_error {
        return Err(error.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{System, detect_system};

    #[test]
    fn detects_game_boy_variants() {
        let mut rom = vec![0_u8; 0x150];
        assert_eq!(
            detect_system(Path::new("game.gb"), &rom),
            Ok(System::GameBoy)
        );
        rom[0x143] = 0x80;
        assert_eq!(
            detect_system(Path::new("game.gb"), &rom),
            Ok(System::GameBoyColor)
        );
        assert_eq!(
            detect_system(Path::new("game.gbc"), &rom),
            Ok(System::GameBoyColor)
        );
    }

    #[test]
    fn detects_game_boy_advance() {
        assert_eq!(
            detect_system(Path::new("game.GBA"), &[]),
            Ok(System::GameBoyAdvance)
        );
    }

    #[test]
    fn requires_override_for_unknown_extension() {
        assert!(detect_system(Path::new("game.zip"), &[]).is_err());
    }
}
