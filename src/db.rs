use std::time::Duration;

use color_eyre::eyre::{Context, Result};
use sqlx::{PgPool, postgres::PgPoolOptions};

pub async fn connect_from_env() -> Result<Option<PgPool>> {
    let database_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(std::env::VarError::NotPresent) => {
            tracing::info!("DATABASE_URL not set; database disabled");
            return Ok(None);
        }
        Err(err) => return Err(err).wrap_err("failed to read DATABASE_URL"),
    };

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&database_url)
        .await
        .wrap_err("failed to connect to database")?;

    sqlx::migrate!()
        .run(&pool)
        .await
        .wrap_err("failed to run database migrations")?;

    tracing::info!("database connection established");

    Ok(Some(pool))
}
