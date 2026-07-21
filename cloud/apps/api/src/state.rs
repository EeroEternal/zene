use zene_cloud_db::Db;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub worker_token: String,
}
