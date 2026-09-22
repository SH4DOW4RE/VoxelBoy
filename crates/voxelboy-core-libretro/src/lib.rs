//! Safe `VoxelBoy` adapter around the libretro C ABI.

mod ffi;

use std::ffi::{CStr, CString, c_void};
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

use libloading::Library;
use voxelboy_core_api::{
    Capabilities, CoreDescriptor, CoreError, EmulatorCore, GameImage, InputState, PixelFormat,
    System, SystemTiming, VideoFrame,
};

use crate::ffi::{
    AudioSampleBatchCallback, AudioSampleCallback, EnvironmentCallback, InputPollCallback,
    InputStateCallback, RetroApiVersion, RetroDeinit, RetroGameInfo, RetroGetSystemAvInfo,
    RetroGetSystemInfo, RetroInit, RetroLoadGame, RetroReset, RetroRun, RetroSerialize,
    RetroSerializeSize, RetroSetAudioSample, RetroSetAudioSampleBatch, RetroSetEnvironment,
    RetroSetInputPoll, RetroSetInputState, RetroSetVideoRefresh, RetroSystemAvInfo,
    RetroSystemInfo, RetroUnloadGame, RetroUnserialize, VideoRefreshCallback,
};

static CORE_ACTIVE: AtomicBool = AtomicBool::new(false);
static CALLBACKS: LazyLock<Mutex<CallbackState>> =
    LazyLock::new(|| Mutex::new(CallbackState::default()));

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum RetroPixelFormat {
    #[default]
    Xrgb1555,
    Xrgb8888,
    Rgb565,
}

#[derive(Default)]
struct CallbackState {
    pixel_format: RetroPixelFormat,
    frame: Vec<u8>,
    width: u32,
    height: u32,
    pitch: usize,
    audio: Vec<i16>,
    input: InputState,
    error: Option<String>,
}

impl CallbackState {
    fn prepare_frame(&mut self, input: InputState) {
        self.input = input;
        self.audio.clear();
        self.error = None;
    }
}

struct Functions {
    api_version: RetroApiVersion,
    set_environment: RetroSetEnvironment,
    set_video_refresh: RetroSetVideoRefresh,
    set_audio_sample: RetroSetAudioSample,
    set_audio_sample_batch: RetroSetAudioSampleBatch,
    set_input_poll: RetroSetInputPoll,
    set_input_state: RetroSetInputState,
    init: RetroInit,
    deinit: RetroDeinit,
    get_system_info: RetroGetSystemInfo,
    get_system_av_info: RetroGetSystemAvInfo,
    reset: RetroReset,
    run: RetroRun,
    serialize_size: RetroSerializeSize,
    serialize: RetroSerialize,
    unserialize: RetroUnserialize,
    load_game: RetroLoadGame,
    unload_game: RetroUnloadGame,
}

impl Functions {
    unsafe fn load(library: &Library) -> Result<Self, CoreError> {
        Ok(Self {
            api_version: unsafe { symbol(library, b"retro_api_version\0")? },
            set_environment: unsafe { symbol(library, b"retro_set_environment\0")? },
            set_video_refresh: unsafe { symbol(library, b"retro_set_video_refresh\0")? },
            set_audio_sample: unsafe { symbol(library, b"retro_set_audio_sample\0")? },
            set_audio_sample_batch: unsafe { symbol(library, b"retro_set_audio_sample_batch\0")? },
            set_input_poll: unsafe { symbol(library, b"retro_set_input_poll\0")? },
            set_input_state: unsafe { symbol(library, b"retro_set_input_state\0")? },
            init: unsafe { symbol(library, b"retro_init\0")? },
            deinit: unsafe { symbol(library, b"retro_deinit\0")? },
            get_system_info: unsafe { symbol(library, b"retro_get_system_info\0")? },
            get_system_av_info: unsafe { symbol(library, b"retro_get_system_av_info\0")? },
            reset: unsafe { symbol(library, b"retro_reset\0")? },
            run: unsafe { symbol(library, b"retro_run\0")? },
            serialize_size: unsafe { symbol(library, b"retro_serialize_size\0")? },
            serialize: unsafe { symbol(library, b"retro_serialize\0")? },
            unserialize: unsafe { symbol(library, b"retro_unserialize\0")? },
            load_game: unsafe { symbol(library, b"retro_load_game\0")? },
            unload_game: unsafe { symbol(library, b"retro_unload_game\0")? },
        })
    }
}

