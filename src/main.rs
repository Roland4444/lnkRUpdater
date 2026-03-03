use axum::{
    extract::{Form, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json,  // <--- добавить эту строку
    Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::fs;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// --- Константы для менеджера ссылок ---
const LINK_FILE: &str = "lnk";

// --- Структуры для менеджера ссылок ---
#[derive(Debug, Deserialize)]
struct UpdateForm {
    content: String,
}

// --- Структуры для бота Битрикс24 ---

#[derive(Debug, Deserialize)]
struct InstallEvent {
    event: String,
    data: Option<serde_json::Value>,
    auth: AuthData,
}

#[derive(Debug, Deserialize)]
struct WebhookEvent {
    event: String,
    data: WebhookData,
    auth: AuthData,
}

#[derive(Debug, Deserialize, Clone)]
struct AuthData {
    #[serde(rename = "access_token")]
    access_token: String,
    #[serde(rename = "refresh_token")]
    refresh_token: String,
    #[serde(rename = "member_id")]
    member_id: String,
    #[serde(rename = "application_token")]
    application_token: String,
    domain: String,
    #[serde(rename = "expires_in")]
    expires_in: u32,
}

#[derive(Debug, Deserialize)]
struct WebhookData {
    #[serde(rename = "PARAMS")]
    params: MessageParams,
}

#[derive(Debug, Deserialize)]
struct MessageParams {
    #[serde(rename = "DIALOG_ID")]
    dialog_id: String,
    #[serde(rename = "MESSAGE")]
    message: String,
    #[serde(rename = "USER_ID")]
    user_id: u64,
}

#[derive(Debug, Clone)]
struct TokenInfo {
    access_token: String,
    refresh_token: String,
    member_id: String,
    application_token: String,
}

// --- Конфигурация приложения (из .env) ---
#[derive(Debug, Clone)]
struct Config {
    client_id: String,
    client_secret: String,
    bot_id: u64,
    your_user_id: u64,
    target_chat_id: String,
    oauth_url: String, // можно захардкодить
}

impl Config {
    fn from_env() -> anyhow::Result<Self> {
        Ok(Config {
            client_id: std::env::var("CLIENT_ID")?,
            client_secret: std::env::var("CLIENT_SECRET")?,
            bot_id: std::env::var("BOT_ID")?.parse()?,
            your_user_id: std::env::var("YOUR_USER_ID")?.parse()?,
            target_chat_id: std::env::var("TARGET_CHAT_ID")?,
            oauth_url: "https://oauth.bitrix.info/oauth/token/".to_string(),
        })
    }
}

// --- Общее состояние приложения ---
#[derive(Clone)]
struct AppState {
    // для бота
    http_client: reqwest::Client,
    config: Config,
    tokens: Arc<Mutex<HashMap<String, TokenInfo>>>,
    // для менеджера ссылок (если понадобится, можно добавить)
}

// --- Основная функция ---
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Загружаем переменные окружения из .env
    dotenv::dotenv().ok();
    
    // Простая инициализация логирования (уровень info по умолчанию)
    tracing_subscriber::fmt::init();

    info!("Starting combined server on http://0.0.0.0:11111");

    // Загружаем конфигурацию бота
    let config = Config::from_env().expect("Failed to load config from .env");
    info!("Config loaded: client_id={}", config.client_id);

    // Создаём состояние
    let state = AppState {
        http_client: reqwest::Client::new(),
        config,
        tokens: Arc::new(Mutex::new(HashMap::new())),
    };

    // Проверяем файл ссылок
    if Path::new(LINK_FILE).exists() {
        info!("Appending to existing log file: {}", LINK_FILE);
    } else {
        info!("Creating new log file: {}", LINK_FILE);
    }

    // Собираем все маршруты
    let app = Router::new()
        // Маршруты менеджера ссылок
        .route("/up", get(up_form))
        .route("/lnk", get(show_links))
        .route("/updatelnk", post(update_link))
        // Маршруты бота
        .route("/install", post(install_handler))
        .route("/webhook", post(webhook_handler))
        .with_state(state); // передаём состояние во все обработчики

    // Запускаем сервер
    let listener = tokio::net::TcpListener::bind("0.0.0.0:11111").await?;
    axum::serve(listener, app).await?;

    Ok(())
}

// --- Обработчики менеджера ссылок (без изменений) ---

async fn up_form() -> impl IntoResponse {
    let html = r#"<!DOCTYPE html>
<html>
<head><title>Update Link</title></head>
<body>
    <h2>Update Link</h2>
    <form method="POST" action="/updatelnk">
        <label for="content">New content:</label>
        <input type="text" name="content" value="" size="50">
        <br/>
        <input type="submit" value="Update">
    </form>
</body>
</html>"#;
    Html(html)
}

async fn show_links() -> Result<Html<String>, (StatusCode, String)> {
    let content = match fs::read_to_string(LINK_FILE).await {
        Ok(data) => data,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Error reading file: {}", e),
            ))
        }
    };

    let links: Vec<String> = content
        .lines()
        .map(|line| format!(r#"<a href="{}">{}</a><br>"#, line, line))
        .collect();

    let body = format!("<h3>HI there!</h3><br>{}", links.join(""));
    Ok(Html(body))
}

async fn update_link(Form(form): Form<UpdateForm>) -> impl IntoResponse {
    if let Err(e) = fs::write(LINK_FILE, &form.content).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to write file: {}", e),
        );
    }
    (StatusCode::OK, "Link updated successfully.".to_string())
}

