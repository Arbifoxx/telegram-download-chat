use serde::{Deserialize, Serialize};
use std::fs;
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ServiceKind {
    #[default]
    Telegram,
    YouTube,
    Spotify,
    ArchiveOrg,
    WebsiteCopy,
    GenericDownload,
}

impl ServiceKind {
    pub const ALL: [ServiceKind; 6] = [
        ServiceKind::Telegram,
        ServiceKind::YouTube,
        ServiceKind::Spotify,
        ServiceKind::ArchiveOrg,
        ServiceKind::WebsiteCopy,
        ServiceKind::GenericDownload,
    ];

    pub fn badge(self) -> &'static str {
        match self {
            ServiceKind::Telegram => "TG",
            ServiceKind::YouTube => "YT",
            ServiceKind::Spotify => "SP",
            ServiceKind::ArchiveOrg => "AR",
            ServiceKind::WebsiteCopy => "WB",
            ServiceKind::GenericDownload => "DL",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            ServiceKind::Telegram => "Telegram",
            ServiceKind::YouTube => "YouTube",
            ServiceKind::Spotify => "Spotify",
            ServiceKind::ArchiveOrg => "Archive.org",
            ServiceKind::WebsiteCopy => "Website Copy",
            ServiceKind::GenericDownload => "Generic",
        }
    }

    pub fn is_available(self) -> bool {
        matches!(self, ServiceKind::Telegram | ServiceKind::YouTube)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum YoutubeQuality {
    P2160,
    P1440,
    P1080,
    #[default]
    P720,
    P480,
    P360,
    AudioOnly,
}

impl YoutubeQuality {
    pub const ALL: [YoutubeQuality; 7] = [
        YoutubeQuality::P2160,
        YoutubeQuality::P1440,
        YoutubeQuality::P1080,
        YoutubeQuality::P720,
        YoutubeQuality::P480,
        YoutubeQuality::P360,
        YoutubeQuality::AudioOnly,
    ];
}

impl fmt::Display for YoutubeQuality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            YoutubeQuality::P2160 => "2160p",
            YoutubeQuality::P1440 => "1440p",
            YoutubeQuality::P1080 => "1080p",
            YoutubeQuality::P720 => "720p",
            YoutubeQuality::P480 => "480p",
            YoutubeQuality::P360 => "360p",
            YoutubeQuality::AudioOnly => "Audio Only",
        };
        f.write_str(label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum YoutubeFormat {
    #[default]
    Mp4,
    Mkv,
    Webm,
    Mp3,
}

impl YoutubeFormat {
    pub const ALL: [YoutubeFormat; 4] = [
        YoutubeFormat::Mp4,
        YoutubeFormat::Mkv,
        YoutubeFormat::Webm,
        YoutubeFormat::Mp3,
    ];
}

impl fmt::Display for YoutubeFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            YoutubeFormat::Mp4 => "mp4",
            YoutubeFormat::Mkv => "mkv",
            YoutubeFormat::Webm => "webm",
            YoutubeFormat::Mp3 => "mp3",
        };
        f.write_str(label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum YoutubeCodec {
    #[default]
    Vp9,
    Vp8,
    Avc,
    Hevc,
    Av1,
}

impl YoutubeCodec {
    pub const ALL: [YoutubeCodec; 5] = [
        YoutubeCodec::Vp9,
        YoutubeCodec::Vp8,
        YoutubeCodec::Avc,
        YoutubeCodec::Hevc,
        YoutubeCodec::Av1,
    ];
}

impl fmt::Display for YoutubeCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            YoutubeCodec::Vp9 => "VP9",
            YoutubeCodec::Vp8 => "VP8",
            YoutubeCodec::Avc => "AVC",
            YoutubeCodec::Hevc => "HEVC",
            YoutubeCodec::Av1 => "AV1",
        };
        f.write_str(label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum YoutubeCookies {
    #[default]
    None,
    Brave,
    Chrome,
    Chromium,
    Edge,
    Opera,
    Vivaldi,
    Whale,
    Firefox,
    Safari,
}

impl YoutubeCookies {
    pub const ALL: [YoutubeCookies; 10] = [
        YoutubeCookies::None,
        YoutubeCookies::Brave,
        YoutubeCookies::Chrome,
        YoutubeCookies::Chromium,
        YoutubeCookies::Edge,
        YoutubeCookies::Opera,
        YoutubeCookies::Vivaldi,
        YoutubeCookies::Whale,
        YoutubeCookies::Firefox,
        YoutubeCookies::Safari,
    ];
}

impl fmt::Display for YoutubeCookies {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            YoutubeCookies::None => "None",
            YoutubeCookies::Brave => "Brave",
            YoutubeCookies::Chrome => "Chrome",
            YoutubeCookies::Chromium => "Chromium",
            YoutubeCookies::Edge => "Edge",
            YoutubeCookies::Opera => "Opera",
            YoutubeCookies::Vivaldi => "Vivaldi",
            YoutubeCookies::Whale => "Whale",
            YoutubeCookies::Firefox => "Firefox",
            YoutubeCookies::Safari => "Safari",
        };
        f.write_str(label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DownloadMode {
    #[default]
    Stopped,
    Running,
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveDownload {
    pub name: String,
    pub progress: f32,
    pub transferred_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub active_service: ServiceKind,
    pub chat_input: String,
    pub output_path: String,
    pub youtube_url: String,
    pub youtube_output_path: String,
    pub youtube_quality: Option<YoutubeQuality>,
    pub youtube_format: YoutubeFormat,
    pub youtube_codec: Option<YoutubeCodec>,
    pub youtube_cookies: YoutubeCookies,
    pub sort_descending: bool,
    pub debug_mode: bool,
    pub overwrite_existing: bool,
    pub download_media: bool,
    pub html_export: bool,
    pub pdf_export: bool,
    pub concurrent_downloads: u8,
    pub api_id: String,
    pub api_hash: String,
    pub phone_number: String,
    pub status_message: String,
    pub download_mode: DownloadMode,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            active_service: ServiceKind::Telegram,
            chat_input: String::new(),
            output_path: String::new(),
            youtube_url: String::new(),
            youtube_output_path: String::new(),
            youtube_quality: None,
            youtube_format: YoutubeFormat::Mp4,
            youtube_codec: Some(YoutubeCodec::Vp9),
            youtube_cookies: YoutubeCookies::None,
            sort_descending: false,
            debug_mode: false,
            overwrite_existing: true,
            download_media: true,
            html_export: true,
            pdf_export: true,
            concurrent_downloads: 5,
            api_id: String::new(),
            api_hash: String::new(),
            phone_number: String::new(),
            status_message: String::new(),
            download_mode: DownloadMode::Stopped,
        }
    }
}

pub fn load() -> AppConfig {
    let path = config_path();
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save(config: &AppConfig) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(raw) = serde_json::to_string_pretty(config) {
        let _ = fs::write(path, raw);
    }
}

pub fn config_path() -> PathBuf {
    let base = dirs::config_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("telegram-download-chat").join("tdc-app.json")
}
