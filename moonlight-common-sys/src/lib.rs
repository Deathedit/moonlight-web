//! Raw, unsafe FFI bindings to moonlight-common-c (`Limelight.h`).
//! Safe wrappers live in the `bridge` crate.

#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case, dead_code)]

// Ensure libcrypto gets linked for moonlight-common-c.
extern crate openssl_sys;

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

// ---- shim.c ----

/// Receives one formatted log line (NUL-terminated, no trailing newline).
pub type LogSink = unsafe extern "C" fn(line: *const std::os::raw::c_char);

extern "C" {
    /// Install the function `mlw_log_message` forwards formatted lines to.
    pub fn mlw_set_log_sink(sink: Option<LogSink>);
    /// printf-style; use as `CONNECTION_LISTENER_CALLBACKS::logMessage`.
    pub fn mlw_log_message(format: *const std::os::raw::c_char, ...);
}

// Function-like macros from Limelight.h that bindgen can't translate.

pub const fn make_audio_configuration(channel_count: i32, channel_mask: i32) -> i32 {
    (channel_mask << 16) | (channel_count << 8) | 0xCA
}

pub const AUDIO_CONFIGURATION_STEREO: i32 = make_audio_configuration(2, 0x3);

pub const fn surround_audio_info(audio_configuration: i32) -> i32 {
    let count = (audio_configuration >> 8) & 0xFF;
    let mask = (audio_configuration >> 16) & 0xFFFF;
    (mask << 16) | count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_configuration_matches_c_macros() {
        // MAKE_AUDIO_CONFIGURATION(2, 0x3) and the value Moonlight sends as surroundAudioInfo.
        assert_eq!(AUDIO_CONFIGURATION_STEREO, 0x0003_02CA);
        assert_eq!(surround_audio_info(AUDIO_CONFIGURATION_STEREO), 196610);
    }
}
