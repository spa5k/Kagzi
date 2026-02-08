use axum::Router;
use axum::body::Body;
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../app/dist"]
pub struct Assets;

/// Create a router that serves embedded static files with SPA fallback
pub fn static_router() -> Router {
    Router::new().fallback(get(static_handler))
}

async fn static_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');

    // Try to serve the requested file
    if let Some(content) = Assets::get(path) {
        return serve_asset(path, content.data.as_ref());
    }

    // SPA fallback: serve index.html for client-side routing
    if let Some(content) = Assets::get("index.html") {
        return serve_asset("index.html", content.data.as_ref());
    }

    // If even index.html is missing, return 404
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from("404 Not Found"))
        .unwrap()
}

fn serve_asset(path: &str, content: &[u8]) -> Response {
    let mime_type = mime_guess::from_path(path)
        .first_or_octet_stream()
        .as_ref()
        .to_string();

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime_type)
        .body(Body::from(content.to_vec()))
        .unwrap()
}
