mod config;
mod downloader;
mod telegram;

use config::{
    ActiveDownload, AppConfig, DownloadMode, ServiceKind, YoutubeCodec, YoutubeCookies,
    YoutubeFormat, YoutubeQuality,
};
use downloader::{start_telegram_download, DownloadController, TelegramDownloadParams};
use iced::theme::{self, Palette};
use iced::widget::{button, checkbox, column, container, pick_list, progress_bar, row, scrollable, text, text_input, Column, Container, Space};
use iced::{time, window, Alignment, Application, Color, Command, Element, Length, Settings, Subscription, Theme};
use std::time::{Duration, Instant};
use telegram::{AuthAction, AuthResult, AuthTokens};

const WINDOW_WIDTH: u32 = 500;
const WINDOW_HEIGHT: u32 = 745;
const RAIL_WIDTH: f32 = 52.0;
const PANEL_RADIUS: f32 = 14.0;
const CONTROL_RADIUS: f32 = 9.0;
const STREAM_OPTIONS: [u8; 5] = [1, 2, 3, 4, 5];
const SORT_OPTIONS: [&str; 2] = ["Ascending", "Descending"];
const YOUTUBE_QUALITY_NONE: [&str; 1] = ["None"];

pub fn main() -> iced::Result {
    TdcApp::run(Settings {
        antialiasing: true,
        default_text_size: iced::Pixels(15.0),
        window: window::Settings {
            size: iced::Size::new(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32),
            min_size: Some(iced::Size::new(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32)),
            max_size: Some(iced::Size::new(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32)),
            resizable: false,
            ..window::Settings::default()
        },
        ..Settings::default()
    })
}

#[derive(Debug, Clone)]
enum Message {
    Tick,
    Noop,
    ServiceSelected(ServiceKind),
    ChatChanged(String),
    OutputPathChanged(String),
    ChooseOutputPressed,
    OutputFolderChosen(Option<String>),
    OverwriteToggled(bool),
    HtmlToggled(bool),
    PdfToggled(bool),
    ConcurrentStreamsSelected(u8),
    SortDescendingSelected(bool),
    CredentialsPressed,
    CredentialsReturnPressed,
    ApiIdChanged(String),
    ApiHashChanged(String),
    PhoneNumberChanged(String),
    CodeChanged(String),
    PasswordChanged(String),
    SaveApiCredentialsPressed,
    RequestCodePressed,
    LogoutPressed,
    TelegramAuthFinished(AuthResult),
    TelegramDownloadStarted(Result<DownloadController, String>),
    ClearTelegramStatus,
    ResetSaveButton,
    YoutubeUrlChanged(String),
    YoutubeOutputPathChanged(String),
    ChooseYoutubeOutputPressed,
    YoutubeOutputFolderChosen(Option<String>),
    YoutubeQualitySelected(YoutubeQuality),
    YoutubeFormatSelected(YoutubeFormat),
    YoutubeCodecSelected(YoutubeCodec),
    YoutubeCookiesSelected(YoutubeCookies),
    YoutubeInfoPressed,
    StartPressed,
    PausePressed,
    StopPressed,
}

struct TdcApp {
    config: AppConfig,
    downloads: Vec<ActiveDownload>,
    credentials_open: bool,
    credentials_form: CredentialsForm,
    auth_tokens: AuthTokens,
    telegram_authorized: bool,
    telegram_status_message: String,
    telegram_identity_label: String,
    telegram_status_deadline: Option<Instant>,
    downloader_started_status_seen: bool,
    auth_busy: bool,
    save_button_state: SaveButtonState,
    request_button_state: RequestButtonState,
    download_controller: Option<DownloadController>,
}

