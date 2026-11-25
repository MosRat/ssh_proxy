pub const QUIC_MAX_BIDI_STREAMS: u32 = 256;
pub const QUIC_STREAM_RECEIVE_WINDOW: u32 = 2 * 1024 * 1024;
pub const QUIC_RECEIVE_WINDOW: u32 = 16 * 1024 * 1024;
pub const QUIC_KEEP_ALIVE_INTERVAL_SECS: u64 = 10;
pub const QUIC_IDLE_TIMEOUT_SECS: u64 = 60;

pub fn default_quic_max_bidi_streams() -> u32 {
    QUIC_MAX_BIDI_STREAMS
}

pub fn default_quic_stream_receive_window() -> u32 {
    QUIC_STREAM_RECEIVE_WINDOW
}

pub fn default_quic_receive_window() -> u32 {
    QUIC_RECEIVE_WINDOW
}

pub fn default_quic_keep_alive_interval_secs() -> u64 {
    QUIC_KEEP_ALIVE_INTERVAL_SECS
}

pub fn default_quic_idle_timeout_secs() -> u64 {
    QUIC_IDLE_TIMEOUT_SECS
}
