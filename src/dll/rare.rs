use crate::{
    dll::win32::{ApiHookResult, Win32Context},
    helper::UnicornHelper,
};
use cpal::{
    FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use minimp3::{Decoder as Mp3Decoder, Error as Mp3Error};
use std::{
    collections::HashMap,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::Ordering},
};
use unicorn_engine::{RegisterX86, Unicorn};

const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_BAD_FORMAT: u32 = 11;
const ERROR_NOT_READY: u32 = 21;

const RARE_CONTEXT_SIZE: usize = 8;
const RARE_SOUND_SIZE: usize = 8;

const RARE_CREATE_CONTEXT: &str = "?Rare_CreateContext@@YAPAVRContext@@XZ";
const RARE_DESTROY_CONTEXT: &str = "?Rare_DestroyContext@@YAXPAVRContext@@@Z";
const RARE_SET_SOUND_NOTIFY: &str = "?Rare_SetSoundNotify@@YAXPAVRContext@@PAVRNotify@@@Z";
const RARE_OPEN_SOUND: &str = "?Rare_OpenSound@@YAPAVRSound@@PAVRContext@@PADW4Rare_Open@@@Z";
const RARE_CLOSE_SOUND: &str = "?Rare_CloseSound@@YAXPAVRSound@@@Z";
const RARE_PLAY_SOUND: &str = "?Rare_PlaySound@@YAXPAVRSound@@@Z";
const RARE_STOP_SOUND: &str = "?Rare_StopSound@@YAXPAVRSound@@@Z";
const RARE_RESET_SOUND: &str = "?Rare_ResetSound@@YAXPAVRSound@@@Z";
const RARE_PAUSE_SOUND: &str = "?Rare_PauseSound@@YAXPAVRSound@@@Z";
const RARE_IS_PLAY_SOUND: &str = "?Rare_IsPlaySound@@YAHPAVRSound@@@Z";
const RARE_SET_FREQUENCY: &str = "?Rare_SetFrequency@@YAXPAVRSound@@J@Z";
const RARE_GET_FREQUENCY: &str = "?Rare_GetFrequency@@YAJPAVRSound@@@Z";
const RARE_SET_PAN: &str = "?Rare_SetPan@@YAXPAVRSound@@J@Z";
const RARE_GET_PAN: &str = "?Rare_GetPan@@YAJPAVRSound@@@Z";
const RARE_SET_VOLUME: &str = "?Rare_SetVolume@@YAXPAVRSound@@J@Z";
const RARE_GET_VOLUME: &str = "?Rare_GetVolume@@YAJPAVRSound@@@Z";
const RARE_SET_REPEAT: &str = "?Rare_SetRepeat@@YAXPAVRSound@@H@Z";
const RARE_IS_REPEAT: &str = "?Rare_IsRepeat@@YAHPAVRSound@@@Z";

const METHOD_CONTEXT_DESTROY: &str = "__RContext_Destroy";
const METHOD_CONTEXT_INIT: &str = "__RContext_Init";
const METHOD_CONTEXT_OPEN_SOUND: &str = "__RContext_OpenSound";
const METHOD_CONTEXT_SET_NOTIFY: &str = "__RContext_SetSoundNotify";

const METHOD_SOUND_DESTROY: &str = "__RSound_Destroy";
const METHOD_SOUND_PLAY: &str = "__RSound_Play";
const METHOD_SOUND_STOP: &str = "__RSound_Stop";
const METHOD_SOUND_RESET: &str = "__RSound_Reset";
const METHOD_SOUND_PAUSE: &str = "__RSound_Pause";
const METHOD_SOUND_IS_PLAYING: &str = "__RSound_IsPlaying";
const METHOD_SOUND_SET_FREQUENCY: &str = "__RSound_SetFrequency";
const METHOD_SOUND_GET_FREQUENCY: &str = "__RSound_GetFrequency";
const METHOD_SOUND_SET_PAN: &str = "__RSound_SetPan";
const METHOD_SOUND_GET_PAN: &str = "__RSound_GetPan";
const METHOD_SOUND_SET_VOLUME: &str = "__RSound_SetVolume";
const METHOD_SOUND_GET_VOLUME: &str = "__RSound_GetVolume";
const METHOD_SOUND_SET_REPEAT: &str = "__RSound_SetRepeat";
const METHOD_SOUND_IS_REPEAT: &str = "__RSound_IsRepeat";

const CONTEXT_VTABLE_KEY: &str = "__RContext_VTable";
const SOUND_VTABLE_KEY: &str = "__RSound_VTable";

/// Rare.dll 프록시가 관리하는 오디오 출력 엔진입니다.
pub(crate) struct RareAudioEngine {
    _stream: Stream,
}

/// Rare.dll의 컨텍스트 객체 상태입니다.
#[derive(Debug, Clone, Default)]
pub(crate) struct RareContextState {
    /// 재생 완료 통지에 사용할 게스트 객체 포인터입니다.
    pub notify_ptr: u32,
}

/// Rare.dll이 재생하는 WAV 소스 데이터입니다.
#[derive(Debug, Clone)]
pub(crate) struct RareWaveData {
    channels: usize,
    sample_rate: u32,
    samples: Vec<f32>,
}

#[derive(Debug, Clone)]
enum ResolvedAudioSource {
    File(PathBuf),
    Packed { display_name: String, data: Vec<u8> },
}

/// Rare.dll의 사운드 객체 상태입니다.
#[derive(Debug, Clone)]
pub(crate) struct RareSoundState {
    /// 이 사운드를 생성한 컨텍스트 객체 포인터입니다.
    pub context_ptr: u32,
    /// 원본 파일 경로입니다.
    pub path: String,
    /// 디코딩된 소스 데이터입니다.
    pub wave: Arc<RareWaveData>,
    /// Rare_SetVolume에 전달된 원시 값입니다.
    pub volume: i32,
    /// Rare_SetPan에 전달된 원시 값입니다.
    pub pan: i32,
    /// Rare_SetFrequency에 전달된 원시 값입니다.
    pub frequency: i32,
    /// 반복 재생 여부입니다.
    pub repeat: bool,
    /// 현재 재생 중인지 여부입니다.
    pub playing: bool,
    /// 일시정지 상태인지 여부입니다.
    pub paused: bool,
    /// 현재 재생 위치(프레임 단위)입니다.
    pub position_frames: f64,
}

impl RareSoundState {
    fn frame_len(&self) -> usize {
        self.wave.samples.len() / self.wave.channels.max(1)
    }

    fn playback_step(&self, output_sample_rate: u32) -> f64 {
        let requested = if self.frequency > 0 {
            self.frequency as u32
        } else {
            self.wave.sample_rate
        };
        (requested as f64 / output_sample_rate.max(1) as f64).max(0.01)
    }