#[derive(Debug, Clone, Default)]
struct CredentialsForm {
    api_id: String,
    api_hash: String,
    phone_number: String,
    code: String,
    password: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum SaveButtonState {
    #[default]
    Idle,
    Saving,
    Saved,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum RequestButtonState {
    #[default]
    RequestCode,
    Requesting,
    LogIn,
    LoggingIn,
}

impl Application for TdcApp {
    type Executor = iced::executor::Default;
    type Message = Message;
    type Theme = Theme;
    type Flags = ();

    fn new(_flags: ()) -> (Self, Command<Self::Message>) {
        let mut config = config::load();
        config.download_mode = DownloadMode::Stopped;
        config.status_message.clear();
        let credentials_form = CredentialsForm {
            api_id: config.api_id.clone(),
            api_hash: config.api_hash.clone(),
            phone_number: config.phone_number.clone(),
            code: String::new(),
            password: String::new(),
        };
        let initial_check = if !config.api_id.trim().is_empty() && !config.api_hash.trim().is_empty() {
            Command::perform(
                telegram::check_authorized(config.api_id.clone(), config.api_hash.clone()),
                Message::TelegramAuthFinished,
            )
        } else {
            Command::none()
        };
        (
            Self {
                config,
                downloads: Vec::new(),
                credentials_open: false,
                credentials_form,
                auth_tokens: AuthTokens::new(),
                telegram_authorized: false,
                telegram_status_message: String::new(),
                telegram_identity_label: String::new(),
                telegram_status_deadline: None,
                downloader_started_status_seen: false,
                auth_busy: false,
                save_button_state: SaveButtonState::Idle,
                request_button_state: RequestButtonState::RequestCode,
                download_controller: None,
            },
            initial_check,
        )
    }

    fn title(&self) -> String {
        "Telegram Download Chat".to_string()
    }

    fn theme(&self) -> Self::Theme {
        Theme::custom(
            "TDC Compact".to_string(),
            Palette {
                background: bg(),
                text: text_primary(),
                primary: accent(),
                success: success(),
                danger: danger(),
            },
        )
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        time::every(std::time::Duration::from_millis(200)).map(|_| Message::Tick)
    }

    fn update(&mut self, message: Self::Message) -> Command<Self::Message> {
        match message {
            Message::Tick => {
                if let Some(deadline) = self.telegram_status_deadline {
                    if Instant::now() >= deadline {
                        self.telegram_status_deadline = None;
                        if self.telegram_status_message == "Downloader started." {
                            self.telegram_status_message.clear();
                            self.config.status_message.clear();
                        }
                    }
                }
                if let Some(controller) = &self.download_controller {
                    let snapshot = controller.snapshot();
                    self.apply_download_snapshot(snapshot);
                }
            }
            Message::Noop => {}
            Message::ServiceSelected(service) => self.config.active_service = service,
            Message::ChatChanged(value) => self.config.chat_input = value,
            Message::OutputPathChanged(value) => self.config.output_path = value,
            Message::ChooseOutputPressed => {
                return Command::perform(pick_folder(), Message::OutputFolderChosen);
            }
            Message::OutputFolderChosen(path) => {
                if let Some(path) = path {
                    self.config.output_path = path;
                }
            }
            Message::OverwriteToggled(value) => self.config.overwrite_existing = value,
            Message::HtmlToggled(value) => self.config.html_export = value,
            Message::PdfToggled(value) => self.config.pdf_export = value,
            Message::ConcurrentStreamsSelected(value) => self.config.concurrent_downloads = value,
            Message::SortDescendingSelected(value) => self.config.sort_descending = value,
            Message::CredentialsPressed => {
                self.credentials_open = true;
                self.credentials_form.api_id = self.config.api_id.clone();
                self.credentials_form.api_hash = self.config.api_hash.clone();
                self.credentials_form.phone_number = self.config.phone_number.clone();
            }
            Message::CredentialsReturnPressed => {
                self.credentials_open = false;
                self.auth_busy = false;
                self.save_button_state = SaveButtonState::Idle;
            }
            Message::ApiIdChanged(value) => self.credentials_form.api_id = value,
            Message::ApiHashChanged(value) => self.credentials_form.api_hash = value,
            Message::PhoneNumberChanged(value) => self.credentials_form.phone_number = value,
            Message::CodeChanged(value) => self.credentials_form.code = value,
            Message::PasswordChanged(value) => self.credentials_form.password = value,
            Message::SaveApiCredentialsPressed => {
                self.config.api_id = self.credentials_form.api_id.trim().to_string();
                self.config.api_hash = self.credentials_form.api_hash.trim().to_string();
                self.config.phone_number = self.credentials_form.phone_number.trim().to_string();
                self.auth_busy = true;
                self.save_button_state = SaveButtonState::Saving;
                config::save(&self.config);
                return Command::perform(
                    telegram::save_api_credentials(
                        self.credentials_form.api_id.clone(),
                        self.credentials_form.api_hash.clone(),
                    ),
                    Message::TelegramAuthFinished,
                );
            }
            Message::RequestCodePressed => {
                self.config.api_id = self.credentials_form.api_id.trim().to_string();
                self.config.api_hash = self.credentials_form.api_hash.trim().to_string();
                self.config.phone_number = self.credentials_form.phone_number.trim().to_string();
                self.auth_busy = true;
                self.request_button_state = match self.request_button_state {
                    RequestButtonState::LogIn | RequestButtonState::LoggingIn => {
                        self.telegram_status_message = "Logging in...".to_string();
                        RequestButtonState::LoggingIn
                    }
                    RequestButtonState::Requesting | RequestButtonState::RequestCode => {
                        self.telegram_status_message = "Requesting code...".to_string();
                        RequestButtonState::Requesting
                    }
                };
                config::save(&self.config);
                return Command::perform(
                    telegram::request_code_or_sign_in(
                        self.credentials_form.api_id.clone(),
                        self.credentials_form.api_hash.clone(),
                        self.credentials_form.phone_number.clone(),
                        self.credentials_form.code.clone(),
                        self.credentials_form.password.clone(),
                        self.auth_tokens.clone(),
                    ),
                    Message::TelegramAuthFinished,
                );
            }
            Message::LogoutPressed => {
                self.auth_busy = true;
                return Command::perform(
                    telegram::logout(
                        self.credentials_form.api_id.clone(),
                        self.credentials_form.api_hash.clone(),
                        self.auth_tokens.clone(),
                    ),
                    Message::TelegramAuthFinished,
                );
            }
            Message::ClearTelegramStatus => {
                self.telegram_status_message.clear();
                self.config.status_message.clear();
            }
            Message::ResetSaveButton => {
                self.save_button_state = SaveButtonState::Idle;
            }
            Message::TelegramAuthFinished(result) => {
                self.auth_busy = false;
                self.telegram_authorized = result.authorized;
                self.telegram_status_message = result.message.clone();
                self.telegram_status_deadline = None;
                if let Some(identity_label) = result.identity_label {
                    self.telegram_identity_label = identity_label;
                }
                self.config.status_message = result.message;
                let mut commands = vec![clear_status_command()];
                match result.action {
                    AuthAction::ApiSaved => {
                        self.save_button_state = SaveButtonState::Saved;
                        commands.push(reset_save_button_command());
                    }
                    AuthAction::CodeRequested => {
                        self.request_button_state = RequestButtonState::LogIn;
                        self.credentials_open = true;
                    }
                    AuthAction::SignedIn => {
                        self.request_button_state = RequestButtonState::RequestCode;
                        self.credentials_form.code.clear();
                        self.credentials_form.password.clear();
                        self.credentials_open = false;
                    }
                    AuthAction::LoggedOut => {
                        self.request_button_state = RequestButtonState::RequestCode;
                        self.credentials_form.code.clear();
                        self.credentials_form.password.clear();
                        self.credentials_open = true;
                        self.telegram_identity_label.clear();
                    }
                    AuthAction::None => {
                        if self.save_button_state == SaveButtonState::Saving {
                            self.save_button_state = SaveButtonState::Idle;
                        }
                        self.request_button_state = match self.request_button_state {
                            RequestButtonState::Requesting => RequestButtonState::RequestCode,
                            RequestButtonState::LoggingIn => RequestButtonState::LogIn,
                            other => other,
                        };
                        self.credentials_open = result.keep_popup_open;
                    }
                }
                if matches!(result.action, AuthAction::None) && result.authorized {
                    self.credentials_form.code.clear();
                    self.credentials_form.password.clear();
                    self.credentials_open = false;
                } else if !matches!(result.action, AuthAction::CodeRequested | AuthAction::LoggedOut) {
                    self.credentials_open = if result.authorized {
                        false
                    } else {
                        result.keep_popup_open
                    };
                }
                config::save(&self.config);
                return Command::batch(commands);
            }
            Message::YoutubeUrlChanged(value) => self.config.youtube_url = value,
            Message::YoutubeOutputPathChanged(value) => self.config.youtube_output_path = value,
            Message::ChooseYoutubeOutputPressed => {
                return Command::perform(pick_folder(), Message::YoutubeOutputFolderChosen);
            }
            Message::YoutubeOutputFolderChosen(path) => {
                if let Some(path) = path {
                    self.config.youtube_output_path = path;
                }
            }
            Message::YoutubeQualitySelected(value) => self.config.youtube_quality = Some(value),
            Message::YoutubeFormatSelected(value) => {
                self.config.youtube_format = value;
                if value == YoutubeFormat::Mp3 {
                    self.config.youtube_codec = None;
                } else if self.config.youtube_codec.is_none() {
                    self.config.youtube_codec = Some(YoutubeCodec::Vp9);
                }
            }
            Message::YoutubeCodecSelected(value) => self.config.youtube_codec = Some(value),
            Message::YoutubeCookiesSelected(value) => self.config.youtube_cookies = value,
            Message::YoutubeInfoPressed => {}
            Message::TelegramDownloadStarted(result) => match result {
                Ok(controller) => {
                    self.download_controller = Some(controller);
                }
                Err(error) => {
                    eprintln!("Failed to start Telegram download: {error}");
                    self.config.download_mode = DownloadMode::Stopped;
                    self.telegram_status_message.clear();
                    return clear_status_command();
                }
            },
            Message::StartPressed => {
                if self.config.active_service == ServiceKind::Telegram
                    && (self.config.api_id.trim().is_empty() || self.config.api_hash.trim().is_empty())
                {
                    self.credentials_open = true;
                    self.telegram_status_message = "Save Telegram API credentials before starting a download.".to_string();
                    self.config.status_message = self.telegram_status_message.clone();
                    return clear_status_command();
                } else if self.config.active_service == ServiceKind::Telegram {
                    self.config.download_mode = DownloadMode::Running;
                    self.telegram_status_message = "Preparing Telegram download...".to_string();
                    self.downloader_started_status_seen = false;
                    return Command::perform(
                        start_telegram_download(TelegramDownloadParams {
                            api_id: self.config.api_id.clone(),
                            api_hash: self.config.api_hash.clone(),
                            chat_input: self.config.chat_input.clone(),
                            output_path: self.config.output_path.clone(),
                            overwrite_existing: self.config.overwrite_existing,
                            concurrent_downloads: self.config.concurrent_downloads,
                            sort_descending: self.config.sort_descending,
                        }),
                        Message::TelegramDownloadStarted,
                    );
                } else {
                    self.config.download_mode = DownloadMode::Running;
                }
            }
            Message::PausePressed => {
                self.config.download_mode = match self.config.download_mode {
                    DownloadMode::Stopped => DownloadMode::Stopped,
                    DownloadMode::Running => {
                        if let Some(controller) = &self.download_controller {
                            controller.pause();
                        }
                        DownloadMode::Paused
                    }
                    DownloadMode::Paused => {
                        if let Some(controller) = &self.download_controller {
                            controller.resume();
                        }
                        DownloadMode::Running
                    }
                }
            }
            Message::StopPressed => {
                if let Some(controller) = &self.download_controller {
                    controller.stop();
                }
                self.download_controller = None;
                self.downloads.clear();
                self.config.download_mode = DownloadMode::Stopped;
                self.downloader_started_status_seen = false;
            }
        }

        config::save(&self.config);
        Command::none()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let layout = row![self.service_rail(), divider(), self.active_screen()]
            .spacing(9)
            .width(Length::Fill)
            .height(Length::Fill);

        container(layout)
            .padding([8, 8, 8, 6])
            .style(shell_style())
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

impl TdcApp {
    fn apply_download_snapshot(&mut self, snapshot: downloader::DownloadSnapshot) {
        if self.download_controller.is_none() {
            return;
        }
        self.downloads = snapshot.files;
        self.config.download_mode = snapshot.mode;
        if !snapshot.status_message.is_empty() {
            if snapshot.status_message == "Downloader started." {
                if !self.downloader_started_status_seen {
                    self.downloader_started_status_seen = true;
                    self.telegram_status_deadline = Some(Instant::now() + Duration::from_secs(3));
                    self.telegram_status_message = snapshot.status_message;
                }
            } else {
                self.telegram_status_deadline = None;
                self.telegram_status_message = snapshot.status_message;
            }
        }
        if snapshot.finished && self.config.download_mode == DownloadMode::Stopped {
            self.download_controller = None;
            self.downloader_started_status_seen = false;
        }
    }

    fn active_screen(&self) -> Element<'_, Message> {
        match self.config.active_service {
            ServiceKind::Telegram => self.telegram_screen(),
            ServiceKind::YouTube => self.youtube_screen(),
            _ => self.coming_soon_screen(),
        }
    }

    fn service_rail(&self) -> Element<'_, Message> {
        let rail = ServiceKind::ALL.into_iter().fold(
            column![].spacing(8).align_items(Alignment::Center),
            |column, service| column.push(self.service_tile(service)),
        );

        container(rail)
            .width(Length::Fixed(RAIL_WIDTH))
            .height(Length::Fill)
            .center_x()
            .center_y()
            .into()
    }

    fn service_tile(&self, service: ServiceKind) -> Element<'_, Message> {
        let active = self.config.active_service == service;
        let button = button(
            container(text(service.badge()).size(18))
                .center_x()
                .center_y()
                .width(Length::Fill)
                .height(Length::Fill),
        )
            .width(Length::Fixed(38.0))
            .height(Length::Fixed(52.0))
            .padding(0)
            .style(service_button_style(active, service.is_available()));

        let button = if service.is_available() {
            button.on_press(Message::ServiceSelected(service))
        } else {
            button
        };

        container(button).into()
    }

    fn telegram_screen(&self) -> Element<'_, Message> {
        let body = scrollable(
            column![
                self.field_panel("URL", "", &self.config.chat_input, Message::ChatChanged),
                self.output_field_panel("Output", "Choose a destination folder", &self.config.output_path, Message::OutputPathChanged, Message::ChooseOutputPressed),
                self.options_panel(),
                self.files_panel(),
            ]
            .spacing(6),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(hidden_scrollable_style());

        let main_content: Element<'_, Message> = if self.credentials_open {
            container(self.credentials_popup())
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            column![body, self.bottom_controls()]
                .spacing(6)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        };

        container(
            column![
                self.title_panel(),
                main_content,
            ]
            .spacing(6),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn youtube_screen(&self) -> Element<'_, Message> {
        let body = scrollable(
            column![
                self.field_panel("URL", "https://www.youtube.com/watch?v=...", &self.config.youtube_url, Message::YoutubeUrlChanged),
                self.preview_panel(),
                self.youtube_quality_panel(),
                self.selector_panel("Format", self.config.youtube_format, &YoutubeFormat::ALL, Message::YoutubeFormatSelected),
                self.codec_panel(),
                self.selector_panel("Cookies", self.config.youtube_cookies, &YoutubeCookies::ALL, Message::YoutubeCookiesSelected),
                self.output_field_panel("Output", "Choose a destination folder", &self.config.youtube_output_path, Message::YoutubeOutputPathChanged, Message::ChooseYoutubeOutputPressed),
                self.youtube_progress_panel(),
            ]
            .spacing(6),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(hidden_scrollable_style());

        container(
            column![
                self.title_panel(),
                body,
                self.youtube_action_button(),
            ]
            .spacing(6),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn coming_soon_screen(&self) -> Element<'_, Message> {
        let service = self.config.active_service.title();
        container(
            column![
                self.title_panel(),
                panel(
                    column![
                        self.section_title(service),
                        text("This downloader is staged for the future native suite.")
                            .size(13)
                            .style(theme::Text::Color(text_muted())),
                    ]
                    .spacing(8),
                ),
            ]
            .spacing(6),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn title_panel(&self) -> Element<'_, Message> {
        panel(
            row![
                column![
                    text(self.config.active_service.title()).size(22),
                    text(if self.config.active_service == ServiceKind::Telegram {
                        if !self.telegram_status_message.is_empty() {
                            &self.telegram_status_message
                        } else {
                            &self.telegram_identity_label
                        }
                    } else {
                        ""
                    })
                    .size(10)
                    .style(theme::Text::Color(text_muted())),
                ]
                .spacing(2)
                .width(Length::Fill),
                status_chip(self.config.download_mode),
            ]
            .align_items(Alignment::Center),
        )
        .into()
    }

    fn preview_panel(&self) -> Element<'_, Message> {
        panel(
            column![
                self.section_title("Preview"),
                container(text("Thumbnail preview will appear here").size(12).style(theme::Text::Color(text_subtle())))
                    .width(Length::Fill)
                    .height(Length::Fixed(132.0))
                    .center_x()
                    .center_y()
                    .style(panel_style(color(0x62, 0x63, 0x69), border_soft(), CONTROL_RADIUS)),
            ]
            .spacing(8),
        )
        .into()
    }

    fn field_panel<'a>(
        &self,
        label: &'a str,
        placeholder: &'a str,
        value: &'a str,
        on_input: fn(String) -> Message,
    ) -> Element<'a, Message> {
        panel(
            column![
                text(label).size(12).style(theme::Text::Color(text_muted())),
                text_input(placeholder, value)
                    .on_input(on_input)
                    .padding([7, 9])
                    .size(13)
                    .style(text_input_style()),
            ]
            .spacing(3),
        )
        .into()
    }

    fn output_field_panel<'a>(
        &self,
        label: &'a str,
        placeholder: &'a str,
        value: &'a str,
        on_input: fn(String) -> Message,
        choose_message: Message,
    ) -> Element<'a, Message> {
        panel(
            column![
                text(label).size(12).style(theme::Text::Color(text_muted())),
                row![
                    text_input(placeholder, value)
                        .on_input(on_input)
                        .padding([7, 9])
                        .size(13)
                        .style(text_input_style())
                        .width(Length::Fill),
                    button(
                        container(text("Choose...").size(12))
                            .center_x()
                            .center_y()
                            .width(Length::Fill)
                    )
                    .width(Length::Fixed(94.0))
                    .padding([7, 8])
                    .style(secondary_button_style())
                    .on_press(choose_message),
                ]
                .spacing(6)
                .align_items(Alignment::Center),
            ]
            .spacing(3),
        )
        .into()
    }

    fn options_panel(&self) -> Element<'_, Message> {
        let sort_label = if self.config.sort_descending {
            "Descending"
        } else {
            "Ascending"
        };

        panel(
            column![
                column![
                    self.section_title("Options"),
                    row![
                        container(
                            column![
                                option_row("Replace Files", self.config.overwrite_existing, Message::OverwriteToggled),
                                option_row("Output PDF", self.config.pdf_export, Message::PdfToggled),
                                option_row("Output HTML", self.config.html_export, Message::HtmlToggled),
                            ]
                            .spacing(6)
                            .width(Length::Fixed(206.0))
                        )
                        .width(Length::FillPortion(1))
                        .align_x(iced::alignment::Horizontal::Left),
                        vertical_rule(),
                        container(
                            column![
                                stacked_control(
                                    "Concurrent Streams",
                                    pick_list(
                                        STREAM_OPTIONS,
                                        Some(self.config.concurrent_downloads),
                                        Message::ConcurrentStreamsSelected,
                                    )
                                    .padding([6, 9])
                                    .text_size(13)
                                    .width(Length::Fixed(128.0))
                                    .into(),
                                ),
                                stacked_control(
                                    "Sort",
                                    pick_list(
                                        SORT_OPTIONS,
                                        Some(sort_label),
                                        |value| Message::SortDescendingSelected(value == "Descending"),
                                    )
                                    .padding([6, 9])
                                    .text_size(13)
                                    .width(Length::Fixed(128.0))
                                    .into(),
                                ),
                            ]
                            .spacing(3)
                            .width(Length::Fixed(128.0))
                            .align_items(Alignment::Center)
                        )
                        .width(Length::FillPortion(1))
                        .align_x(iced::alignment::Horizontal::Center),
                    ]
                    .spacing(14)
                    .width(Length::Fill)
                    .align_items(Alignment::Start),
                    credentials_button_full(),
                ]
                .spacing(8),
            ]
        )
        .into()
    }

    fn selector_panel<'a, T: Copy + Eq + std::fmt::Display + 'static>(
        &self,
        label: &'a str,
        selected: T,
        options: &'a [T],
        on_selected: fn(T) -> Message,
    ) -> Element<'a, Message> {
        panel(
            column![
                text(label).size(12).style(theme::Text::Color(text_muted())),
                pick_list(options, Some(selected), on_selected)
                    .padding([7, 9])
                    .text_size(13)
                    .width(Length::Fill),
            ]
            .spacing(3),
        )
        .into()
    }

    fn youtube_quality_panel(&self) -> Element<'_, Message> {
        panel(
            column![
                text("Quality").size(12).style(theme::Text::Color(text_muted())),
                if let Some(selected) = self.config.youtube_quality {
                    pick_list(
                        &YoutubeQuality::ALL[..],
                        Some(selected),
                        Message::YoutubeQualitySelected,
                    )
                    .padding([7, 9])
                    .text_size(13)
                    .width(Length::Fill)
                    .into()
                } else {
                    {
                        let placeholder: Element<'_, Message> = pick_list(
                            &YOUTUBE_QUALITY_NONE[..],
                            Some("None"),
                            |_: &str| Message::Noop,
                        )
                        .padding([7, 9])
                        .text_size(13)
                        .width(Length::Fill)
                        .into();
                        placeholder
                    }
                },
            ]
            .spacing(3),
        )
        .into()
    }

    fn codec_panel(&self) -> Element<'_, Message> {
        let codec_value = match self.config.youtube_format {
            YoutubeFormat::Mp3 => None,
            _ => self.config.youtube_codec,
        };

        panel(
            column![
                text("Codec").size(12).style(theme::Text::Color(text_muted())),
                if self.config.youtube_format == YoutubeFormat::Mp3 {
                    empty_select_box()
                } else {
                    pick_list(
                        &YoutubeCodec::ALL[..],
                        codec_value,
                        Message::YoutubeCodecSelected,
                    )
                    .padding([7, 9])
                    .text_size(13)
                    .width(Length::Fill)
                    .into()
                },
            ]
            .spacing(3),
        )
        .into()
    }

    fn youtube_progress_panel(&self) -> Element<'_, Message> {
        panel(
            column![
                self.section_title("Progress"),
                text("Ready to fetch video information").size(12).style(theme::Text::Color(text_muted())),
                panel_softened(
                    column![
                        row![
                            text("youtube_preview_download").size(11).width(Length::Fill),
                            text("0 MB / 0 MB   0%").size(10).style(theme::Text::Color(text_muted())),
                        ]
                        .align_items(Alignment::Center),
                        progress_bar(0.0..=1.0, 0.0).width(Length::Fill),
                    ]
                    .spacing(4),
                ),
            ]
            .spacing(6),
        )
        .into()
    }

    fn files_panel(&self) -> Element<'_, Message> {
        let rows = if self.downloads.is_empty() {
            Column::new()
                .spacing(6)
                .push(self.empty_file_row())
                .push(self.empty_file_row())
                .push(self.empty_file_row())
                .push(self.empty_file_row())
                .push(self.empty_file_row())
        } else {
            self.downloads
                .iter()
                .enumerate()
                .fold(Column::new().spacing(6), |column, (index, item)| {
                    column.push(self.file_row(index + 1, item))
                })
        };

        panel(
            column![
                self.section_title("Files:"),
                scrollable(rows)
                    .height(Length::Fixed(226.0))
                    .style(hidden_scrollable_style()),
            ]
            .spacing(6),
        )
        .into()
    }

    fn file_row(&self, index: usize, item: &ActiveDownload) -> Element<'_, Message> {
        panel_softened(
            column![
                row![
                    text(format!("file {}", index))
                        .size(11)
                        .width(Length::Fixed(34.0))
                        .style(theme::Text::Color(text_muted())),
                    text(&item.name)
                        .size(11)
                        .width(Length::Fill)
                        .style(theme::Text::Color(text_primary())),
                    text(&item.transferred_label)
                        .size(10)
                        .style(theme::Text::Color(text_muted())),
                ]
                .align_items(Alignment::Center),
                progress_bar(0.0..=1.0, item.progress).width(Length::Fill),
            ]
            .spacing(4),
        )
        .into()
    }

    fn empty_file_row(&self) -> Element<'_, Message> {
        panel_softened(
            container(text("").size(11))
                .height(Length::Fixed(56.0))
                .width(Length::Fill),
        )
        .into()
    }

    fn bottom_controls(&self) -> Element<'_, Message> {
        match self.config.download_mode {
            DownloadMode::Stopped => button(
                container(text("Start").size(16))
                    .center_x()
                    .center_y()
                    .width(Length::Fill),
            )
                .width(Length::Fill)
                .padding([12, 18])
                .style(primary_button_style())
                .on_press(Message::StartPressed)
                .into(),
            DownloadMode::Running | DownloadMode::Paused => row![
                button(
                    container(text(if self.config.download_mode == DownloadMode::Paused { "Resume" } else { "Pause" }).size(16))
                        .center_x()
                        .center_y()
                        .width(Length::Fill)
                )
                    .width(Length::Fill)
                    .padding([12, 18])
                    .style(secondary_button_style())
                    .on_press(Message::PausePressed),
                button(
                    container(text("Stop").size(16))
                        .center_x()
                        .center_y()
                        .width(Length::Fill)
                )
                    .width(Length::Fill)
                    .padding([12, 18])
                    .style(danger_button_style())
                    .on_press(Message::StopPressed),
            ]
            .spacing(8)
            .into(),
        }
    }

    fn youtube_action_button(&self) -> Element<'_, Message> {
        button(
            container(text("Get Video Information").size(16))
                .center_x()
                .center_y()
                .width(Length::Fill),
        )
        .width(Length::Fill)
        .padding([12, 18])
        .style(primary_button_style())
        .on_press(Message::YoutubeInfoPressed)
        .into()
    }

    fn section_title<'a>(&self, label: &'a str) -> Element<'a, Message> {
        text(label).size(17).into()
    }

    fn credentials_popup(&self) -> Element<'_, Message> {
        let save_label = match self.save_button_state {
            SaveButtonState::Idle | SaveButtonState::Saving => "Save API Credentials",
            SaveButtonState::Saved => "✓",
        };
        let request_label = match self.request_button_state {
            RequestButtonState::RequestCode => "Request Code",
            RequestButtonState::Requesting => "Requesting...",
            RequestButtonState::LogIn => "Log In",
            RequestButtonState::LoggingIn => "Logging In...",
        };

        panel(
            column![
                self.section_title("Credentials"),
                popup_field("API ID", "", &self.credentials_form.api_id, Message::ApiIdChanged, false),
                popup_field("API Hash", "", &self.credentials_form.api_hash, Message::ApiHashChanged, false),
                popup_button(save_label, Message::SaveApiCredentialsPressed, primary_button_style(), !self.auth_busy),
                popup_divider(),
                popup_field("Phone Number", "+12345678900", &self.credentials_form.phone_number, Message::PhoneNumberChanged, false),
                popup_field("Code", "", &self.credentials_form.code, Message::CodeChanged, false),
                popup_field("Password", "(Optional)", &self.credentials_form.password, Message::PasswordChanged, true),
                popup_button(request_label, Message::RequestCodePressed, primary_button_style(), !self.auth_busy),
                popup_button(
                    "Log Out",
                    Message::LogoutPressed,
                    if self.telegram_authorized {
                        danger_button_style()
                    } else {
                        danger_disabled_button_style()
                    },
                    self.telegram_authorized && !self.auth_busy,
                ),
                popup_divider_top_only(),
                container(
                    popup_tall_button(
                        "Return",
                        Message::CredentialsReturnPressed,
                        secondary_button_style(),
                        !self.auth_busy,
                    ),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .center_y(),
            ]
            .spacing(8)
            .height(Length::Fill),
        )
        .height(Length::Fill)
        .into()
    }
}

