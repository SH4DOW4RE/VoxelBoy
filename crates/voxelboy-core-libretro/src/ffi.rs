//! Minimal libretro ABI declarations used by the adapter.

use std::ffi::{c_char, c_void};

pub const API_VERSION: u32 = 1;

pub const ENVIRONMENT_GET_CAN_DUPE: u32 = 3;
pub const ENVIRONMENT_SET_PIXEL_FORMAT: u32 = 10;
pub const ENVIRONMENT_SET_SUPPORT_NO_GAME: u32 = 18;
pub const ENVIRONMENT_GET_INPUT_BITMASKS: u32 = 0x33 | 0x1_0000;

pub const DEVICE_JOYPAD: u32 = 1;
pub const DEVICE_MASK: u32 = 0xff;
pub const JOYPAD_B: u32 = 0;
pub const JOYPAD_SELECT: u32 = 2;
pub const JOYPAD_START: u32 = 3;
pub const JOYPAD_UP: u32 = 4;
pub const JOYPAD_DOWN: u32 = 5;
pub const JOYPAD_LEFT: u32 = 6;
pub const JOYPAD_RIGHT: u32 = 7;
pub const JOYPAD_A: u32 = 8;
pub const JOYPAD_L: u32 = 10;
pub const JOYPAD_R: u32 = 11;
pub const JOYPAD_MASK: u32 = 256;

#[repr(C)]
pub struct RetroSystemInfo {
    pub library_name: *const c_char,
    pub library_version: *const c_char,
    pub valid_extensions: *const c_char,
    pub need_fullpath: bool,
    pub block_extract: bool,
}

#[repr(C)]
pub struct RetroGameInfo {
    pub path: *const c_char,
    pub data: *const c_void,
    pub size: usize,
    pub meta: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RetroGameGeometry {
    pub base_width: u32,
    pub base_height: u32,
    pub max_width: u32,
    pub max_height: u32,
    pub aspect_ratio: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RetroSystemTiming {
    pub fps: f64,
    pub sample_rate: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RetroSystemAvInfo {
    pub geometry: RetroGameGeometry,
    pub timing: RetroSystemTiming,
}

pub type EnvironmentCallback = unsafe extern "C" fn(u32, *mut c_void) -> bool;
pub type VideoRefreshCallback = unsafe extern "C" fn(*const c_void, u32, u32, usize);
pub type AudioSampleCallback = unsafe extern "C" fn(i16, i16);
pub type AudioSampleBatchCallback = unsafe extern "C" fn(*const i16, usize) -> usize;
pub type InputPollCallback = unsafe extern "C" fn();
pub type InputStateCallback = unsafe extern "C" fn(u32, u32, u32, u32) -> i16;

pub type RetroApiVersion = unsafe extern "C" fn() -> u32;
pub type RetroSetEnvironment = unsafe extern "C" fn(EnvironmentCallback);
pub type RetroSetVideoRefresh = unsafe extern "C" fn(VideoRefreshCallback);
pub type RetroSetAudioSample = unsafe extern "C" fn(AudioSampleCallback);
pub type RetroSetAudioSampleBatch = unsafe extern "C" fn(AudioSampleBatchCallback);
pub type RetroSetInputPoll = unsafe extern "C" fn(InputPollCallback);
pub type RetroSetInputState = unsafe extern "C" fn(InputStateCallback);
pub type RetroInit = unsafe extern "C" fn();
pub type RetroDeinit = unsafe extern "C" fn();
pub type RetroGetSystemInfo = unsafe extern "C" fn(*mut RetroSystemInfo);
pub type RetroGetSystemAvInfo = unsafe extern "C" fn(*mut RetroSystemAvInfo);
pub type RetroReset = unsafe extern "C" fn();
pub type RetroRun = unsafe extern "C" fn();
pub type RetroSerializeSize = unsafe extern "C" fn() -> usize;
pub type RetroSerialize = unsafe extern "C" fn(*mut c_void, usize) -> bool;
pub type RetroUnserialize = unsafe extern "C" fn(*const c_void, usize) -> bool;
pub type RetroLoadGame = unsafe extern "C" fn(*const RetroGameInfo) -> bool;
pub type RetroUnloadGame = unsafe extern "C" fn();
