use axum::http::{header, StatusCode};

#[derive(rust_embed::RustEmbed)]
#[folder = "public"]
struct Public;

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

macro_rules! static_response {
    ($method: ident, $filename: literal) => {{
        async fn static_response() -> impl axum::response::IntoResponse {
            public::File($filename)
        }
        $method(static_response)
    }};
}

pub(crate) use static_response;
