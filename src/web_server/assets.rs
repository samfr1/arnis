//! Embedded static assets for the admin panel.

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

const INDEX_HTML: &str = include_str!("assets/index.html");

pub async fn serve_index() -> Response {
    let mut resp = (StatusCode::OK, INDEX_HTML).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    resp
}