    fn volume_gain(&self) -> f32 {
        if (1..=100).contains(&self.volume) {
            return self.volume as f32 / 100.0;
        }
        if (-10_000..=0).contains(&self.volume) {
            return 10f32.powf(self.volume as f32 / 2000.0);
        }
        if self.volume == 0 {
            return 1.0;
        }
        (self.volume as f32 / 100.0).clamp(0.0, 4.0)
    }

    fn channel_gains(&self) -> (f32, f32) {
        if self.pan == 0 {
            return (1.0, 1.0);
        }
        let pan = (self.pan as f32 / 10_000.0).clamp(-1.0, 1.0);
        if pan >= 0.0 {
            (1.0 - pan, 1.0)
        } else {
            (1.0, 1.0 + pan)
        }
    }

    fn current_frame(&self, frame_index: usize) -> (f32, f32) {
        let src_channels = self.wave.channels.max(1);
        let src_offset = frame_index * src_channels;
        let left = self.wave.samples.get(src_offset).copied().unwrap_or(0.0);
        let right = if src_channels >= 2 {
            self.wave
                .samples
                .get(src_offset + 1)
                .copied()
                .unwrap_or(left)
        } else {
            left
        };
        (left, right)
    }
}

/// Rare.dll의 사운드 API를 호스트 오디오로 연결하는 프록시 모듈입니다.
pub struct Rare;