// --- Обработчики бота Битрикс24 (адаптированы под общее состояние) ---

/// Установка приложения — сохраняем токены
async fn install_handler(
    State(state): State<AppState>,
    Json(payload): Json<InstallEvent>,
) -> impl IntoResponse {
    info!("Получено событие установки: {:?}", payload.event);

    let member_id = payload.auth.member_id.clone();
    let token_info = TokenInfo {
        access_token: payload.auth.access_token,
        refresh_token: payload.auth.refresh_token,
        member_id: payload.auth.member_id,
        application_token: payload.auth.application_token,
    };

    // Сохраняем токены для этого портала
    {
        let mut tokens = state.tokens.lock().unwrap();
        tokens.insert(member_id.clone(), token_info);
    }

    info!("Токены сохранены для member_id: {}", member_id);

    (StatusCode::OK, Json(serde_json::json!({"result": "ok"})))
}

/// Обработка нового сообщения
async fn webhook_handler(
    State(state): State<AppState>,
    Json(payload): Json<WebhookEvent>,
) -> impl IntoResponse {
    info!("Получен вебхук: event={}", payload.event);

    // Проверяем, что это событие нового сообщения
    if payload.event != "ONIMBOTMESSAGEADD" {
        return (StatusCode::OK, Json(serde_json::json!({"status": "ignored"})));
    }

    let member_id = &payload.auth.member_id;

    // Достаём сохранённые токены для этого портала
    let token_info = {
        let tokens = state.tokens.lock().unwrap();
        tokens.get(member_id).cloned()
    };

    let token_info = match token_info {
        Some(t) => t,
        None => {
            eprintln!("Нет токенов для member_id: {}", member_id);
            return (StatusCode::FORBIDDEN, Json(serde_json::json!({"status": "no_tokens"})));
        }
    };

    // Проверяем application_token (совпадает ли с сохранённым)
    if payload.auth.application_token != token_info.application_token {
        eprintln!("Неверный application_token");
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"status": "invalid_token"})));
    }

    // Определяем ID чата (убираем префикс 'chat' если есть)
    let dialog_id = &payload.data.params.dialog_id;
    let clean_dialog_id = dialog_id.strip_prefix("chat").unwrap_or(dialog_id);

    // Сравниваем с целевым ID из конфига
    if clean_dialog_id != state.config.target_chat_id {
        info!("Сообщение не из целевого чата: {}", clean_dialog_id);
        return (StatusCode::OK, Json(serde_json::json!({"status": "ignored"})));
    }

    // Формируем текст пересылки
    let forward_text = format!(
        "Сообщение из коллабы от пользователя {}:\n{}",
        payload.data.params.user_id, payload.data.params.message
    );

    // Отправляем сообщение себе через API
    let send_result = send_message(
        &state.http_client,
        &token_info.access_token,
        &payload.auth.domain,
        state.config.bot_id,
        state.config.your_user_id,
        &forward_text,
    )
    .await;

    match send_result {
        Ok(_) => info!("Сообщение переслано"),
        Err(e) => eprintln!("Ошибка отправки: {}", e),
    }

    (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
}

/// Отправка сообщения через REST API Битрикс24
async fn send_message(
    client: &reqwest::Client,
    access_token: &str,
    domain: &str,
    bot_id: u64,
    dialog_id: u64,
    text: &str,
) -> anyhow::Result<()> {
    let url = format!("https://{}/rest/imbot.message.add", domain);

    let params = serde_json::json!({
        "BOT_ID": bot_id,
        "DIALOG_ID": dialog_id,
        "MESSAGE": text,
    });

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .json(&params)
        .send()
        .await?;

    if response.status().is_success() {
        Ok(())
    } else {
        let status = response.status();
        let text = response.text().await?;
        anyhow::bail!("Ошибка API: {} - {}", status, text)
    }
}