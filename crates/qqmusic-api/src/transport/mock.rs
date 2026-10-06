//! In-memory transport for tests.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;

use super::{Request, Response, Transport, TransportError};

type Handler = dyn Fn(&Request) -> Result<Response, TransportError> + Send + Sync;

enum Route {
    /// Matches when the predicate returns true; answered by the handler.
    Handler {
        matcher: Box<dyn Fn(&Request) -> bool + Send + Sync>,
        handler: Box<Handler>,
    },
}

/// Mock transport recording every request.
///
/// Responses are produced, in order of priority, by:
/// 1. queued responses ([`MockTransport::push_response`]);
/// 2. routes registered with [`MockTransport::route`] (first match wins);
/// 3. the fallback handler (defaults to `404`).
#[derive(Clone)]
pub struct MockTransport {
    inner: Arc<Inner>,
}

struct Inner {
    queue: Mutex<VecDeque<Result<Response, TransportError>>>,
    routes: Mutex<Vec<Route>>,
    fallback: Mutex<Option<Box<Handler>>>,
    requests: Mutex<Vec<Request>>,
}

impl fmt::Debug for MockTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MockTransport")
            .field("requests", &self.request_count())
            .finish_non_exhaustive()
    }
}

impl Default for MockTransport {
    fn default() -> Self {
        Self::new()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl MockTransport {
    /// Empty mock (every request returns `404`).
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                queue: Mutex::new(VecDeque::new()),
                routes: Mutex::new(Vec::new()),
                fallback: Mutex::new(None),
                requests: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Mock answering every request with `handler`.
    pub fn with_handler<F>(handler: F) -> Self
    where
        F: Fn(&Request) -> Result<Response, TransportError> + Send + Sync + 'static,
    {
        let mock = Self::new();
        *lock(&mock.inner.fallback) = Some(Box::new(handler));
        mock
    }

    /// Queue a response for the next request.
    pub fn push_response(&self, response: Response) -> &Self {
        lock(&self.inner.queue).push_back(Ok(response));
        self
    }

    /// Queue a JSON response for the next request.
    pub fn push_json(&self, value: Value) -> &Self {
        self.push_response(Response::json(&value))
    }

    /// Queue an error for the next request.
    pub fn push_error(&self, error: TransportError) -> &Self {
        lock(&self.inner.queue).push_back(Err(error));
        self
    }

    /// Register a route.
    pub fn route<M, F>(&self, matcher: M, handler: F) -> &Self
    where
        M: Fn(&Request) -> bool + Send + Sync + 'static,
        F: Fn(&Request) -> Result<Response, TransportError> + Send + Sync + 'static,
    {
        lock(&self.inner.routes).push(Route::Handler {
            matcher: Box::new(matcher),
            handler: Box::new(handler),
        });
        self
    }

    /// Register a route for URLs containing `fragment`.
    pub fn route_url<F>(&self, fragment: &'static str, handler: F) -> &Self
    where
        F: Fn(&Request) -> Result<Response, TransportError> + Send + Sync + 'static,
    {
        self.route(move |req| req.url.contains(fragment), handler)
    }

    /// All recorded requests.
    pub fn requests(&self) -> Vec<Request> {
        lock(&self.inner.requests).clone()
    }

    /// Recorded requests whose URL contains `fragment`.
    pub fn requests_to(&self, fragment: &str) -> Vec<Request> {
        lock(&self.inner.requests)
            .iter()
            .filter(|r| r.url.contains(fragment))
            .cloned()
            .collect()
    }

    /// Number of recorded requests.
    pub fn request_count(&self) -> usize {
        lock(&self.inner.requests).len()
    }

    /// Last recorded request.
    pub fn last_request(&self) -> Option<Request> {
        lock(&self.inner.requests).last().cloned()
    }

    /// Forget recorded requests.
    pub fn clear_requests(&self) {
        lock(&self.inner.requests).clear();
    }

    fn respond(&self, request: &Request) -> Result<Response, TransportError> {
        if let Some(queued) = lock(&self.inner.queue).pop_front() {
            return queued;
        }
        {
            let routes = lock(&self.inner.routes);
            for Route::Handler { matcher, handler } in routes.iter() {
                if matcher(request) {
                    return handler(request);
                }
            }
        }
        match lock(&self.inner.fallback).as_ref() {
            Some(handler) => handler(request),
            None => Ok(Response::new(404, format!("no mock for {}", request.url))),
        }
    }
}

#[async_trait]
impl Transport for MockTransport {
    async fn send(&self, request: Request) -> Result<Response, TransportError> {
        lock(&self.inner.requests).push(request.clone());
        let mut response = self.respond(&request)?;
        if response.url.is_empty() {
            response.url = request.full_url();
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::Method;
    use serde_json::json;

    #[tokio::test]
    async fn priority_queue_routes_fallback() {
        let mock = MockTransport::with_handler(|_| Ok(Response::new(500, "fallback")));
        mock.route_url("/a", |_| Ok(Response::new(200, "route")));
        mock.push_json(json!({"q": 1}));

        let req = |url: &str| Request::new(Method::Get, url);
        assert_eq!(mock.send(req("http://x/a")).await.unwrap().text(), r#"{"q":1}"#);
        assert_eq!(mock.send(req("http://x/a")).await.unwrap().text(), "route");
        assert_eq!(mock.send(req("http://x/b")).await.unwrap().status, 500);
        assert_eq!(mock.request_count(), 3);
        assert_eq!(mock.requests_to("/a").len(), 2);
        assert_eq!(mock.last_request().unwrap().url, "http://x/b");
        mock.clear_requests();
        assert_eq!(mock.request_count(), 0);
    }

    #[tokio::test]
    async fn default_404_and_errors() {
        let mock = MockTransport::new();
        let resp = mock.send(Request::new(Method::Get, "http://x")).await.unwrap();
        assert_eq!(resp.status, 404);
        assert_eq!(resp.url, "http://x");
        mock.push_error(TransportError::timeout("t"));
        let err = mock.send(Request::new(Method::Get, "http://x")).await.unwrap_err();
        assert!(err.timeout);
    }
}
