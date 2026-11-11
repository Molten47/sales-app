use anyhow::{Context, Ok, bail};

#[derive(Debug, Clone)]
pub enum EmailProvider {
    Log,
    Resend { api_key: String, from: String },
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub port: u16,
    pub jwt_secret: String,
    pub email: EmailProvider,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let email = match std::env::var("EMAIL_PROVIDER").as_deref().unwrap_or("log") {
            "log" => EmailProvider::Log,
            "resend" => EmailProvider::Resend {
                api_key: std::env::var("RESEND_API_KEY").context("RESEND_API_KEY must be set")?,
                from: std::env::var("EMAIL_FROM").context("EMAIL_FROM must be set")?,
            },
            other => bail!("EMAIL_PROVIDER must be 'log' or 'resend', got '{other}'"),
        };

        Ok(Self {
            database_url: std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?,
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),
            jwt_secret: std::env::var("JWT_SECRET").context("JWT_SECRET must be set")?,
            email,
        })
    }
}