fn panel<'a, Message>(content: impl Into<Element<'a, Message>>) -> Container<'a, Message> {
    container(content)
        .padding([8, 8])
        .style(panel_style(panel_main(), border_soft(), PANEL_RADIUS))
}

fn empty_select_box<'a>() -> Element<'a, Message> {
    container(text("").size(13))
        .width(Length::Fill)
        .padding([7, 9])
        .style(panel_style(panel_soft(), border_soft(), CONTROL_RADIUS))
        .into()
}

fn panel_softened<'a, Message>(content: impl Into<Element<'a, Message>>) -> Container<'a, Message> {
    container(content)
        .padding([7, 8])
        .style(panel_style(panel_soft(), border_soft(), CONTROL_RADIUS))
}

fn option_row<'a>(
    label: &'a str,
    value: bool,
    on_toggle: fn(bool) -> Message,
) -> Element<'a, Message> {
    container(
        checkbox(label, value)
            .on_toggle(on_toggle)
            .spacing(8)
            .size(16)
            .text_size(16),
    )
    .height(Length::Fixed(28.0))
    .padding([0, 0, 0, 0])
    .align_y(iced::alignment::Vertical::Center)
    .into()
}

fn stacked_control<'a>(label: &'a str, control: Element<'a, Message>) -> Element<'a, Message> {
    column![
        container(text(label).size(11).style(theme::Text::Color(text_muted())))
            .width(Length::Fixed(128.0))
            .center_x(),
        control,
    ]
    .spacing(1)
    .align_items(Alignment::Center)
    .width(Length::Fixed(128.0))
    .into()
}

