use axum::{
    extract::Form,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use std::path::Path;
use tokio::fs;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const LINK_FILE: &str = "lnk";

#[derive(Debug, Deserialize)]
struct UpdateForm {
    content: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "axum_link_manager=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting server on http://localhost:11111");

    if Path::new(LINK_FILE).exists() {
        info!("Appending to existing log file: {}", LINK_FILE);
    } else {
        info!("Creating new log file: {}", LINK_FILE);
    }

    let app = Router::new()
        .route("/up", get(up_form))
        .route("/lnk", get(show_links))
        .route("/updatelnk", post(update_link));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:11111").await?;
    axum::serve(listener, app).await?;

    Ok(())
}

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
