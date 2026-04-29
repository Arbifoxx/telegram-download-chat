use grammers_client::client::LoginToken;
use grammers_client::{Client, SignInError};
use grammers_mtsender::SenderPool;
use grammers_session::storages::SqliteSession;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::config;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AuthAction {
    #[default]
    None,
    ApiSaved,
    CodeRequested,
    SignedIn,
    LoggedOut,
}

#[derive(Debug, Clone, Default)]
pub struct AuthResult {
    pub authorized: bool,
    pub message: String,
    pub keep_popup_open: bool,
    pub action: AuthAction,
    pub identity_label: Option<String>,
}

#[derive(Clone, Default)]
pub struct AuthTokens {
    login: Arc<Mutex<Option<Arc<LoginToken>>>>,
    password_pending: Arc<Mutex<bool>>,
}

impl AuthTokens {
    pub fn new() -> Self {
        Self::default()
    }

    async fn clear(&self) {
        *self.login.lock().await = None;
        *self.password_pending.lock().await = false;
    }
}

pub async fn save_api_credentials(api_id: String, api_hash: String) -> AuthResult {
    if api_id.trim().is_empty() || api_hash.trim().is_empty() {
        return AuthResult {
            authorized: false,
            message: "API ID and API Hash are required.".to_string(),
            keep_popup_open: true,
            action: AuthAction::None,
            identity_label: None,
        };
    }

    match api_id.trim().parse::<i32>() {
        Ok(_) => AuthResult {
            authorized: false,
            message: "API credentials saved.".to_string(),
            keep_popup_open: true,
            action: AuthAction::ApiSaved,
            identity_label: None,
        },
        Err(_) => AuthResult {
            authorized: false,
            message: "API ID must be a number.".to_string(),
            keep_popup_open: true,
            action: AuthAction::None,
            identity_label: None,
        },
    }
}

pub async fn check_authorized(api_id: String, api_hash: String) -> AuthResult {
    let creds = match parse_credentials(&api_id, &api_hash) {
        Ok(creds) => creds,
        Err(message) => {
            return AuthResult {
                authorized: false,
                message,
                keep_popup_open: false,
                action: AuthAction::None,
                identity_label: None,
            }
        }
    };

    match connect_client(creds.api_id, &creds.api_hash).await {
        Ok(client) => match client.is_authorized().await {
            Ok(true) => {
                let identity_label = client
                    .get_me()
                    .await
                    .ok()
                    .map(|user| format!("Logged in as {}", user.full_name()));
                AuthResult {
                    authorized: true,
                    message: "Telegram session ready.".to_string(),
                    keep_popup_open: false,
                    action: AuthAction::None,
                    identity_label,
                }
            }
            Ok(false) => AuthResult {
                authorized: false,
                message: "Telegram session not authorized yet.".to_string(),
                keep_popup_open: false,
                action: AuthAction::None,
                identity_label: None,
            },
            Err(error) => AuthResult {
                authorized: false,
                message: format!("Failed to verify session: {error}"),
                keep_popup_open: false,
                action: AuthAction::None,
                identity_label: None,
            },
        },
        Err(message) => AuthResult {
            authorized: false,
            message,
            keep_popup_open: false,
            action: AuthAction::None,
            identity_label: None,
        },
    }
}

