#[derive(Clone)]
pub struct AppConfig {
    pub port: u16,
    pub database_url: String,
    pub auth_secret: String,
    // better-auth uses the below to generate verification urls for signups.
    pub base_url: String,
    pub frontend_allow_origins: Vec<String>,
}

impl AppConfig {
    pub fn from_env() -> Self {
        Self {
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),

            database_url: std::env::var("DATABASE_URL").expect("DATABSE_URL is not set."),

            auth_secret: std::env::var("AUTH_SECRET").expect("AUTH_SECRET is not set."),

            base_url: std::env::var("BASE_URL").expect("BASE_URL is not set."),

            frontend_allow_origins: std::env::var("FRONTEND_ALLOW_ORIGINS")
                .unwrap_or_else(|_| "http://localhost:5173".to_string())
                .trim()
                .split(',')
                .map(|url| url.trim().to_string())
                .collect(),
        }
    }
}