fn credentials_button_full<'a>() -> Element<'a, Message> {
    button(
        container(text("Credentials").size(13))
            .center_x()
            .center_y()
            .width(Length::Fill),
    )
    .padding([8, 12])
    .width(Length::Fill)
    .style(secondary_button_style())
    .on_press(Message::CredentialsPressed)
    .into()
}

fn popup_field<'a>(
    label: &'a str,
    placeholder: &'a str,
    value: &'a str,
    on_input: fn(String) -> Message,
    secure: bool,
) -> Element<'a, Message> {
    panel_softened(
        column![
            text(label).size(11).style(theme::Text::Color(text_muted())),
            text_input(placeholder, value)
                .on_input(on_input)
                .padding([7, 9])
                .size(13)
                .secure(secure)
                .style(text_input_style()),
        ]
        .spacing(3),
    )
    .into()
}

fn popup_button<'a>(
    label: &'a str,
    message: Message,
    style: theme::Button,
    enabled: bool,
) -> Element<'a, Message> {
    let mut button = button(
        container(text(label).size(13))
            .center_x()
            .center_y()
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .padding([8, 12])
    .style(style);
    if enabled {
        button = button.on_press(message);
    }
    button.into()
}

fn popup_tall_button<'a>(
    label: &'a str,
    message: Message,
    style: theme::Button,
    enabled: bool,
) -> Element<'a, Message> {
    let mut button = button(
        container(text(label).size(13))
            .center_x()
            .center_y()
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fixed(88.0))
    .padding([0, 12])
    .style(style);
    if enabled {
        button = button.on_press(message);
    }
    button.into()
}