impl Rare {
    /// Rare.dll의 프록시 익스포트 주소를 해소합니다.
    pub(crate) fn resolve_export(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<u32> {
        let symbol = match normalize_export_name(func_name) {
            Some(symbol) => symbol,
            None => return None,
        };
        Some(Self::register_proxy_symbol(uc, symbol))
    }

    /// 함수명 기준 `Rare.dll` API 구현체를 선택합니다.
    pub fn handle(uc: &mut Unicorn<Win32Context>, func_name: &str) -> Option<ApiHookResult> {
        match func_name {
            RARE_CREATE_CONTEXT | "Rare_CreateContext" => Self::rare_create_context(uc),
            RARE_DESTROY_CONTEXT | "Rare_DestroyContext" => Self::rare_destroy_context(uc),
            RARE_SET_SOUND_NOTIFY | "Rare_SetSoundNotify" => Self::rare_set_sound_notify(uc),
            RARE_OPEN_SOUND | "Rare_OpenSound" => Self::rare_open_sound(uc),
            RARE_CLOSE_SOUND | "Rare_CloseSound" => Self::rare_close_sound(uc),
            RARE_PLAY_SOUND | "Rare_PlaySound" => Self::rare_play_sound(uc),
            RARE_STOP_SOUND | "Rare_StopSound" => Self::rare_stop_sound(uc),
            RARE_RESET_SOUND | "Rare_ResetSound" => Self::rare_reset_sound(uc),
            RARE_PAUSE_SOUND | "Rare_PauseSound" => Self::rare_pause_sound(uc),
            RARE_IS_PLAY_SOUND | "Rare_IsPlaySound" => Self::rare_is_play_sound(uc),
            RARE_SET_FREQUENCY | "Rare_SetFrequency" => Self::rare_set_frequency(uc),
            RARE_GET_FREQUENCY | "Rare_GetFrequency" => Self::rare_get_frequency(uc),
            RARE_SET_PAN | "Rare_SetPan" => Self::rare_set_pan(uc),
            RARE_GET_PAN | "Rare_GetPan" => Self::rare_get_pan(uc),
            RARE_SET_VOLUME | "Rare_SetVolume" => Self::rare_set_volume(uc),
            RARE_GET_VOLUME | "Rare_GetVolume" => Self::rare_get_volume(uc),
            RARE_SET_REPEAT | "Rare_SetRepeat" => Self::rare_set_repeat(uc),
            RARE_IS_REPEAT | "Rare_IsRepeat" => Self::rare_is_repeat(uc),
            METHOD_CONTEXT_DESTROY => Self::context_destroy_method(uc),
            METHOD_CONTEXT_INIT => Self::context_init_method(uc),
            METHOD_CONTEXT_OPEN_SOUND => Self::context_open_sound_method(uc),
            METHOD_CONTEXT_SET_NOTIFY => Self::context_set_notify_method(uc),
            METHOD_SOUND_DESTROY => Self::sound_destroy_method(uc),
            METHOD_SOUND_PLAY => Self::sound_play_method(uc),
            METHOD_SOUND_STOP => Self::sound_stop_method(uc),
            METHOD_SOUND_RESET => Self::sound_reset_method(uc),
            METHOD_SOUND_PAUSE => Self::sound_pause_method(uc),
            METHOD_SOUND_IS_PLAYING => Self::sound_is_playing_method(uc),
            METHOD_SOUND_SET_FREQUENCY => Self::sound_set_frequency_method(uc),
            METHOD_SOUND_GET_FREQUENCY => Self::sound_get_frequency_method(uc),
            METHOD_SOUND_SET_PAN => Self::sound_set_pan_method(uc),
            METHOD_SOUND_GET_PAN => Self::sound_get_pan_method(uc),
            METHOD_SOUND_SET_VOLUME => Self::sound_set_volume_method(uc),
            METHOD_SOUND_GET_VOLUME => Self::sound_get_volume_method(uc),
            METHOD_SOUND_SET_REPEAT => Self::sound_set_repeat_method(uc),
            METHOD_SOUND_IS_REPEAT => Self::sound_is_repeat_method(uc),
            _ => {
                crate::emu_log!("[!] Rare.dll Unhandled: {}", func_name);
                None
            }
        }
    }

    fn rare_create_context(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let context_ptr = Self::create_context_object(uc);
        crate::emu_log!(
            "[RARE] Rare_CreateContext() -> RContext* {:#x}",
            context_ptr
        );
        Some(ApiHookResult::caller(Some(context_ptr as i32)))
    }

    fn rare_destroy_context(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let context_ptr = uc.read_arg(0);
        Self::destroy_context(uc, context_ptr);
        crate::emu_log!("[RARE] Rare_DestroyContext({:#x})", context_ptr);
        Some(ApiHookResult::caller(None))
    }

    fn rare_set_sound_notify(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let context_ptr = uc.read_arg(0);
        let notify_ptr = uc.read_arg(1);
        Self::set_context_notify(uc, context_ptr, notify_ptr);
        crate::emu_log!(
            "[RARE] Rare_SetSoundNotify({:#x}, {:#x})",
            context_ptr,
            notify_ptr
        );
        Some(ApiHookResult::caller(None))
    }

    fn rare_open_sound(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let context_ptr = uc.read_arg(0);
        let filename_ptr = uc.read_arg(1);
        let open_mode = uc.read_arg(2);
        let sound_ptr = Self::open_sound_for_context(uc, context_ptr, filename_ptr, open_mode);
        crate::emu_log!(
            "[RARE] Rare_OpenSound({:#x}, {:#x}, {}) -> RSound* {:#x}",
            context_ptr,
            filename_ptr,
            open_mode,
            sound_ptr
        );
        Some(ApiHookResult::caller(Some(sound_ptr as i32)))
    }

    fn rare_close_sound(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = uc.read_arg(0);
        Self::destroy_sound(uc, sound_ptr);
        crate::emu_log!("[RARE] Rare_CloseSound({:#x})", sound_ptr);
        Some(ApiHookResult::caller(None))
    }

    fn rare_play_sound(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = uc.read_arg(0);
        Self::set_sound_playing(uc, sound_ptr, true, false, false);
        crate::emu_log!("[RARE] Rare_PlaySound({:#x})", sound_ptr);
        Some(ApiHookResult::caller(None))
    }

    fn rare_stop_sound(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = uc.read_arg(0);
        Self::stop_sound(uc, sound_ptr);
        crate::emu_log!("[RARE] Rare_StopSound({:#x})", sound_ptr);
        Some(ApiHookResult::caller(None))
    }

    fn rare_reset_sound(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = uc.read_arg(0);
        Self::reset_sound_position(uc, sound_ptr);
        crate::emu_log!("[RARE] Rare_ResetSound({:#x})", sound_ptr);
        Some(ApiHookResult::caller(None))
    }

    fn rare_pause_sound(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = uc.read_arg(0);
        Self::set_sound_playing(uc, sound_ptr, false, true, false);
        crate::emu_log!("[RARE] Rare_PauseSound({:#x})", sound_ptr);
        Some(ApiHookResult::caller(None))
    }

    fn rare_is_play_sound(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = uc.read_arg(0);
        let value = Self::sound_is_playing_value(uc, sound_ptr);
        crate::emu_log!("[RARE] Rare_IsPlaySound({:#x}) -> int {}", sound_ptr, value);
        Some(ApiHookResult::caller(Some(value)))
    }

    fn rare_set_frequency(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = uc.read_arg(0);
        let value = uc.read_arg(1) as i32;
        Self::set_sound_frequency(uc, sound_ptr, value);
        crate::emu_log!("[RARE] Rare_SetFrequency({:#x}, {})", sound_ptr, value);
        Some(ApiHookResult::caller(None))
    }

    fn rare_get_frequency(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = uc.read_arg(0);
        let value = Self::sound_frequency_value(uc, sound_ptr);
        crate::emu_log!(
            "[RARE] Rare_GetFrequency({:#x}) -> long {}",
            sound_ptr,
            value
        );
        Some(ApiHookResult::caller(Some(value)))
    }

    fn rare_set_pan(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = uc.read_arg(0);
        let value = uc.read_arg(1) as i32;
        Self::set_sound_pan(uc, sound_ptr, value);
        crate::emu_log!("[RARE] Rare_SetPan({:#x}, {})", sound_ptr, value);
        Some(ApiHookResult::caller(None))
    }

    fn rare_get_pan(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = uc.read_arg(0);
        let value = Self::sound_pan_value(uc, sound_ptr);
        crate::emu_log!("[RARE] Rare_GetPan({:#x}) -> long {}", sound_ptr, value);
        Some(ApiHookResult::caller(Some(value)))
    }

    fn rare_set_volume(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = uc.read_arg(0);
        let value = uc.read_arg(1) as i32;
        Self::set_sound_volume(uc, sound_ptr, value);
        crate::emu_log!("[RARE] Rare_SetVolume({:#x}, {})", sound_ptr, value);
        Some(ApiHookResult::caller(None))
    }

    fn rare_get_volume(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = uc.read_arg(0);
        let value = Self::sound_volume_value(uc, sound_ptr);
        crate::emu_log!("[RARE] Rare_GetVolume({:#x}) -> long {}", sound_ptr, value);
        Some(ApiHookResult::caller(Some(value)))
    }

    fn rare_set_repeat(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = uc.read_arg(0);
        let repeat = uc.read_arg(1) != 0;
        Self::set_sound_repeat(uc, sound_ptr, repeat);
        crate::emu_log!(
            "[RARE] Rare_SetRepeat({:#x}, {})",
            sound_ptr,
            i32::from(repeat)
        );
        Some(ApiHookResult::caller(None))
    }

    fn rare_is_repeat(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = uc.read_arg(0);
        let value = Self::sound_is_repeat_value(uc, sound_ptr);
        crate::emu_log!("[RARE] Rare_IsRepeat({:#x}) -> int {}", sound_ptr, value);
        Some(ApiHookResult::caller(Some(value)))
    }

    fn context_destroy_method(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let context_ptr = Self::this_ptr(uc);
        Self::destroy_context(uc, context_ptr);
        Some(ApiHookResult::callee(1, None))
    }

    fn context_init_method(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let context_ptr = Self::this_ptr(uc);
        let ok = Self::initialize_context(uc, context_ptr);
        Some(ApiHookResult::callee(0, Some(i32::from(ok))))
    }

    fn context_open_sound_method(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let context_ptr = Self::this_ptr(uc);
        let filename_ptr = uc.read_arg(0);
        let open_mode = uc.read_arg(1);
        let sound_ptr = Self::open_sound_for_context(uc, context_ptr, filename_ptr, open_mode);
        Some(ApiHookResult::callee(2, Some(sound_ptr as i32)))
    }

    fn context_set_notify_method(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let context_ptr = Self::this_ptr(uc);
        let notify_ptr = uc.read_arg(0);
        Self::set_context_notify(uc, context_ptr, notify_ptr);
        Some(ApiHookResult::callee(1, None))
    }

    fn sound_destroy_method(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = Self::this_ptr(uc);
        Self::destroy_sound(uc, sound_ptr);
        Some(ApiHookResult::callee(1, None))
    }

    fn sound_play_method(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = Self::this_ptr(uc);
        Self::set_sound_playing(uc, sound_ptr, true, false, false);
        Some(ApiHookResult::callee(0, None))
    }

    fn sound_stop_method(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = Self::this_ptr(uc);
        Self::stop_sound(uc, sound_ptr);
        Some(ApiHookResult::callee(0, None))
    }

    fn sound_reset_method(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = Self::this_ptr(uc);
        Self::reset_sound_position(uc, sound_ptr);
        Some(ApiHookResult::callee(0, None))
    }

    fn sound_pause_method(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = Self::this_ptr(uc);
        Self::set_sound_playing(uc, sound_ptr, false, true, false);
        Some(ApiHookResult::callee(0, None))
    }

    fn sound_is_playing_method(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = Self::this_ptr(uc);
        Some(ApiHookResult::callee(
            0,
            Some(Self::sound_is_playing_value(uc, sound_ptr)),
        ))
    }

    fn sound_set_frequency_method(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = Self::this_ptr(uc);
        Self::set_sound_frequency(uc, sound_ptr, uc.read_arg(0) as i32);
        Some(ApiHookResult::callee(1, None))
    }

    fn sound_get_frequency_method(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = Self::this_ptr(uc);
        Some(ApiHookResult::callee(
            0,
            Some(Self::sound_frequency_value(uc, sound_ptr)),
        ))
    }

    fn sound_set_pan_method(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = Self::this_ptr(uc);
        Self::set_sound_pan(uc, sound_ptr, uc.read_arg(0) as i32);
        Some(ApiHookResult::callee(1, None))
    }

    fn sound_get_pan_method(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = Self::this_ptr(uc);
        Some(ApiHookResult::callee(
            0,
            Some(Self::sound_pan_value(uc, sound_ptr)),
        ))
    }

    fn sound_set_volume_method(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = Self::this_ptr(uc);
        Self::set_sound_volume(uc, sound_ptr, uc.read_arg(0) as i32);
        Some(ApiHookResult::callee(1, None))
    }

    fn sound_get_volume_method(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = Self::this_ptr(uc);
        Some(ApiHookResult::callee(
            0,
            Some(Self::sound_volume_value(uc, sound_ptr)),
        ))
    }

    fn sound_set_repeat_method(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = Self::this_ptr(uc);
        Self::set_sound_repeat(uc, sound_ptr, uc.read_arg(0) != 0);
        Some(ApiHookResult::callee(1, None))
    }

    fn sound_is_repeat_method(uc: &mut Unicorn<Win32Context>) -> Option<ApiHookResult> {
        let sound_ptr = Self::this_ptr(uc);
        Some(ApiHookResult::callee(
            0,
            Some(Self::sound_is_repeat_value(uc, sound_ptr)),
        ))
    }

    fn create_context_object(uc: &mut Unicorn<Win32Context>) -> u32 {
        let vtable_ptr = Self::ensure_context_vtable(uc);
        let context_ptr = uc.malloc(RARE_CONTEXT_SIZE) as u32;
        uc.write_u32(context_ptr as u64, vtable_ptr);
        uc.write_u32(context_ptr as u64 + 4, 0);

        uc.get_data()
            .rare_contexts
            .lock()
            .unwrap()
            .insert(context_ptr, RareContextState::default());

        let _ = Self::initialize_context(uc, context_ptr);
        context_ptr
    }

    fn initialize_context(uc: &mut Unicorn<Win32Context>, context_ptr: u32) -> bool {
        let has_context = {
            let ctx = uc.get_data();
            ctx.rare_contexts.lock().unwrap().contains_key(&context_ptr)
        };
        if !has_context {
            uc.get_data()
                .last_error
                .store(ERROR_INVALID_HANDLE, Ordering::SeqCst);
            return false;
        }

        if let Err(err) = Self::ensure_audio_engine(uc) {
            uc.get_data()
                .last_error
                .store(ERROR_NOT_READY, Ordering::SeqCst);
            crate::emu_log!(
                "[RARE] RContext::Init({:#x}) audio init skipped: {}",
                context_ptr,
                err
            );
            return true;
        }

        uc.get_data().last_error.store(0, Ordering::SeqCst);
        true
    }

    fn destroy_context(uc: &mut Unicorn<Win32Context>, context_ptr: u32) {
        if context_ptr == 0 {
            return;
        }

        let ctx = uc.get_data();
        let removed = ctx
            .rare_contexts
            .lock()
            .unwrap()
            .remove(&context_ptr)
            .is_some();

        let sound_ptrs = {
            let sounds = ctx.rare_sounds.lock().unwrap();
            sounds
                .iter()
                .filter_map(|(sound_ptr, sound)| {
                    (sound.context_ptr == context_ptr).then_some(*sound_ptr)
                })
                .collect::<Vec<_>>()
        };
        let mut sounds = ctx.rare_sounds.lock().unwrap();
        for sound_ptr in sound_ptrs {
            sounds.remove(&sound_ptr);
        }

        ctx.last_error.store(
            if removed { 0 } else { ERROR_INVALID_HANDLE },
            Ordering::SeqCst,
        );
    }

    fn set_context_notify(uc: &mut Unicorn<Win32Context>, context_ptr: u32, notify_ptr: u32) {
        let ctx = uc.get_data();
        match ctx.rare_contexts.lock().unwrap().get_mut(&context_ptr) {
            Some(state) => {
                state.notify_ptr = notify_ptr;
                ctx.last_error.store(0, Ordering::SeqCst);
            }
            None => {
                ctx.last_error.store(ERROR_INVALID_HANDLE, Ordering::SeqCst);
            }
        }
    }

    fn open_sound_for_context(
        uc: &mut Unicorn<Win32Context>,
        context_ptr: u32,
        filename_ptr: u32,
        open_mode: u32,
    ) -> u32 {
        if context_ptr == 0 {
            uc.get_data()
                .last_error
                .store(ERROR_INVALID_HANDLE, Ordering::SeqCst);
            return 0;
        }

        let filename = if filename_ptr != 0 {
            uc.read_euc_kr(filename_ptr as u64)
        } else {
            String::new()
        };

        let has_context = {
            let ctx = uc.get_data();
            ctx.rare_contexts.lock().unwrap().contains_key(&context_ptr)
        };
        if !has_context {
            uc.get_data()
                .last_error
                .store(ERROR_INVALID_HANDLE, Ordering::SeqCst);
            crate::emu_log!(
                "[RARE] RContext::OpenSound({:#x}, \"{}\", {}) -> invalid context",
                context_ptr,
                filename,
                open_mode
            );
            return 0;
        }

        let Some(source) = resolve_sound_source(&filename) else {
            uc.get_data()
                .last_error
                .store(ERROR_FILE_NOT_FOUND, Ordering::SeqCst);
            crate::emu_log!(
                "[RARE] RContext::OpenSound({:#x}, \"{}\", {}) -> file not found",
                context_ptr,
                filename,
                open_mode
            );
            return 0;
        };

        let display_name = match &source {
            ResolvedAudioSource::File(path) => path.display().to_string(),
            ResolvedAudioSource::Packed { display_name, .. } => display_name.clone(),
        };

        let wave = match decode_audio_source(&source) {
            Ok(wave) => Arc::new(wave),
            Err(err) => {
                uc.get_data()
                    .last_error
                    .store(ERROR_BAD_FORMAT, Ordering::SeqCst);
                crate::emu_log!(
                    "[RARE] RContext::OpenSound({:#x}, \"{}\", {}) -> {}",
                    context_ptr,
                    display_name,
                    open_mode,
                    err
                );
                return 0;
            }
        };

        let sound_ptr = Self::create_sound_object(uc, context_ptr, &display_name, wave);
        uc.get_data().last_error.store(0, Ordering::SeqCst);
        sound_ptr
    }

    fn create_sound_object(
        uc: &mut Unicorn<Win32Context>,
        context_ptr: u32,
        path: &str,
        wave: Arc<RareWaveData>,
    ) -> u32 {
        let vtable_ptr = Self::ensure_sound_vtable(uc);
        let sound_ptr = uc.malloc(RARE_SOUND_SIZE) as u32;
        uc.write_u32(sound_ptr as u64, vtable_ptr);
        uc.write_u32(sound_ptr as u64 + 4, context_ptr);

        let initial_frequency = wave.sample_rate as i32;
        uc.get_data().rare_sounds.lock().unwrap().insert(
            sound_ptr,
            RareSoundState {
                context_ptr,
                path: path.to_string(),
                wave,
                volume: 0,
                pan: 0,
                frequency: initial_frequency,
                repeat: false,
                playing: false,
                paused: false,
                position_frames: 0.0,
            },
        );
        sound_ptr
    }

    fn destroy_sound(uc: &mut Unicorn<Win32Context>, sound_ptr: u32) {
        if sound_ptr == 0 {
            return;
        }

        let removed = uc
            .get_data()
            .rare_sounds
            .lock()
            .unwrap()
            .remove(&sound_ptr)
            .is_some();

        uc.get_data().last_error.store(
            if removed { 0 } else { ERROR_INVALID_HANDLE },
            Ordering::SeqCst,
        );
    }

    fn set_sound_playing(
        uc: &mut Unicorn<Win32Context>,
        sound_ptr: u32,
        playing: bool,
        paused: bool,
        reset_position: bool,
    ) {
        let audio_init = if playing {
            Some(Self::ensure_audio_engine(uc))
        } else {
            None
        };
        let ctx = uc.get_data();

        match ctx.rare_sounds.lock().unwrap().get_mut(&sound_ptr) {
            Some(sound) => {
                if let Some(Err(err)) = audio_init {
                    crate::emu_log!(
                        "[RARE] RSound({:#x}) audio init skipped: {}",
                        sound_ptr,
                        err
                    );
                }
                sound.playing = playing;
                sound.paused = paused;
                if reset_position {
                    sound.position_frames = 0.0;
                }
                ctx.last_error.store(0, Ordering::SeqCst);
            }
            None => {
                ctx.last_error.store(ERROR_INVALID_HANDLE, Ordering::SeqCst);
            }
        }
    }

    fn stop_sound(uc: &mut Unicorn<Win32Context>, sound_ptr: u32) {
        let ctx = uc.get_data();
        match ctx.rare_sounds.lock().unwrap().get_mut(&sound_ptr) {
            Some(sound) => {
                sound.playing = false;
                sound.paused = false;
                sound.position_frames = 0.0;
                ctx.last_error.store(0, Ordering::SeqCst);
            }
            None => {
                ctx.last_error.store(ERROR_INVALID_HANDLE, Ordering::SeqCst);
            }
        }
    }

    fn reset_sound_position(uc: &mut Unicorn<Win32Context>, sound_ptr: u32) {
        let ctx = uc.get_data();
        match ctx.rare_sounds.lock().unwrap().get_mut(&sound_ptr) {
            Some(sound) => {
                sound.position_frames = 0.0;
                ctx.last_error.store(0, Ordering::SeqCst);
            }
            None => {
                ctx.last_error.store(ERROR_INVALID_HANDLE, Ordering::SeqCst);
            }
        }
    }

    fn sound_is_playing_value(uc: &mut Unicorn<Win32Context>, sound_ptr: u32) -> i32 {
        let ctx = uc.get_data();
        let sounds = ctx.rare_sounds.lock().unwrap();
        match sounds.get(&sound_ptr) {
            Some(sound) => {
                ctx.last_error.store(0, Ordering::SeqCst);
                i32::from(sound.playing && !sound.paused)
            }
            None => {
                ctx.last_error.store(ERROR_INVALID_HANDLE, Ordering::SeqCst);
                0
            }
        }
    }

    fn set_sound_frequency(uc: &mut Unicorn<Win32Context>, sound_ptr: u32, value: i32) {
        let ctx = uc.get_data();
        match ctx.rare_sounds.lock().unwrap().get_mut(&sound_ptr) {
            Some(sound) => {
                sound.frequency = value;
                ctx.last_error.store(0, Ordering::SeqCst);
            }
            None => {
                ctx.last_error.store(ERROR_INVALID_HANDLE, Ordering::SeqCst);
            }
        }
    }

    fn sound_frequency_value(uc: &mut Unicorn<Win32Context>, sound_ptr: u32) -> i32 {
        let ctx = uc.get_data();
        let sounds = ctx.rare_sounds.lock().unwrap();
        match sounds.get(&sound_ptr) {
            Some(sound) => {
                ctx.last_error.store(0, Ordering::SeqCst);
                sound.frequency
            }
            None => {
                ctx.last_error.store(ERROR_INVALID_HANDLE, Ordering::SeqCst);
                -1
            }
        }
    }

    fn set_sound_pan(uc: &mut Unicorn<Win32Context>, sound_ptr: u32, value: i32) {
        let ctx = uc.get_data();
        match ctx.rare_sounds.lock().unwrap().get_mut(&sound_ptr) {
            Some(sound) => {
                sound.pan = value;
                ctx.last_error.store(0, Ordering::SeqCst);
            }
            None => {
                ctx.last_error.store(ERROR_INVALID_HANDLE, Ordering::SeqCst);
            }
        }
    }

    fn sound_pan_value(uc: &mut Unicorn<Win32Context>, sound_ptr: u32) -> i32 {
        let ctx = uc.get_data();
        let sounds = ctx.rare_sounds.lock().unwrap();
        match sounds.get(&sound_ptr) {
            Some(sound) => {
                ctx.last_error.store(0, Ordering::SeqCst);
                sound.pan
            }
            None => {
                ctx.last_error.store(ERROR_INVALID_HANDLE, Ordering::SeqCst);
                0
            }
        }
    }

    fn set_sound_volume(uc: &mut Unicorn<Win32Context>, sound_ptr: u32, value: i32) {
        let ctx = uc.get_data();
        match ctx.rare_sounds.lock().unwrap().get_mut(&sound_ptr) {
            Some(sound) => {
                sound.volume = value;
                ctx.last_error.store(0, Ordering::SeqCst);
            }
            None => {
                ctx.last_error.store(ERROR_INVALID_HANDLE, Ordering::SeqCst);
            }
        }
    }

    fn sound_volume_value(uc: &mut Unicorn<Win32Context>, sound_ptr: u32) -> i32 {
        let ctx = uc.get_data();
        let sounds = ctx.rare_sounds.lock().unwrap();
        match sounds.get(&sound_ptr) {
            Some(sound) => {
                ctx.last_error.store(0, Ordering::SeqCst);
                sound.volume
            }
            None => {
                ctx.last_error.store(ERROR_INVALID_HANDLE, Ordering::SeqCst);
                -1
            }
        }
    }

    fn set_sound_repeat(uc: &mut Unicorn<Win32Context>, sound_ptr: u32, repeat: bool) {
        let ctx = uc.get_data();
        match ctx.rare_sounds.lock().unwrap().get_mut(&sound_ptr) {
            Some(sound) => {
                sound.repeat = repeat;
                ctx.last_error.store(0, Ordering::SeqCst);
            }
            None => {
                ctx.last_error.store(ERROR_INVALID_HANDLE, Ordering::SeqCst);
            }
        }
    }

    fn sound_is_repeat_value(uc: &mut Unicorn<Win32Context>, sound_ptr: u32) -> i32 {
        let ctx = uc.get_data();
        let sounds = ctx.rare_sounds.lock().unwrap();
        match sounds.get(&sound_ptr) {
            Some(sound) => {
                ctx.last_error.store(0, Ordering::SeqCst);
                i32::from(sound.repeat)
            }
            None => {
                ctx.last_error.store(ERROR_INVALID_HANDLE, Ordering::SeqCst);
                0
            }
        }
    }

    fn this_ptr(uc: &mut Unicorn<Win32Context>) -> u32 {
        uc.reg_read(RegisterX86::ECX).unwrap_or(0) as u32
    }

    fn register_proxy_symbol(uc: &mut Unicorn<Win32Context>, symbol: &str) -> u32 {
        let cache_key = format!("Rare.dll!{}", symbol);
        if let Some(addr) = uc
            .get_data()
            .proxy_exports
            .lock()
            .unwrap()
            .get(&cache_key)
            .copied()
        {
            return addr;
        }

        let addr = uc.get_data().import_address.fetch_add(4, Ordering::SeqCst);
        uc.get_data()
            .proxy_exports
            .lock()
            .unwrap()
            .insert(cache_key.clone(), addr);
        uc.get_data()
            .address_map
            .lock()
            .unwrap()
            .insert(addr as u64, cache_key);
        addr
    }

    fn ensure_context_vtable(uc: &mut Unicorn<Win32Context>) -> u32 {
        if let Some(ptr) = uc
            .get_data()
            .proxy_exports
            .lock()
            .unwrap()
            .get(CONTEXT_VTABLE_KEY)
            .copied()
        {
            return ptr;
        }

        let methods = [
            METHOD_CONTEXT_DESTROY,
            METHOD_CONTEXT_INIT,
            METHOD_CONTEXT_OPEN_SOUND,
            METHOD_CONTEXT_SET_NOTIFY,
        ];
        let ptr = uc.malloc(methods.len() * 4) as u32;
        for (index, method) in methods.iter().enumerate() {
            let target = Self::register_proxy_symbol(uc, method);
            uc.write_u32(ptr as u64 + (index * 4) as u64, target);
        }
        uc.get_data()
            .proxy_exports
            .lock()
            .unwrap()
            .insert(CONTEXT_VTABLE_KEY.to_string(), ptr);
        ptr
    }

    fn ensure_sound_vtable(uc: &mut Unicorn<Win32Context>) -> u32 {
        if let Some(ptr) = uc
            .get_data()
            .proxy_exports
            .lock()
            .unwrap()
            .get(SOUND_VTABLE_KEY)
            .copied()
        {
            return ptr;
        }

        let methods = [
            METHOD_SOUND_DESTROY,
            METHOD_SOUND_PLAY,
            METHOD_SOUND_STOP,
            METHOD_SOUND_RESET,
            METHOD_SOUND_PAUSE,
            METHOD_SOUND_IS_PLAYING,
            METHOD_SOUND_SET_FREQUENCY,
            METHOD_SOUND_GET_FREQUENCY,
            METHOD_SOUND_SET_PAN,
            METHOD_SOUND_GET_PAN,
            METHOD_SOUND_SET_VOLUME,
            METHOD_SOUND_GET_VOLUME,
            METHOD_SOUND_SET_REPEAT,
            METHOD_SOUND_IS_REPEAT,
        ];
        let ptr = uc.malloc(methods.len() * 4) as u32;
        for (index, method) in methods.iter().enumerate() {
            let target = Self::register_proxy_symbol(uc, method);
            uc.write_u32(ptr as u64 + (index * 4) as u64, target);
        }
        uc.get_data()
            .proxy_exports
            .lock()
            .unwrap()
            .insert(SOUND_VTABLE_KEY.to_string(), ptr);
        ptr
    }

    fn ensure_audio_engine(uc: &mut Unicorn<Win32Context>) -> Result<(), String> {
        let existing = {
            let ctx = uc.get_data();
            ctx.rare_audio.lock().unwrap().is_some()
        };
        if existing {
            return Ok(());
        }

        let sounds = uc.get_data().rare_sounds.clone();
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "출력 오디오 디바이스를 찾지 못했습니다".to_string())?;
        let supported_config = device
            .default_output_config()
            .map_err(|err| format!("기본 오디오 설정 조회 실패: {err}"))?;
        let sample_format = supported_config.sample_format();
        let config: StreamConfig = supported_config.config();

        let stream = match sample_format {
            SampleFormat::F32 => build_output_stream::<f32>(&device, &config, sounds.clone()),
            SampleFormat::I16 => build_output_stream::<i16>(&device, &config, sounds.clone()),
            SampleFormat::U16 => build_output_stream::<u16>(&device, &config, sounds.clone()),
            other => Err(format!("지원하지 않는 출력 샘플 포맷: {other:?}")),
        }?;

        stream
            .play()
            .map_err(|err| format!("오디오 스트림 시작 실패: {err}"))?;

        uc.get_data()
            .rare_audio
            .lock()
            .unwrap()
            .replace(RareAudioEngine { _stream: stream });
        crate::emu_log!(
            "[RARE] CPAL output ready: {}ch @ {}Hz",
            config.channels,
            config.sample_rate
        );
        Ok(())
    }
}