pub async fn request_code_or_sign_in(
    api_id: String,
    api_hash: String,
    phone_number: String,
    code: String,
    password: String,
    tokens: AuthTokens,
) -> AuthResult {
    let creds = match parse_credentials(&api_id, &api_hash) {
        Ok(creds) => creds,
        Err(message) => {
            return AuthResult {
                authorized: false,
                message,
                keep_popup_open: true,
                action: AuthAction::None,
                identity_label: None,
            }
        }
    };

    if phone_number.trim().is_empty() {
        return AuthResult {
            authorized: false,
            message: "Phone Number is required.".to_string(),
            keep_popup_open: true,
            action: AuthAction::None,
            identity_label: None,
        };
    }

    let client = match connect_client(creds.api_id, &creds.api_hash).await {
        Ok(client) => client,
        Err(message) => {
            return AuthResult {
                authorized: false,
                message,
                keep_popup_open: true,
                action: AuthAction::None,
                identity_label: None,
            }
        }
    };

    if code.trim().is_empty() {
        match client
            .request_login_code(phone_number.trim(), &creds.api_hash)
            .await
        {
            Ok(token) => {
                *tokens.login.lock().await = Some(Arc::new(token));
                *tokens.password_pending.lock().await = false;
                AuthResult {
                    authorized: false,
                    message: "Code requested. Enter it below to continue.".to_string(),
                    keep_popup_open: true,
                    action: AuthAction::CodeRequested,
                    identity_label: None,
                }
            }
            Err(error) => AuthResult {
                authorized: false,
                message: format!("Failed to request code: {error}"),
                keep_popup_open: true,
                action: AuthAction::None,
                identity_label: None,
            },
        }
    } else {
        let login_state = match tokens.login.lock().await.clone() {
            Some(token) => token,
            None => {
                return AuthResult {
                    authorized: false,
                    message: "Request a code first.".to_string(),
                    keep_popup_open: true,
                    action: AuthAction::None,
                    identity_label: None,
                }
            }
        };

        match client
            .sign_in(&login_state, code.trim())
            .await
        {
            Ok(_) => {
                tokens.clear().await;
                let identity_label = client
                    .get_me()
                    .await
                    .ok()
                    .map(|user| format!("Logged in as {}", user.full_name()));
                AuthResult {
                    authorized: true,
                    message: "Signed in successfully.".to_string(),
                    keep_popup_open: false,
                    action: AuthAction::SignedIn,
                    identity_label,
                }
            }
            Err(SignInError::PasswordRequired(password_token)) => {
                if password.trim().is_empty() {
                    *tokens.password_pending.lock().await = true;
                    AuthResult {
                        authorized: false,
                        message: password_token
                            .hint()
                            .map(|hint| format!("Password required. Hint: {hint}"))
                            .unwrap_or_else(|| "Password required.".to_string()),
                        keep_popup_open: true,
                        action: AuthAction::None,
                        identity_label: None,
                    }
                } else {
                    match client.check_password(password_token, password.trim()).await {
                        Ok(_) => {
                            tokens.clear().await;
                            let identity_label = client
                                .get_me()
                                .await
                                .ok()
                                .map(|user| format!("Logged in as {}", user.full_name()));
                            AuthResult {
                                authorized: true,
                                message: "Signed in successfully.".to_string(),
                                keep_popup_open: false,
                                action: AuthAction::SignedIn,
                                identity_label,
                            }
                        }
                        Err(error) => AuthResult {
                            authorized: false,
                            message: format!("Password check failed: {error}"),
                            keep_popup_open: true,
                            action: AuthAction::None,
                            identity_label: None,
                        },
                    }
                }
            }
            Err(error) => AuthResult {
                authorized: false,
                message: format!("Failed to sign in: {error}"),
                keep_popup_open: true,
                action: AuthAction::None,
                identity_label: None,
            },
        }
    }
}

pub async fn logout(api_id: String, api_hash: String, tokens: AuthTokens) -> AuthResult {
    let creds = match parse_credentials(&api_id, &api_hash) {
        Ok(creds) => creds,
        Err(message) => {
            return AuthResult {
                authorized: false,
                message,
                keep_popup_open: true,
                action: AuthAction::None,
                identity_label: None,
            }
        }
    };

    let client = match connect_client(creds.api_id, &creds.api_hash).await {
        Ok(client) => client,
        Err(message) => {
            return AuthResult {
                authorized: false,
                message,
                keep_popup_open: true,
                action: AuthAction::None,
                identity_label: None,
            }
        }
    };

    let result = client.sign_out().await;
    let _ = tokio::fs::remove_file(session_path()).await;
    tokens.clear().await;

    match result {
        Ok(_) => AuthResult {
            authorized: false,
            message: "Logged out.".to_string(),
            keep_popup_open: true,
            action: AuthAction::LoggedOut,
            identity_label: None,
        },
        Err(error) => AuthResult {
            authorized: false,
            message: format!("Logout failed: {error}"),
            keep_popup_open: true,
            action: AuthAction::None,
            identity_label: None,
        },
    }
}

struct ParsedCredentials {
    api_id: i32,
    api_hash: String,
}

fn parse_credentials(api_id: &str, api_hash: &str) -> Result<ParsedCredentials, String> {
    if api_id.trim().is_empty() || api_hash.trim().is_empty() {
        return Err("Save API credentials first.".to_string());
    }

    let api_id = api_id
        .trim()
        .parse::<i32>()
        .map_err(|_| "API ID must be a number.".to_string())?;

    Ok(ParsedCredentials {
        api_id,
        api_hash: api_hash.trim().to_string(),
    })
}

pub(crate) async fn connect_client(api_id: i32, api_hash: &str) -> Result<Client, String> {
    let session = Arc::new(
        SqliteSession::open(session_path())
            .await
            .map_err(|error| format!("Failed to open Telegram session: {error}"))?,
    );

    let _ = api_hash;
    let SenderPool { runner, handle, .. } = SenderPool::new(Arc::clone(&session), api_id);
    let client = Client::new(handle);
    tokio::spawn(runner.run());
    Ok(client)
}

pub(crate) fn session_path() -> PathBuf {
    config::config_path()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("telegram.session")
}