fn horizontal_rule<'a>() -> Container<'a, Message> {
    container(Space::with_height(Length::Fixed(2.0)))
        .style(panel_style(color(0x27, 0x2B, 0x36), color(0x27, 0x2B, 0x36), 0.0))
        .width(Length::Fill)
        .height(Length::Fixed(2.0))
}

fn popup_divider<'a>() -> Element<'a, Message> {
    container(horizontal_rule())
        .padding([8, 0])
        .width(Length::Fill)
        .into()
}

fn popup_divider_top_only<'a>() -> Element<'a, Message> {
    container(horizontal_rule())
        .padding([8, 0, 0, 0])
        .width(Length::Fill)
        .into()
}

async fn pick_folder() -> Option<String> {
    rfd::FileDialog::new()
        .pick_folder()
        .map(|path| path.display().to_string())
}

fn hidden_scrollable_style() -> theme::Scrollable {
    theme::Scrollable::Custom(Box::new(HiddenScrollable))
}

struct HiddenScrollable;

impl iced::widget::scrollable::StyleSheet for HiddenScrollable {
    type Style = Theme;

    fn active(&self, _style: &Self::Style) -> iced::widget::scrollable::Appearance {
        iced::widget::scrollable::Appearance {
            container: iced::widget::container::Appearance::default(),
            scrollbar: iced::widget::scrollable::Scrollbar {
                background: None,
                border: iced::Border::default(),
                scroller: iced::widget::scrollable::Scroller {
                    color: Color::TRANSPARENT,
                    border: iced::Border::default(),
                },
            },
            gap: None,
        }
    }

