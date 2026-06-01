pub struct ServerConfig;

impl ServerConfig {
    pub const DISCOVERY_PORT: u16 = 45454;
    pub const COMMAND_PORT: u16 = 45455;
    pub const DISCOVER_MESSAGE: &'static str = "DISCOVER";
    pub const DISCOVERY_BUFFER_SIZE: usize = 1024;
    pub const COMMAND_BUFFER_SIZE: usize = 4096;
    pub const UNKNOWN_HOSTNAME: &'static str = "Unknown";

    // Input simulation delays
    pub const MOUSE_CLICK_DELAY_MS: u64 = 10;
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub const DOUBLE_CLICK_TIMEOUT_MS: u64 = 350;
    pub const FALLBACK_SCREEN_WIDTH: f64 = 1920.0;
    pub const FALLBACK_SCREEN_HEIGHT: f64 = 1080.0;
}