unsafe fn symbol<T: Copy>(library: &Library, name: &[u8]) -> Result<T, CoreError> {
    let loaded = unsafe { library.get::<T>(name) }
        .map_err(|error| CoreError::Backend(format!("missing libretro symbol: {error}")))?;
    Ok(*loaded)
}

/// A dynamically loaded libretro core.
///
/// Libretro uses process-global callbacks, so only one instance may be active
/// in a process. A second call to [`Self::load`] fails until the first adapter
/// is dropped.
pub struct LibretroCore {
    // Kept before `functions` conceptually: function pointers are valid only
    // while the library remains loaded.
    _library: Library,
    functions: Functions,
    descriptor: CoreDescriptor,
    valid_extensions: Vec<String>,
    need_fullpath: bool,
    game_loaded: bool,
    timing: Option<SystemTiming>,
    frame: Vec<u8>,
    frame_width: u32,
    frame_height: u32,
    frame_pitch: usize,
    audio: Vec<i16>,
}

impl LibretroCore {
    /// Loads and initializes a dynamic libretro core.
    ///
    /// # Errors
    ///
    /// Returns an error if another core is active, the dynamic library or a
    /// required symbol cannot be loaded, or the ABI version is unsupported.
    pub fn load(path: &Path, systems: Vec<System>) -> Result<Self, CoreError> {
        if CORE_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(CoreError::Backend(
                "only one libretro core can be active at a time".into(),
            ));
        }

        match unsafe { Self::load_exclusive(path, systems) } {
            Ok(core) => Ok(core),
            Err(error) => {
                CORE_ACTIVE.store(false, Ordering::Release);
                Err(error)
            }
        }
    }

    unsafe fn load_exclusive(path: &Path, systems: Vec<System>) -> Result<Self, CoreError> {
        let library = unsafe { Library::new(path) }
            .map_err(|error| CoreError::Backend(format!("could not load core: {error}")))?;
        let functions = unsafe { Functions::load(&library) }?;

        let api_version = unsafe { (functions.api_version)() };
        if api_version != ffi::API_VERSION {
            return Err(CoreError::Backend(format!(
                "unsupported libretro API version {api_version}"
            )));
        }

        reset_callbacks();
        unsafe {
            (functions.set_environment)(environment_callback as EnvironmentCallback);
            (functions.set_video_refresh)(video_refresh_callback as VideoRefreshCallback);
            (functions.set_audio_sample)(audio_sample_callback as AudioSampleCallback);
            (functions.set_audio_sample_batch)(
                audio_sample_batch_callback as AudioSampleBatchCallback,
            );
            (functions.set_input_poll)(input_poll_callback as InputPollCallback);
            (functions.set_input_state)(input_state_callback as InputStateCallback);
            (functions.init)();
        }

        let mut info = RetroSystemInfo {
            library_name: ptr::null(),
            library_version: ptr::null(),
            valid_extensions: ptr::null(),
            need_fullpath: false,
            block_extract: false,
        };
        unsafe { (functions.get_system_info)(&raw mut info) };

        let name = unsafe { optional_c_string(info.library_name) }
            .unwrap_or_else(|| "Unknown libretro core".into());
        let version = unsafe { optional_c_string(info.library_version) }.unwrap_or_default();
        let extensions = unsafe { optional_c_string(info.valid_extensions) }
            .unwrap_or_default()
            .split('|')
            .filter(|extension| !extension.is_empty())
            .map(str::to_owned)
            .collect();
        let id = identifier(&name);

        Ok(Self {
            _library: library,
            functions,
            descriptor: CoreDescriptor {
                id,
                display_name: name,
                version,
                systems,
                capabilities: Capabilities {
                    save_states: true,
                    persistent_memory: false,
                    semantic_scene: false,
                },
            },
            valid_extensions: extensions,
            need_fullpath: info.need_fullpath,
            game_loaded: false,
            timing: None,
            frame: Vec::new(),
            frame_width: 0,
            frame_height: 0,
            frame_pitch: 0,
            audio: Vec::new(),
        })
    }

    #[must_use]
    pub fn valid_extensions(&self) -> &[String] {
        &self.valid_extensions
    }

    #[must_use]
    pub const fn needs_full_path(&self) -> bool {
        self.need_fullpath
    }

    fn collect_callbacks(&mut self) -> Result<(), CoreError> {
        let mut callbacks = CALLBACKS
            .lock()
            .map_err(|_| CoreError::Backend("libretro callback state was poisoned".into()))?;
        if let Some(error) = callbacks.error.take() {
            return Err(CoreError::Backend(error));
        }
        self.frame = std::mem::take(&mut callbacks.frame);
        self.frame_width = callbacks.width;
        self.frame_height = callbacks.height;
        self.frame_pitch = callbacks.pitch;
        self.audio = std::mem::take(&mut callbacks.audio);
        Ok(())
    }
}

