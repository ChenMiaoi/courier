use std::sync::Once;

use tracing_subscriber::EnvFilter;

use crate::infra::error::{CourierError, ErrorCode, Result};

static INIT: Once = Once::new();

pub fn init(default_filter: &str) -> Result<()> {
    let mut result = Ok(());

    INIT.call_once(|| {
        let env_filter = EnvFilter::try_from_default_env()
            .or_else(|_| EnvFilter::try_new(default_filter))
            .unwrap_or_else(|_| EnvFilter::new("info"));

        if let Err(err) = tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_target(false)
            .compact()
            .try_init()
        {
            result = Err(CourierError::new(
                ErrorCode::LoggingInit,
                format!("failed to initialize tracing subscriber: {err}"),
            ));
        }
    });

    result
}