fn normalize_export_name(func_name: &str) -> Option<&'static str> {
    match func_name {
        RARE_CREATE_CONTEXT | "Rare_CreateContext" => Some(RARE_CREATE_CONTEXT),
        RARE_DESTROY_CONTEXT | "Rare_DestroyContext" => Some(RARE_DESTROY_CONTEXT),
        RARE_SET_SOUND_NOTIFY | "Rare_SetSoundNotify" => Some(RARE_SET_SOUND_NOTIFY),
        RARE_OPEN_SOUND | "Rare_OpenSound" => Some(RARE_OPEN_SOUND),
        RARE_CLOSE_SOUND | "Rare_CloseSound" => Some(RARE_CLOSE_SOUND),
        RARE_PLAY_SOUND | "Rare_PlaySound" => Some(RARE_PLAY_SOUND),
        RARE_STOP_SOUND | "Rare_StopSound" => Some(RARE_STOP_SOUND),
        RARE_RESET_SOUND | "Rare_ResetSound" => Some(RARE_RESET_SOUND),
        RARE_PAUSE_SOUND | "Rare_PauseSound" => Some(RARE_PAUSE_SOUND),
        RARE_IS_PLAY_SOUND | "Rare_IsPlaySound" => Some(RARE_IS_PLAY_SOUND),
        RARE_SET_FREQUENCY | "Rare_SetFrequency" => Some(RARE_SET_FREQUENCY),
        RARE_GET_FREQUENCY | "Rare_GetFrequency" => Some(RARE_GET_FREQUENCY),
        RARE_SET_PAN | "Rare_SetPan" => Some(RARE_SET_PAN),
        RARE_GET_PAN | "Rare_GetPan" => Some(RARE_GET_PAN),
        RARE_SET_VOLUME | "Rare_SetVolume" => Some(RARE_SET_VOLUME),
        RARE_GET_VOLUME | "Rare_GetVolume" => Some(RARE_GET_VOLUME),
        RARE_SET_REPEAT | "Rare_SetRepeat" => Some(RARE_SET_REPEAT),
        RARE_IS_REPEAT | "Rare_IsRepeat" => Some(RARE_IS_REPEAT),
        _ => None,
    }
}

