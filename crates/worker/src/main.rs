use anyhow::Result;
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderValue, Method, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::{net::SocketAddr, sync::Arc};
use tower_http::cors::CorsLayer;
use zim_engine::{templates, ArchiveTemplate, Engine, Job, NewJob, Settings};

#[derive(Clone)]
struct App {
    engine: Engine,
    token: Option<Arc<String>>,
}
type ApiResult<T> = std::result::Result<T, ApiError>;
struct ApiError(anyhow::Error);
impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self(error)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{:#}", self.0)})),
        )
            .into_response()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let address: SocketAddr = std::env::var("ZIMSTUDIO_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8788".into())
        .parse()?;
    let token = std::env::var("ZIMSTUDIO_TOKEN")
        .ok()
        .filter(|s| s.len() >= 24)
        .map(Arc::new);
    anyhow::ensure!(
        address.ip().is_loopback() || token.is_some(),
        "A ZIMSTUDIO_TOKEN of at least 24 characters is required when listening beyond loopback"
    );
    let root = std::env::var("ZIMSTUDIO_DATA_DIR").unwrap_or_else(|_| ".zim-studio".into());
    let app = App {
        engine: Engine::open(&root).await?,
        token,
    };
    let mut origins = vec![
        "http://localhost:1420",
        "http://127.0.0.1:1420",
        "tauri://localhost",
        "http://tauri.localhost",
        "https://tauri.localhost",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    if let Ok(extra) = std::env::var("ZIMSTUDIO_ORIGINS") {
        origins.extend(extra.split(',').map(|s| s.trim().to_string()));
    }
    let origins: Vec<HeaderValue> = origins
        .into_iter()
        .map(|s| s.parse())
        .collect::<std::result::Result<_, _>>()?;
    let cors = CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);
    let router = Router::new()
        .route("/api/health", get(health))
        .route("/api/settings", get(settings).post(update_settings))
        .route("/api/templates", get(archive_templates))
        .route(
            "/api/templates/{template}/preview",
            get(preview_archive_template),
        )
        .route("/api/jobs", get(list).post(create))
        .route("/api/jobs/{id}", get(job))
        .route("/api/jobs/{id}/pause", post(pause))
        .route("/api/jobs/{id}/resume", post(resume))
        .route("/api/jobs/{id}/build", post(build))
        .route("/api/jobs/{id}/edit", post(edit))
        .route("/api/jobs/{id}/rebuild", post(rebuild))
        .route("/api/jobs/{id}/refresh", post(refresh))
        .route("/api/jobs/{id}/artifact", get(artifact))
        .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn_with_state(app.clone(), authorize))
        .layer(cors)
        .with_state(app);
    let listener = tokio::net::TcpListener::bind(address).await?;
    println!(
        "Zim Studio worker listening on http://{}; data: {}",
        listener.local_addr()?,
        root
    );
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

async fn authorize(State(app): State<App>, request: Request<Body>, next: Next) -> Response {
    // Requiring JSON also on bodyless mutations prevents simple-request CSRF against localhost.
    if request.method() == Method::POST
        && request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| !v.starts_with("application/json"))
            .unwrap_or(true)
    {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type must be application/json",
        )
            .into_response();
    }
    if let Some(token) = app.token {
        let expected = format!("Bearer {token}");
        let actual = request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !constant_time_eq(expected.as_bytes(), actual.as_bytes()) {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error":"Worker token is missing or incorrect"})),
            )
                .into_response();
        }
    }
    next.run(request).await
}
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for (i, value) in a.iter().enumerate() {
        diff |= (*value ^ b.get(i).copied().unwrap_or(0)) as usize;
    }
    diff == 0
}
async fn health(State(app): State<App>) -> Json<zim_engine::Capabilities> {
    Json(app.engine.capabilities().await)
}
async fn settings(State(app): State<App>) -> Json<Settings> {
    Json(app.engine.settings().await)
}
async fn archive_templates() -> Json<Vec<templates::TemplateOption>> {
    Json(templates::catalog())
}
async fn preview_archive_template(
    Path(template): Path<ArchiveTemplate>,
) -> ApiResult<Json<String>> {
    Ok(Json(templates::preview(template)?))
}
async fn update_settings(
    State(app): State<App>,
    Json(settings): Json<Settings>,
) -> ApiResult<Json<Settings>> {
    Ok(Json(app.engine.update_settings(settings).await?))
}
async fn list(State(app): State<App>) -> Json<Vec<Job>> {
    Json(app.engine.list().await.into_iter().map(Job::view).collect())
}
async fn create(State(app): State<App>, Json(config): Json<NewJob>) -> ApiResult<Json<Job>> {
    Ok(Json(app.engine.create(config).await?.view()))
}
async fn job(State(app): State<App>, Path(id): Path<String>) -> ApiResult<Json<Job>> {
    Ok(Json(app.engine.get(&id).await?.view()))
}
async fn pause(State(app): State<App>, Path(id): Path<String>) -> ApiResult<Json<Job>> {
    Ok(Json(app.engine.pause(&id).await?.view()))
}
async fn resume(State(app): State<App>, Path(id): Path<String>) -> ApiResult<Json<Job>> {
    Ok(Json(app.engine.resume(&id).await?.view()))
}
async fn refresh(State(app): State<App>, Path(id): Path<String>) -> ApiResult<Json<Job>> {
    Ok(Json(app.engine.refresh_collection(&id).await?.view()))
}
async fn edit(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(config): Json<NewJob>,
) -> ApiResult<Json<Job>> {
    Ok(Json(app.engine.edit_retry(&id, config).await?.view()))
}
#[derive(Deserialize)]
struct Selection {
    selected: Vec<String>,
}
async fn build(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(selection): Json<Selection>,
) -> ApiResult<Json<Job>> {
    Ok(Json(
        app.engine.build(&id, selection.selected).await?.view(),
    ))
}
#[derive(Deserialize)]
struct Rebuild {
    template: ArchiveTemplate,
}
async fn rebuild(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(options): Json<Rebuild>,
) -> ApiResult<Json<Job>> {
    Ok(Json(
        app.engine.rebuild(&id, options.template).await?.view(),
    ))
}
async fn artifact(State(app): State<App>, Path(id): Path<String>) -> ApiResult<Response> {
    let path = app.engine.artifact(&id).await?;
    let file = tokio::fs::File::open(path)
        .await
        .map_err(anyhow::Error::from)?;
    let size = file.metadata().await.map_err(anyhow::Error::from)?.len();
    let body = Body::from_stream(tokio_util::io::ReaderStream::new(file));
    Ok((
        [
            (header::CONTENT_TYPE, "application/x-zim".to_string()),
            (header::CONTENT_LENGTH, size.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{id}.zim\""),
            ),
        ],
        body,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    #[test]
    fn tokens() {
        assert!(super::constant_time_eq(b"secret", b"secret"));
        assert!(!super::constant_time_eq(b"secret", b"secre"));
        assert!(!super::constant_time_eq(b"secret", b"secrex"));
    }
}