    fn hovered(
        &self,
        style: &Self::Style,
        _is_mouse_over_scrollbar: bool,
    ) -> iced::widget::scrollable::Appearance {
        self.active(style)
    }

    fn dragging(&self, style: &Self::Style) -> iced::widget::scrollable::Appearance {
        self.active(style)
    }
}

fn status_chip<'a>(mode: DownloadMode) -> Container<'a, Message> {
    let (label, fill, border) = match mode {
        DownloadMode::Stopped => ("Idle", panel_soft(), border_soft()),
        DownloadMode::Running => ("Running", accent_soft(), accent()),
        DownloadMode::Paused => ("Paused", color(0x4A, 0x40, 0x2A), color(0xD3, 0x97, 0x45)),
    };

    container(text(label).size(12))
        .padding([8, 14])
        .style(panel_style(fill, border, 999.0))
}

fn vertical_rule<'a>() -> Container<'a, Message> {
    container(Space::with_width(Length::Fixed(1.0)))
        .style(panel_style(color(0x27, 0x2B, 0x36), color(0x27, 0x2B, 0x36), 0.0))
        .width(Length::Fixed(1.0))
        .height(Length::Fixed(106.0))
}

fn divider<'a>() -> Container<'a, Message> {
    container(Space::with_width(Length::Fixed(1.0)))
        .style(panel_style(color(0x27, 0x2B, 0x36), color(0x27, 0x2B, 0x36), 0.0))
        .width(Length::Fixed(1.0))
        .height(Length::Fill)
}