fn build_output_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    sounds: Arc<Mutex<HashMap<u32, RareSoundState>>>,
) -> Result<Stream, String>
where
    T: Sample + SizedSample + FromSample<f32>,
{
    let output_channels = usize::from(config.channels.max(1));
    let output_sample_rate = config.sample_rate;
    let err_fn = |err| crate::emu_log!("[RARE] CPAL stream error: {}", err);

    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                mix_output_buffer(data, output_channels, output_sample_rate, &sounds);
            },
            err_fn,
            None,
        )
        .map_err(|err| format!("출력 스트림 생성 실패: {err}"))
}

fn mix_output_buffer<T>(
    data: &mut [T],
    output_channels: usize,
    output_sample_rate: u32,
    sounds: &Arc<Mutex<HashMap<u32, RareSoundState>>>,
) where
    T: Sample + FromSample<f32>,
{
    if data.is_empty() || output_channels == 0 {
        return;
    }

    let mut mix = vec![0.0f32; data.len()];
    if let Ok(mut guard) = sounds.lock() {
        for sound in guard.values_mut() {
            mix_sound_into(&mut mix, output_channels, output_sample_rate, sound);
        }
    }

    for (slot, sample) in data.iter_mut().zip(mix.into_iter()) {
        *slot = T::from_sample(sample.clamp(-1.0, 1.0));
    }
}

