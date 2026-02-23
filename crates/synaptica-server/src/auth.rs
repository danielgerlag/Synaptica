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
            Some(t) if config.tokens.contains(&t.to_string()) => Ok(request),
            _ => Err(Status::unauthenticated("invalid or missing bearer token")),
        }
    }
}