impl EmulatorCore for LibretroCore {
    fn descriptor(&self) -> &CoreDescriptor {
        &self.descriptor
    }

    fn load_game(&mut self, game: GameImage<'_>) -> Result<System, CoreError> {
        if self.game_loaded {
            self.unload_game();
        }
        if !self.descriptor.systems.contains(&game.system) {
            return Err(CoreError::InvalidRom(format!(
                "core does not advertise support for {:?}",
                game.system
            )));
        }

        let path = game
            .path
            .map(path_to_c_string)
            .transpose()?
            .unwrap_or_default();
        if self.need_fullpath && path.as_bytes().is_empty() {
            return Err(CoreError::InvalidRom(
                "this core requires a filesystem path".into(),
            ));
        }

        let info = RetroGameInfo {
            path: if path.as_bytes().is_empty() {
                ptr::null()
            } else {
                path.as_ptr()
            },
            data: if self.need_fullpath {
                ptr::null()
            } else {
                game.data.as_ptr().cast::<c_void>()
            },
            size: if self.need_fullpath {
                0
            } else {
                game.data.len()
            },
            meta: ptr::null(),
        };

        if !unsafe { (self.functions.load_game)(&raw const info) } {
            return Err(CoreError::InvalidRom(
                "the libretro core rejected the game".into(),
            ));
        }
        self.game_loaded = true;

        let mut av_info = RetroSystemAvInfo::default();
        unsafe { (self.functions.get_system_av_info)(&raw mut av_info) };
        self.timing = Some(SystemTiming {
            frames_per_second: av_info.timing.fps,
            audio_sample_rate: av_info.timing.sample_rate,
        });
        self.descriptor.capabilities.save_states = unsafe { (self.functions.serialize_size)() > 0 };
        Ok(game.system)
    }

    fn unload_game(&mut self) {
        if self.game_loaded {
            unsafe { (self.functions.unload_game)() };
            self.game_loaded = false;
            self.timing = None;
            self.frame.clear();
            self.audio.clear();
            reset_callbacks();
        }
    }

    fn reset(&mut self) -> Result<(), CoreError> {
        if !self.game_loaded {
            return Err(CoreError::NotLoaded);
        }
        unsafe { (self.functions.reset)() };
        Ok(())
    }

    fn run_frame(&mut self, input: InputState) -> Result<(), CoreError> {
        if !self.game_loaded {
            return Err(CoreError::NotLoaded);
        }
        {
            let mut callbacks = CALLBACKS
                .lock()
                .map_err(|_| CoreError::Backend("libretro callback state was poisoned".into()))?;
            callbacks.prepare_frame(input);
        }
        unsafe { (self.functions.run)() };
        self.collect_callbacks()
    }

    fn video_frame(&self) -> Option<VideoFrame<'_>> {
        (!self.frame.is_empty()).then_some(VideoFrame {
            pixels: &self.frame,
            width: self.frame_width,
            height: self.frame_height,
            pitch: self.frame_pitch,
            format: PixelFormat::Rgba8888,
        })
    }

    fn audio_samples(&self) -> &[i16] {
        &self.audio
    }

    fn system_timing(&self) -> Option<SystemTiming> {
        self.timing
    }

    fn save_state(&self) -> Result<Vec<u8>, CoreError> {
        if !self.game_loaded {
            return Err(CoreError::NotLoaded);
        }
        let size = unsafe { (self.functions.serialize_size)() };
        if size == 0 {
            return Err(CoreError::Unsupported("save states"));
        }
        let mut state = vec![0_u8; size];
        if !unsafe { (self.functions.serialize)(state.as_mut_ptr().cast::<c_void>(), size) } {
            return Err(CoreError::Backend(
                "core failed to create save state".into(),
            ));
        }
        Ok(state)
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), CoreError> {
        if !self.game_loaded {
            return Err(CoreError::NotLoaded);
        }
        if !unsafe { (self.functions.unserialize)(state.as_ptr().cast::<c_void>(), state.len()) } {
            return Err(CoreError::Backend("core rejected save state".into()));
        }
        Ok(())
    }
}