fn mix_sound_into(
    mix: &mut [f32],
    output_channels: usize,
    output_sample_rate: u32,
    sound: &mut RareSoundState,
) {
    if !sound.playing || sound.paused || sound.wave.samples.is_empty() {
        return;
    }

    let total_frames = sound.frame_len();
    if total_frames == 0 {
        sound.playing = false;
        return;
    }

    let volume = sound.volume_gain();
    let (left_gain, right_gain) = sound.channel_gains();
    let step = sound.playback_step(output_sample_rate);
    let frame_count = mix.len() / output_channels;
    let mut position = sound.position_frames;

    for frame_index in 0..frame_count {
        let mut source_frame = position.floor() as usize;
        if source_frame >= total_frames {
            if sound.repeat {
                position %= total_frames as f64;
                source_frame = position.floor() as usize;
            } else {
                sound.playing = false;
                sound.paused = false;
                sound.position_frames = 0.0;
                return;
            }
        }

        let (left, right) = sound.current_frame(source_frame);
        let mix_offset = frame_index * output_channels;
        let mono = (left + right) * 0.5 * volume;

        if output_channels == 1 {
            mix[mix_offset] += mono;
        } else {
            mix[mix_offset] += left * volume * left_gain;
            mix[mix_offset + 1] += right * volume * right_gain;
            for extra_channel in 2..output_channels {
                mix[mix_offset + extra_channel] += mono;
            }
        }

        position += step;
    }

    sound.position_frames = position;
}