fn shell_style() -> theme::Container {
    panel_style(bg(), bg(), 0.0)
}

fn panel_style(background: Color, border: Color, radius: f32) -> theme::Container {
    theme::Container::Custom(Box::new(move |_theme: &Theme| iced::widget::container::Appearance {
        background: Some(background.into()),
        text_color: Some(text_primary()),
        border: iced::Border {
            color: border,
            width: 1.0,
            radius: radius.into(),
        },
        shadow: iced::Shadow {
            color: Color { a: 0.10, ..Color::BLACK },
            offset: iced::Vector::new(0.0, 4.0),
            blur_radius: 12.0,
        },
    }))
}

fn service_button_style(active: bool, available: bool) -> theme::Button {
    if active {
        theme::Button::Custom(Box::new(PillButton { fill: accent(), border: accent(), text: color(0x10, 0x0D, 0x15) }))
    } else if available {
        theme::Button::Custom(Box::new(PillButton { fill: panel_alt(), border: border_soft(), text: text_primary() }))
    } else {
        theme::Button::Custom(Box::new(PillButton { fill: color(0x55, 0x54, 0x5F), border: color(0x65, 0x64, 0x6E), text: color(0xE7, 0xE3, 0xF3) }))
    }
}

fn primary_button_style() -> theme::Button {
    theme::Button::Custom(Box::new(PillButton { fill: accent(), border: accent(), text: color(0x10, 0x0D, 0x15) }))
}

