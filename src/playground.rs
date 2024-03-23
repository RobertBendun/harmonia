use std::time::Duration;

use tokio::time::sleep;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "linky_groups=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let mut groups = linky_groups::listen();

    for i in 0..10 {
        let s = format!("hello #{i}");
        groups.start(&s).await.unwrap();
        sleep(Duration::new(1, 0)).await;
    }

    groups.shutdown().await;
}