fn resolve_sound_source(filename: &str) -> Option<ResolvedAudioSource> {
    let trimmed = filename.trim().trim_matches('"');
    if trimmed.is_empty() {
        return None;
    }

    let normalized = trimmed.replace('\\', "/");
    let normalized_path = Path::new(&normalized);
    let mut candidates = vec![
        PathBuf::from(&normalized),
        crate::resource_dir().join(&normalized),
    ];
    if normalized_path.extension().is_none() {
        candidates.push(PathBuf::from(format!("{normalized}.wav")));
        candidates.push(crate::resource_dir().join(format!("{normalized}.wav")));
        candidates.push(PathBuf::from(format!("{normalized}.mp3")));
        candidates.push(crate::resource_dir().join(format!("{normalized}.mp3")));
    }

    for candidate in candidates {
        if candidate.is_file() {
            return Some(ResolvedAudioSource::File(candidate));
        }
    }

    let wanted_name = normalized_path.file_name()?.to_str()?.to_string();
    let mut pack_names = vec![wanted_name.clone()];
    let wanted_stem = normalized_path.file_stem()?.to_str()?.to_string();
    if normalized_path.extension().is_none() {
        pack_names.push(format!("{wanted_stem}.wav"));
        pack_names.push(format!("{wanted_stem}.mp3"));
    }

    for pack_name in &pack_names {
        if let Some(source) =
            resolve_sound_source_from_pack(pack_name, &crate::resource_dir().join("Sound.pak"))
        {
            return Some(source);
        }
        if let Some(source) =
            resolve_sound_source_from_pack(pack_name, &crate::resource_dir().join("BGM.pak"))
        {
            return Some(source);
        }
    }

    find_resource_file(crate::resource_dir(), &wanted_name).map(ResolvedAudioSource::File)
}

fn decode_audio_source(source: &ResolvedAudioSource) -> Result<RareWaveData, String> {
    match source {
        ResolvedAudioSource::File(path) => decode_audio_file(path),
        ResolvedAudioSource::Packed { display_name, data } => {
            decode_audio_bytes(display_name, data)
        }
    }
}

fn decode_audio_file(path: &Path) -> Result<RareWaveData, String> {
    let data = fs::read(path).map_err(|err| format!("파일 읽기 실패: {err}"))?;
    decode_audio_bytes(&path.display().to_string(), &data)
}

fn decode_audio_bytes(name_hint: &str, data: &[u8]) -> Result<RareWaveData, String> {
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WAVE" {
        return decode_wave_bytes(data);
    }

    if name_hint.to_ascii_lowercase().ends_with(".mp3")
        || data.starts_with(b"ID3")
        || data
            .windows(2)
            .any(|pair| pair[0] == 0xff && (pair[1] & 0xe0) == 0xe0)
    {
        return decode_mp3_bytes(data);
    }

    Err("지원하지 않는 오디오 포맷입니다".to_string())
}