fn secondary_button_style() -> theme::Button {
    theme::Button::Custom(Box::new(PillButton { fill: color(0x70, 0x6B, 0x7E), border: color(0x70, 0x6B, 0x7E), text: text_primary() }))
}

fn danger_button_style() -> theme::Button {
    theme::Button::Custom(Box::new(PillButton { fill: danger(), border: danger(), text: color(0x10, 0x0D, 0x12) }))
}

fn danger_disabled_button_style() -> theme::Button {
    theme::Button::Custom(Box::new(PillButton {
        fill: color(0x42, 0x1E, 0x26),
        border: danger(),
        text: text_muted(),
    }))
}

fn text_input_style() -> theme::TextInput {
    theme::TextInput::Custom(Box::new(FieldInputStyle))
}

struct PillButton {
    fill: Color,
    border: Color,
    text: Color,
}

impl iced::widget::button::StyleSheet for PillButton {
    type Style = Theme;

    fn active(&self, _style: &Self::Style) -> iced::widget::button::Appearance {
        iced::widget::button::Appearance {
            background: Some(self.fill.into()),
            text_color: self.text,
            border: iced::Border {
                color: self.border,
                width: 1.0,
                radius: CONTROL_RADIUS.into(),
            },
            shadow: iced::Shadow {
                color: Color { a: 0.08, ..Color::BLACK },
                offset: iced::Vector::new(0.0, 3.0),
                blur_radius: 10.0,
            },
            shadow_offset: iced::Vector::new(0.0, 0.0),
        }
    }
}

struct FieldInputStyle;

impl iced::widget::text_input::StyleSheet for FieldInputStyle {
    type Style = Theme;

    fn active(&self, _style: &Self::Style) -> iced::widget::text_input::Appearance {
        iced::widget::text_input::Appearance {
            background: panel_soft().into(),
            border: iced::Border {
                color: border_soft(),
                width: 1.0,
                radius: CONTROL_RADIUS.into(),
            },
            icon_color: text_muted(),
        }
    }

    fn focused(&self, _style: &Self::Style) -> iced::widget::text_input::Appearance {
        iced::widget::text_input::Appearance {
            background: panel_soft().into(),
            border: iced::Border {
                color: accent(),
                width: 1.0,
                radius: CONTROL_RADIUS.into(),
            },
            icon_color: accent(),
        }
    }

    fn placeholder_color(&self, _style: &Self::Style) -> Color {
        text_subtle()
    }

    fn value_color(&self, _style: &Self::Style) -> Color {
        text_primary()
    }

    fn disabled_color(&self, _style: &Self::Style) -> Color {
        text_subtle()
    }

    fn selection_color(&self, _style: &Self::Style) -> Color {
        accent_soft()
    }

    fn disabled(&self, _style: &Self::Style) -> iced::widget::text_input::Appearance {
        self.active(_style)
    }
}

fn bg() -> Color {
    color(0x0B, 0x0E, 0x14)
}

fn panel_main() -> Color {
    color(0x18, 0x1C, 0x27)
}

fn panel_alt() -> Color {
    color(0x1E, 0x22, 0x2E)
}

fn panel_soft() -> Color {
    color(0x14, 0x18, 0x22)
}

fn accent() -> Color {
    color(0x9B, 0x67, 0xF6)
}

fn accent_soft() -> Color {
    color(0x2C, 0x20, 0x49)
}

fn text_primary() -> Color {
    color(0xF0, 0xEC, 0xFB)
}

fn text_muted() -> Color {
    color(0xB7, 0xAF, 0xCD)
}

fn text_subtle() -> Color {
    color(0x8D, 0x86, 0xA5)
}

fn border_soft() -> Color {
    color(0x2B, 0x2F, 0x3B)
}

fn success() -> Color {
    color(0x4C, 0xBE, 0x76)
}

fn danger() -> Color {
    color(0xE2, 0x63, 0x72)
}

fn color(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}

fn clear_status_command() -> Command<Message> {
    Command::perform(
        async {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        },
        |_| Message::ClearTelegramStatus,
    )
}

fn reset_save_button_command() -> Command<Message> {
    Command::perform(
        async {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        },
        |_| Message::ResetSaveButton,
    )
}