impl Drop for LibretroCore {
    fn drop(&mut self) {
        self.unload_game();
        unsafe { (self.functions.deinit)() };
        reset_callbacks();
        CORE_ACTIVE.store(false, Ordering::Release);
    }
}

fn reset_callbacks() {
    if let Ok(mut callbacks) = CALLBACKS.lock() {
        *callbacks = CallbackState::default();
    }
}

unsafe fn optional_c_string(value: *const std::ffi::c_char) -> Option<String> {
    (!value.is_null()).then(|| {
        unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned()
    })
}

fn path_to_c_string(path: &Path) -> Result<CString, CoreError> {
    CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| CoreError::InvalidRom("game path contains a null byte".into()))
}

fn identifier(name: &str) -> String {
    let id: String = name
        .chars()
        .flat_map(char::to_lowercase)
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect();
    id.trim_matches('-').to_owned()
}

unsafe extern "C" fn environment_callback(command: u32, data: *mut c_void) -> bool {
    match command {
        ffi::ENVIRONMENT_GET_CAN_DUPE => {
            if data.is_null() {
                return false;
            }
            unsafe { data.cast::<bool>().write(true) };
            true
        }
        ffi::ENVIRONMENT_SET_PIXEL_FORMAT => {
            if data.is_null() {
                return false;
            }
            let value = unsafe { data.cast::<i32>().read() };
            let format = match value {
                0 => RetroPixelFormat::Xrgb1555,
                1 => RetroPixelFormat::Xrgb8888,
                2 => RetroPixelFormat::Rgb565,
                _ => return false,
            };
            if let Ok(mut callbacks) = CALLBACKS.lock() {
                callbacks.pixel_format = format;
                true
            } else {
                false
            }
        }
        ffi::ENVIRONMENT_SET_SUPPORT_NO_GAME | ffi::ENVIRONMENT_GET_INPUT_BITMASKS => true,
        _ => false,
    }
}

unsafe extern "C" fn video_refresh_callback(
    data: *const c_void,
    width: u32,
    height: u32,
    source_pitch: usize,
) {
    if data.is_null() {
        return;
    }
    let Ok(mut callbacks) = CALLBACKS.lock() else {
        return;
    };
    if let Err(error) = copy_video_frame(&mut callbacks, data, width, height, source_pitch) {
        callbacks.error = Some(error);
    }
}

fn copy_video_frame(
    state: &mut CallbackState,
    data: *const c_void,
    width: u32,
    height: u32,
    source_pitch: usize,
) -> Result<(), String> {
    let width = usize::try_from(width).map_err(|_| "video width is too large")?;
    let height = usize::try_from(height).map_err(|_| "video height is too large")?;
    let source_bytes_per_pixel = match state.pixel_format {
        RetroPixelFormat::Xrgb8888 => 4,
        RetroPixelFormat::Xrgb1555 | RetroPixelFormat::Rgb565 => 2,
    };
    let source_row_bytes = width
        .checked_mul(source_bytes_per_pixel)
        .ok_or("video row size overflow")?;
    if source_pitch < source_row_bytes {
        return Err("core supplied a video pitch shorter than one row".into());
    }
    let source_len = source_pitch
        .checked_mul(height)
        .ok_or("video buffer size overflow")?;
    let destination_pitch = width.checked_mul(4).ok_or("video row size overflow")?;
    let destination_len = destination_pitch
        .checked_mul(height)
        .ok_or("video buffer size overflow")?;
    let source = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), source_len) };
    state.frame.resize(destination_len, 0);

    for y in 0..height {
        for x in 0..width {
            let source_offset = y * source_pitch + x * source_bytes_per_pixel;
            let destination_offset = y * destination_pitch + x * 4;
            let [red, green, blue] = decode_pixel(
                state.pixel_format,
                &source[source_offset..source_offset + source_bytes_per_pixel],
            );
            state.frame[destination_offset..destination_offset + 4].copy_from_slice(&[
                red,
                green,
                blue,
                u8::MAX,
            ]);
        }
    }

    state.width = u32::try_from(width).map_err(|_| "video width is too large")?;
    state.height = u32::try_from(height).map_err(|_| "video height is too large")?;
    state.pitch = destination_pitch;
    Ok(())
}

