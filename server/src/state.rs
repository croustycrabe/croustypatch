use crate::config::Config;
use crate::crypto::MasterKey;
use crate::error::{ApiError, ApiResult};
use crate::gateway::Hub;
use diesel::r2d2::{ConnectionManager, Pool};
use diesel::PgConnection;
use std::collections::HashMap;
use std::sync::Mutex;
use zeroize::Zeroizing;

pub type PgPool = Pool<ConnectionManager<PgConnection>>;

pub struct AppState {
    pub pool: PgPool,
    pub cfg: Config,
    pub master: MasterKey,
    pub server_private_pem: Zeroizing<String>,
    pub server_public_pem: String,
    pub session_secret: Zeroizing<Vec<u8>>,
    pub hub: Hub,
    /// Whether the server itself terminates TLS (drives the `Secure` cookie flag).
    pub tls_direct: bool,
    /// Agent JWT ids already seen (replay protection): jti -> exp.
    pub seen_jti: Mutex<HashMap<String, i64>>,
}

impl AppState {
    /// Run blocking Diesel work off the async executor.
    pub async fn db<T, F>(&self, f: F) -> ApiResult<T>
    where
        F: FnOnce(&mut PgConnection) -> ApiResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let pool = self.pool.clone();
        actix_web::web::block(move || {
            let mut conn = pool.get().map_err(ApiError::internal)?;
            f(&mut conn)
        })
        .await
        .map_err(ApiError::internal)?
    }
}
