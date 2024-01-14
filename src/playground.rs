use std::time::Duration;

use linky_start::StartListSession;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "linky_start=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let _session = StartListSession::start_listening();

    loop {
        std::thread::sleep(Duration::from_secs(100_000));
    }
}