fn decode_pixel(format: RetroPixelFormat, bytes: &[u8]) -> [u8; 3] {
    match format {
        RetroPixelFormat::Xrgb8888 => {
            let value = u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            [
                ((value >> 16) & 0xff) as u8,
                ((value >> 8) & 0xff) as u8,
                (value & 0xff) as u8,
            ]
        }
        RetroPixelFormat::Xrgb1555 => {
            let value = u16::from_ne_bytes([bytes[0], bytes[1]]);
            [
                expand_five_bit(((value >> 10) & 0x1f) as u8),
                expand_five_bit(((value >> 5) & 0x1f) as u8),
                expand_five_bit((value & 0x1f) as u8),
            ]
        }
        RetroPixelFormat::Rgb565 => {
            let value = u16::from_ne_bytes([bytes[0], bytes[1]]);
            let red = ((value >> 11) & 0x1f) as u8;
            let green = ((value >> 5) & 0x3f) as u8;
            let blue = (value & 0x1f) as u8;
            [
                expand_five_bit(red),
                (green << 2) | (green >> 4),
                expand_five_bit(blue),
            ]
        }
    }
}

const fn expand_five_bit(value: u8) -> u8 {
    (value << 3) | (value >> 2)
}

unsafe extern "C" fn audio_sample_callback(left: i16, right: i16) {
    if let Ok(mut callbacks) = CALLBACKS.lock() {
        callbacks.audio.extend_from_slice(&[left, right]);
    }
}

unsafe extern "C" fn audio_sample_batch_callback(data: *const i16, frames: usize) -> usize {
    let Some(sample_count) = frames.checked_mul(2) else {
        return 0;
    };
    if sample_count == 0 {
        return frames;
    }
    if data.is_null() && sample_count != 0 {
        return 0;
    }
    let samples = unsafe { std::slice::from_raw_parts(data, sample_count) };
    if let Ok(mut callbacks) = CALLBACKS.lock() {
        callbacks.audio.extend_from_slice(samples);
        frames
    } else {
        0
    }
}

unsafe extern "C" fn input_poll_callback() {}

unsafe extern "C" fn input_state_callback(port: u32, device: u32, index: u32, id: u32) -> i16 {
    if port != 0 || device & ffi::DEVICE_MASK != ffi::DEVICE_JOYPAD || index != 0 {
        return 0;
    }
    let Ok(callbacks) = CALLBACKS.lock() else {
        return 0;
    };
    let mask = input_mask(callbacks.input);
    if id == ffi::JOYPAD_MASK {
        mask.cast_signed()
    } else {
        i16::from(id < u16::BITS && mask & (1_u16 << id) != 0)
    }
}

fn input_mask(input: InputState) -> u16 {
    let mut mask = 0_u16;
    let buttons = [
        (ffi::JOYPAD_B, input.b),
        (ffi::JOYPAD_SELECT, input.select),
        (ffi::JOYPAD_START, input.start),
        (ffi::JOYPAD_UP, input.up),
        (ffi::JOYPAD_DOWN, input.down),
        (ffi::JOYPAD_LEFT, input.left),
        (ffi::JOYPAD_RIGHT, input.right),
        (ffi::JOYPAD_A, input.a),
        (ffi::JOYPAD_L, input.shoulder_left),
        (ffi::JOYPAD_R, input.shoulder_right),
    ];
    for (id, pressed) in buttons {
        if pressed {
            mask |= 1_u16 << id;
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::{RetroPixelFormat, decode_pixel, identifier, input_mask};
    use voxelboy_core_api::InputState;

    #[test]
    fn sanitizes_core_identifier() {
        assert_eq!(identifier("SameBoy 1.2"), "sameboy-1-2");
    }

    #[test]
    fn decodes_libretro_pixel_formats() {
        assert_eq!(
            decode_pixel(RetroPixelFormat::Xrgb8888, &0x00ff_0000_u32.to_ne_bytes()),
            [255, 0, 0]
        );
        assert_eq!(
            decode_pixel(RetroPixelFormat::Xrgb1555, &0x7c00_u16.to_ne_bytes()),
            [255, 0, 0]
        );
        assert_eq!(
            decode_pixel(RetroPixelFormat::Rgb565, &0x07e0_u16.to_ne_bytes()),
            [0, 255, 0]
        );
    }

    #[test]
    fn maps_named_input_to_retropad_bits() {
        let input = InputState {
            a: true,
            start: true,
            ..InputState::default()
        };
        let mask = input_mask(input);
        assert_ne!(mask & (1 << 8), 0);
        assert_ne!(mask & (1 << 3), 0);
        assert_eq!(mask.count_ones(), 2);
    }
}
