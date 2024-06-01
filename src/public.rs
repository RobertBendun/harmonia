//! Baking static files into Harmonia binary
//!
//! To simplify distribution, Harmonia has embeded all of it's static files into itself.
//! This however means, that to modify CSS, rebuild of the whole binary is required.
//!
//! [Public] holds the embeded static files, [File] is the index type and [static_response]
//! simplifies creating handlers for [axum] out of them.

use axum::http::{header, StatusCode};


/// All files in `public` directory should be baked into this struct
#[derive(rust_embed::RustEmbed)]
#[folder = "public"]
struct Public;

/// Name of the file baked in from `public` directory
pub struct File(pub &'static str);

impl axum::response::IntoResponse for File {
    fn into_response(self) -> axum::response::Response {
        let path = self.0;

        match Public::get(path) {
            Some(content) => {
                let mime = mime_guess::from_path(path).first_or_octet_stream();
                ([(header::CONTENT_TYPE, mime.as_ref())], content.data).into_response()
            }
            None => (StatusCode::NOT_FOUND, "404 Not Found").into_response(),
        }
    }
}


/// Convenience macro to simplify definition of routes that only return static response
///
/// This macro helps to write the static responding handlers by putting them in typed environment.
/// Without the return type [axum::response::IntoResponse] Rust doesn't see that this function
/// returns in fact something that can be converted into [axum::response::Response].
macro_rules! static_response {
    ($method: ident, $filename: literal) => {{
        async fn static_response() -> impl axum::response::IntoResponse {
            public::File($filename)
        }
        $method(static_response)
    }};
}

pub(crate) use static_response;
