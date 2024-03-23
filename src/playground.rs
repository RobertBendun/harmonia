use linky_start::StartListSession;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "linky_start=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let mut session = StartListSession::listen();
    if let Some(task) = session.listener.take() {
        let _ = task.await;
    }
}
