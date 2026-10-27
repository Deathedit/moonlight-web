//! Safe-ish wrapper around moonlight-common-c's connection API.
//!
//! moonlight-common-c keeps global state: exactly one connection per process, and
//! its callbacks are plain C function pointers without a user-data argument. So the
//! callbacks forward into a process-wide [`Sink`] of channels, and [`Connection`]
//! guards that only one exists at a time.

use std::{
    ffi::{c_char, c_int, CStr, CString},
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

use bytes::Bytes;
use moonlight_common_sys as ml;
use tokio::sync::mpsc;

/// One encoded video frame (Annex-B access unit).
pub struct VideoFrame {
    pub data: Bytes,
    pub idr: bool,
    /// When the frame was fully received from the host.
    pub received: Instant,
}

/// One Opus packet (stereo: a plain Opus packet, playable as-is by WebRTC).
pub struct AudioFrame {
    pub data: Bytes,
    pub duration: Duration,
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    StageStarting(String),
    StageFailed { stage: String, code: i32 },
    Started,
    Terminated { code: i32 },
    ConnectionPoor(bool),
    /// Host wants controller `controller` to rumble (0..=65535 per motor; 0/0 = stop).
    Rumble { controller: u16, low: u16, high: u16 },
}

pub struct ConnectionParams {
    pub address: String,
    pub app_version: String,
    pub gfe_version: String,
    pub rtsp_url: Option<String>,
    pub server_codec_mode_support: i32,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub video_formats: i32,
    pub audio_configuration: i32,
    pub ri_key: [u8; 16],
    pub ri_iv: [u8; 16],
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("another stream is already active on this bridge")]
    Busy,
    #[error("connection failed at stage \"{stage}\" (error {code})")]
    Failed { stage: String, code: i32 },
    #[error("invalid parameter: {0}")]
    Invalid(&'static str),
}

struct Sink {
    video: mpsc::Sender<VideoFrame>,
    audio: mpsc::Sender<AudioFrame>,
    events: mpsc::UnboundedSender<StreamEvent>,
}

static SINK: Mutex<Option<Sink>> = Mutex::new(None);
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Frames buffered between the receive thread and the WebRTC writer. Small on purpose:
/// if the browser side falls behind we'd rather drop and re-sync on an IDR than lag.
pub const VIDEO_QUEUE: usize = 8;
/// Audio packets are tiny and frequent (5–10 ms each); a late packet is dropped, not queued.
pub const AUDIO_QUEUE: usize = 32;

/// Opus frame duration in microseconds, from the audio init callback.
static AUDIO_FRAME_US: AtomicU32 = AtomicU32::new(5_000);

/// A running moonlight-common-c connection. Call [`Connection::stop`] to end it.
pub struct Connection {
    stopped: bool,
}

impl Connection {
    /// Run the (blocking) RTSP/ENet handshake and start streaming. Call from a blocking thread.
    pub fn start(
        p: ConnectionParams,
        video: mpsc::Sender<VideoFrame>,
        audio: mpsc::Sender<AudioFrame>,
        events: mpsc::UnboundedSender<StreamEvent>,
    ) -> Result<Self, ConnectError> {
        if ACTIVE.swap(true, Ordering::AcqRel) {
            return Err(ConnectError::Busy);
        }
        *SINK.lock().unwrap() = Some(Sink { video, audio, events });

        let cstr = |s: &str, what| CString::new(s).map_err(|_| ConnectError::Invalid(what));
        let strings = (|| {
            Ok::<_, ConnectError>((
                cstr(&p.address, "address")?,
                cstr(&p.app_version, "app version")?,
                cstr(&p.gfe_version, "gfe version")?,
                p.rtsp_url.as_deref().map(|u| cstr(u, "rtsp url")).transpose()?,
            ))
        })();
        let (address, app_version, gfe_version, rtsp_url) = match strings {
            Ok(s) => s,
            Err(e) => {
                release();
                return Err(e);
            }
        };

        // SAFETY: plain C structs; zero-initialised by the library's own init functions.
        let mut server: ml::SERVER_INFORMATION = unsafe { std::mem::zeroed() };
        let mut config: ml::STREAM_CONFIGURATION = unsafe { std::mem::zeroed() };
        let mut listener: ml::CONNECTION_LISTENER_CALLBACKS = unsafe { std::mem::zeroed() };
        let mut video_cb: ml::DECODER_RENDERER_CALLBACKS = unsafe { std::mem::zeroed() };
        let mut audio_cb: ml::AUDIO_RENDERER_CALLBACKS = unsafe { std::mem::zeroed() };
        unsafe {
            ml::LiInitializeServerInformation(&mut server);
            ml::LiInitializeStreamConfiguration(&mut config);
            ml::LiInitializeConnectionCallbacks(&mut listener);
            ml::LiInitializeVideoCallbacks(&mut video_cb);
            ml::LiInitializeAudioCallbacks(&mut audio_cb);
        }

        server.address = address.as_ptr();
        server.serverInfoAppVersion = app_version.as_ptr();
        server.serverInfoGfeVersion = gfe_version.as_ptr();
        server.rtspSessionUrl = rtsp_url.as_ref().map_or(std::ptr::null(), |u| u.as_ptr());
        server.serverCodecModeSupport = p.server_codec_mode_support;

        config.width = p.width as c_int;
        config.height = p.height as c_int;
        config.fps = p.fps as c_int;
        config.bitrate = p.bitrate_kbps as c_int;
        config.packetSize = 1392;
        config.streamingRemotely = ml::STREAM_CFG_AUTO as c_int;
        config.audioConfiguration = p.audio_configuration;
        config.supportedVideoFormats = p.video_formats;
        config.clientRefreshRateX100 = (p.fps * 100) as c_int;
        config.colorSpace = ml::COLORSPACE_REC_709 as c_int;
        config.colorRange = ml::COLOR_RANGE_LIMITED as c_int;
        config.encryptionFlags = ml::ENCFLG_AUDIO as c_int;
        for i in 0..16 {
            config.remoteInputAesKey[i] = p.ri_key[i] as c_char;
            config.remoteInputAesIv[i] = p.ri_iv[i] as c_char;
        }

        listener.stageStarting = Some(on_stage_starting);
        listener.stageFailed = Some(on_stage_failed);
        listener.connectionStarted = Some(on_connection_started);
        listener.connectionTerminated = Some(on_connection_terminated);
        listener.connectionStatusUpdate = Some(on_connection_status);
        listener.rumble = Some(on_rumble);
        listener.logMessage = Some(ml::mlw_log_message);
        unsafe { ml::mlw_set_log_sink(Some(on_log)) };

        video_cb.setup = Some(on_video_setup);
        video_cb.submitDecodeUnit = Some(on_decode_unit);
        // We forward frames as they arrive; no pull renderer, no reference frame invalidation.
        video_cb.capabilities = ml::CAPABILITY_DIRECT_SUBMIT as c_int;

        audio_cb.init = Some(on_audio_init);
        audio_cb.decodeAndPlaySample = Some(on_audio_sample);
        audio_cb.capabilities = ml::CAPABILITY_DIRECT_SUBMIT as c_int;

        // SAFETY: all pointers are valid for the duration of the call; the library copies
        // what it keeps.
        let rc = unsafe {
            ml::LiStartConnection(
                &mut server,
                &mut config,
                &mut listener,
                &mut video_cb,
                &mut audio_cb,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc != 0 {
            let stage = LAST_FAILED_STAGE.lock().unwrap().take().unwrap_or_else(|| "unknown".into());
            release();
            return Err(ConnectError::Failed { stage, code: rc });
        }
        Ok(Self { stopped: false })
    }

    /// Ask the host for a keyframe (e.g. after a WebRTC PLI or dropped frames).
    pub fn request_idr() {
        if ACTIVE.load(Ordering::Acquire) {
            unsafe { ml::LiRequestIdrFrame() };
        }
    }

    /// Whether a connection is up (input calls are only meaningful then).
    pub fn is_active() -> bool {
        ACTIVE.load(Ordering::Acquire)
    }

    /// Tear down the connection. Blocks until the library's threads have exited.
    pub fn stop(mut self) {
        self.stop_inner();
    }

    fn stop_inner(&mut self) {
        if !self.stopped {
            self.stopped = true;
            unsafe { ml::LiStopConnection() };
            release();
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.stop_inner();
    }
}

fn release() {
    *SINK.lock().unwrap() = None;
    ACTIVE.store(false, Ordering::Release);
}

static LAST_FAILED_STAGE: Mutex<Option<String>> = Mutex::new(None);

fn stage_name(stage: c_int) -> String {
    // SAFETY: returns a static string for any input.
    unsafe { CStr::from_ptr(ml::LiGetStageName(stage)) }.to_string_lossy().into_owned()
}

fn emit(event: StreamEvent) {
    if let Some(sink) = SINK.lock().unwrap().as_ref() {
        let _ = sink.events.send(event);
    }
}

// ---- C callbacks (called on moonlight-common-c's threads; must not block) ----

unsafe extern "C" fn on_log(line: *const c_char) {
    let line = CStr::from_ptr(line).to_string_lossy();
    tracing::debug!(target: "moonlight_common", "{line}");
}

extern "C" fn on_stage_starting(stage: c_int) {
    emit(StreamEvent::StageStarting(stage_name(stage)));
}

extern "C" fn on_stage_failed(stage: c_int, code: c_int) {
    let stage = stage_name(stage);
    *LAST_FAILED_STAGE.lock().unwrap() = Some(stage.clone());
    emit(StreamEvent::StageFailed { stage, code });
}

extern "C" fn on_connection_started() {
    emit(StreamEvent::Started);
}

extern "C" fn on_connection_terminated(code: c_int) {
    emit(StreamEvent::Terminated { code });
}

extern "C" fn on_connection_status(status: c_int) {
    emit(StreamEvent::ConnectionPoor(status == ml::CONN_STATUS_POOR as c_int));
}

extern "C" fn on_video_setup(format: c_int, width: c_int, height: c_int, fps: c_int, _ctx: *mut std::ffi::c_void, _flags: c_int) -> c_int {
    tracing::info!(format = format!("{format:#x}"), width, height, fps, "video stream configured");
    // We advertise exactly one 8-bit 4:2:0 format, so anything else is unexpected.
    let supported = (ml::VIDEO_FORMAT_H264 | ml::VIDEO_FORMAT_H265 | ml::VIDEO_FORMAT_AV1_MAIN8) as c_int;
    if format & supported == 0 {
        tracing::error!("host chose an unsupported video format {format:#x}");
        return -1;
    }
    0
}

unsafe extern "C" fn on_decode_unit(du: *mut ml::DECODE_UNIT) -> c_int {
    let du = &*du;
    let mut data = Vec::with_capacity(du.fullLength.max(0) as usize);
    let mut entry = du.bufferList;
    while !entry.is_null() {
        let e = &*entry;
        data.extend_from_slice(std::slice::from_raw_parts(e.data as *const u8, e.length as usize));
        entry = e.next;
    }
    let frame = VideoFrame {
        data: Bytes::from(data),
        idr: du.frameType == ml::FRAME_TYPE_IDR as c_int,
        received: Instant::now(),
    };

    let guard = SINK.lock().unwrap();
    let Some(sink) = guard.as_ref() else { return ml::DR_OK as c_int };
    match sink.video.try_send(frame) {
        Ok(()) => ml::DR_OK as c_int,
        // Writer is behind: this frame is lost, so later P-frames would reference
        // missing data. Ask for a fresh IDR instead of showing corruption.
        Err(mpsc::error::TrySendError::Full(_)) => ml::DR_NEED_IDR,
        Err(mpsc::error::TrySendError::Closed(_)) => ml::DR_OK as c_int,
    }
}

extern "C" fn on_rumble(controller: u16, low: u16, high: u16) {
    emit(StreamEvent::Rumble { controller, low, high });
}

unsafe extern "C" fn on_audio_init(
    config: c_int,
    // `const POPUS_...` in C: the pointer is const, not the data, hence *mut.
    opus: *mut ml::OPUS_MULTISTREAM_CONFIGURATION,
    _ctx: *mut std::ffi::c_void,
    _flags: c_int,
) -> c_int {
    let opus = &*opus;
    tracing::info!(
        channels = opus.channelCount,
        streams = opus.streams,
        coupled = opus.coupledStreams,
        samples_per_frame = opus.samplesPerFrame,
        "audio stream configured"
    );
    // Stereo multistream with one coupled stream is bit-identical to a plain Opus
    // packet, which is what WebRTC carries. Surround would need re-encoding.
    if config != ml::AUDIO_CONFIGURATION_STEREO || opus.streams != 1 || opus.coupledStreams != 1 {
        tracing::error!("unsupported audio layout; only stereo is forwarded");
        return -1;
    }
    let us = (opus.samplesPerFrame as u64 * 1_000_000 / opus.sampleRate.max(1) as u64) as u32;
    AUDIO_FRAME_US.store(us, Ordering::Relaxed);
    0
}

unsafe extern "C" fn on_audio_sample(data: *mut c_char, len: c_int) {
    // NULL marks a lost packet; the browser's Opus decoder conceals the gap itself.
    if data.is_null() || len <= 0 {
        return;
    }
    let frame = AudioFrame {
        data: Bytes::copy_from_slice(std::slice::from_raw_parts(data as *const u8, len as usize)),
        duration: Duration::from_micros(AUDIO_FRAME_US.load(Ordering::Relaxed) as u64),
    };
    if let Some(sink) = SINK.lock().unwrap().as_ref() {
        let _ = sink.audio.try_send(frame);
    }
}
