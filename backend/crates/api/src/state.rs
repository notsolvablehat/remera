use crate::config::AppConfig;

#[derive(Clone)]
pub struct AppState {
    #[allow(dead_code)]
    pub config: AppConfig,
}

impl AppState {
    pub async fn new(config: AppConfig) -> Self {
        Self { config }
    }
}