fn resolve_sound_source_from_pack(filename: &str, pack_path: &Path) -> Option<ResolvedAudioSource> {
    let wanted = filename.to_ascii_lowercase();
    let data = fs::read(pack_path).ok()?;
    let entry_count_raw = i32::from_le_bytes(data.get(0..4)?.try_into().ok()?);
    let entry_count = if entry_count_raw < 0 {
        (-entry_count_raw - 1) as usize
    } else {
        entry_count_raw as usize
    };
    let header_size = 4usize.checked_add(entry_count.checked_mul(28)?)?;
    if data.len() < header_size {
        return None;
    }

    for index in 0..entry_count {
        let entry_offset = 4 + index * 28;
        let entry = data.get(entry_offset..entry_offset + 28)?;
        let name = entry[0..20]
            .iter()
            .map(|byte| byte ^ 0xff)
            .take_while(|byte| *byte != 0)
            .collect::<Vec<_>>();
        let entry_name = String::from_utf8_lossy(&name).to_string();
        if entry_name.to_ascii_lowercase() != wanted {
            continue;
        }

        let start_marker = i32::from_le_bytes(entry[20..24].try_into().ok()?);
        let end_marker = i32::from_le_bytes(entry[24..28].try_into().ok()?);
        if start_marker >= 0 || end_marker >= 0 {
            return None;
        }

        // pack 헤더 끝을 기준으로 1-based 음수 오프셋을 저장하는 포맷입니다.
        let start = header_size.checked_add((-start_marker - 1) as usize)?;
        let end = header_size.checked_add((-end_marker) as usize)?;
        if start >= end || end > data.len() {
            return None;
        }

        return Some(ResolvedAudioSource::Packed {
            display_name: format!("{}::{}", pack_path.display(), entry_name),
            data: data[start..end].to_vec(),
        });
    }

    None
}

fn find_resource_file(root: &Path, wanted_name: &str) -> Option<PathBuf> {
    let wanted_name = wanted_name.to_ascii_lowercase();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.to_ascii_lowercase() == wanted_name)
                .unwrap_or(false)
            {
                return Some(path);
            }
        }
    }

    None
}

fn decode_mp3_bytes(data: &[u8]) -> Result<RareWaveData, String> {
    let mut decoder = Mp3Decoder::new(Cursor::new(data));
    let mut sample_rate = 0u32;
    let mut channels = 0usize;
    let mut samples = Vec::new();

    loop {
        match decoder.next_frame() {
            Ok(frame) => {
                if sample_rate == 0 {
                    sample_rate = frame.sample_rate as u32;
                    channels = frame.channels;
                } else if sample_rate != frame.sample_rate as u32 || channels != frame.channels {
                    return Err("프레임마다 MP3 오디오 설정이 달라 지원하지 않습니다".to_string());
                }

                samples.extend(
                    frame
                        .data
                        .into_iter()
                        .map(|sample| sample as f32 / i16::MAX as f32),
                );
            }
            Err(Mp3Error::Eof) => break,
            Err(Mp3Error::InsufficientData) => break,
            Err(err) => return Err(format!("MP3 디코딩 실패: {err:?}")),
        }
    }

    if sample_rate == 0 || channels == 0 || samples.is_empty() {
        return Err("MP3 프레임을 읽지 못했습니다".to_string());
    }

    Ok(RareWaveData {
        channels,
        sample_rate,
        samples,
    })
}

fn decode_wave_bytes(data: &[u8]) -> Result<RareWaveData, String> {
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err("WAV RIFF 헤더가 아닙니다".to_string());
    }

    let mut audio_format = 0u16;
    let mut channels = 0u16;
    let mut sample_rate = 0u32;
    let mut bits_per_sample = 0u16;
    let mut pcm_data: Option<&[u8]> = None;
    let mut pos = 12usize;

    while pos + 8 <= data.len() {
        let chunk_id = &data[pos..pos + 4];
        let chunk_size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        pos += 8;
        if pos + chunk_size > data.len() {
            break;
        }

        match chunk_id {
            b"fmt " => {
                if chunk_size < 16 {
                    return Err("WAV fmt 청크가 너무 짧습니다".to_string());
                }
                audio_format = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap());
                channels = u16::from_le_bytes(data[pos + 2..pos + 4].try_into().unwrap());
                sample_rate = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap());
                bits_per_sample = u16::from_le_bytes(data[pos + 14..pos + 16].try_into().unwrap());
            }
            b"data" => {
                pcm_data = Some(&data[pos..pos + chunk_size]);
            }
            _ => {}
        }

        pos += (chunk_size + 1) & !1;
    }

    if channels == 0 || sample_rate == 0 || bits_per_sample == 0 {
        return Err("WAV fmt 정보가 누락되었습니다".to_string());
    }
    let pcm_data = pcm_data.ok_or_else(|| "WAV data 청크를 찾지 못했습니다".to_string())?;
    let samples = decode_samples(audio_format, bits_per_sample, pcm_data)?;

    Ok(RareWaveData {
        channels: usize::from(channels),
        sample_rate,
        samples,
    })
}

fn decode_samples(
    audio_format: u16,
    bits_per_sample: u16,
    data: &[u8],
) -> Result<Vec<f32>, String> {
    match (audio_format, bits_per_sample) {
        (1, 8) => Ok(data
            .iter()
            .map(|sample| (*sample as f32 - 128.0) / 128.0)
            .collect()),
        (1, 16) => Ok(data
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]) as f32 / i16::MAX as f32)
            .collect()),
        (1, 24) => Ok(data
            .chunks_exact(3)
            .map(|chunk| {
                let value =
                    ((chunk[2] as i32) << 24 | (chunk[1] as i32) << 16 | (chunk[0] as i32) << 8)
                        >> 8;
                value as f32 / 8_388_607.0
            })
            .collect()),
        (1, 32) => Ok(data
            .chunks_exact(4)
            .map(|chunk| {
                i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as f32
                    / i32::MAX as f32
            })
            .collect()),
        (3, 32) => Ok(data
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect()),
        _ => Err(format!(
            "지원하지 않는 WAV 포맷입니다 (format={}, bits={})",
            audio_format, bits_per_sample
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::decode_wave_bytes;

    #[test]
    fn decode_pcm16_wave() {
        let wave = vec![
            b'R', b'I', b'F', b'F', 40, 0, 0, 0, b'W', b'A', b'V', b'E', b'f', b'm', b't', b' ',
            16, 0, 0, 0, 1, 0, 1, 0, 0x44, 0xac, 0, 0, 0x88, 0x58, 0x01, 0, 2, 0, 16, 0, b'd',
            b'a', b't', b'a', 4, 0, 0, 0, 0, 0, 0xff, 0x7f,
        ];
        let decoded = decode_wave_bytes(&wave).expect("PCM16 decoding should succeed");
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.sample_rate, 44_100);
        assert_eq!(decoded.samples.len(), 2);
        assert!(decoded.samples[0].abs() < 0.0001);
        assert!(decoded.samples[1] > 0.99);
    }
}
