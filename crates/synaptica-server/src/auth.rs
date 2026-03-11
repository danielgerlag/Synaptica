use tonic::service::Interceptor;
use tonic::{Request, Status};

use crate::config::AuthConfig;

#[derive(Clone)]
pub struct AuthInterceptor {
    config: Option<AuthConfig>,
}

impl AuthInterceptor {
    pub fn new(config: Option<AuthConfig>) -> Self {
        Self { config }
    }
}

/// Constant-time byte comparison to prevent timing side-channels on token validation.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

impl Interceptor for AuthInterceptor {
    fn call(&mut self, request: Request<()>) -> Result<Request<()>, Status> {
        let config = match &self.config {
            Some(c) if c.enabled => c,
            _ => return Ok(request),
        };

        let token = request
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));

        match token {
            Some(t) => {
                let matched = config
                    .tokens
                    .iter()
                    .any(|valid| constant_time_eq(t.as_bytes(), valid.as_bytes()));
                if matched {
                    Ok(request)
                } else {
                    Err(Status::unauthenticated("invalid or missing bearer token"))
                }
            }
            _ => Err(Status::unauthenticated("invalid or missing bearer token")),
        }
    }
}
