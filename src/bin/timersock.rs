use std::convert::Infallible;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use tokio::net::UnixListener;
use tokio::sync::RwLock;

const DEFAULT_SOCKET_PATH: &str = "/tmp/timer.sock";

#[derive(Debug, Clone)]
struct TimerState {
    time_left_secs: u64,
    running: bool,
    updated_at: Instant,
}

impl Default for TimerState {
    fn default() -> Self {
        Self {
            time_left_secs: 0,
            running: false,
            updated_at: Instant::now(),
        }
    }
}

impl TimerState {
    fn refresh(&mut self) {
        if !self.running {
            self.updated_at = Instant::now();
            return;
        }

        let now = Instant::now();
        let elapsed = now.duration_since(self.updated_at).as_secs();
        if elapsed == 0 {
            return;
        }

        self.time_left_secs = self.time_left_secs.saturating_sub(elapsed);
        if self.time_left_secs == 0 {
            self.running = false;
        }
        self.updated_at = now;
    }

    fn start(&mut self, seconds: u64) {
        self.time_left_secs = seconds;
        self.running = seconds > 0;
        self.updated_at = Instant::now();
    }

    fn pause(&mut self) {
        self.refresh();
        self.running = false;
    }

    fn resume(&mut self) {
        self.refresh();
        if self.time_left_secs > 0 {
            self.running = true;
            self.updated_at = Instant::now();
        }
    }

    fn extend(&mut self, seconds: u64) {
        self.refresh();
        self.time_left_secs = self.time_left_secs.saturating_add(seconds);
    }

    fn snapshot(&mut self) -> TimerSnapshot {
        self.refresh();
        TimerSnapshot {
            time_left_secs: self.time_left_secs,
            time_left_hms: format_hms(self.time_left_secs),
            running: self.running,
        }
    }
}

fn format_hms(total_secs: u64) -> String {
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

#[derive(Debug, Clone, Serialize)]
struct TimerSnapshot {
    time_left_secs: u64,
    time_left_hms: String,
    running: bool,
}

#[derive(Debug, Deserialize)]
struct SecondsBody {
    seconds: u64,
}

type SharedState = Arc<RwLock<TimerState>>;

type RespBody = Full<Bytes>;

fn json_response<T: Serialize>(status: StatusCode, value: &T) -> Response<RespBody> {
    match serde_json::to_vec(value) {
        Ok(body) => Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(body)))
            .expect("building JSON response should not fail"),
        Err(err) => Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(format!(
                "{{\"error\":\"serialization failure: {err}\"}}"
            ))))
            .expect("building error response should not fail"),
    }
}

fn error_response(status: StatusCode, message: &str) -> Response<RespBody> {
    json_response(status, &serde_json::json!({ "error": message }))
}

async fn parse_seconds_body(req: Request<Incoming>) -> Result<SecondsBody, Response<RespBody>> {
    let bytes = req
        .into_body()
        .collect()
        .await
        .map_err(|err| error_response(StatusCode::BAD_REQUEST, &format!("invalid body: {err}")))?
        .to_bytes();

    serde_json::from_slice::<SecondsBody>(&bytes).map_err(|err| {
        error_response(
            StatusCode::BAD_REQUEST,
            &format!("expected JSON like {{\"seconds\": 300}}: {err}"),
        )
    })
}

async fn handle_request(
    req: Request<Incoming>,
    state: SharedState,
) -> Result<Response<RespBody>, Infallible> {
    let path = req.uri().path();
    let method = req.method().clone();

    let response = match (method, path) {
        (Method::GET, "/time_left") => {
            let mut guard = state.write().await;
            let snapshot = guard.snapshot();
            json_response(StatusCode::OK, &snapshot)
        }
        (Method::POST, "/start") => match parse_seconds_body(req).await {
            Ok(body) => {
                let mut guard = state.write().await;
                guard.start(body.seconds);
                let snapshot = guard.snapshot();
                json_response(StatusCode::OK, &snapshot)
            }
            Err(err) => err,
        },
        (Method::POST, "/pause") => {
            let mut guard = state.write().await;
            guard.pause();
            let snapshot = guard.snapshot();
            json_response(StatusCode::OK, &snapshot)
        }
        (Method::POST, "/resume") => {
            let mut guard = state.write().await;
            guard.resume();
            let snapshot = guard.snapshot();
            json_response(StatusCode::OK, &snapshot)
        }
        (Method::POST, "/extend") => match parse_seconds_body(req).await {
            Ok(body) => {
                let mut guard = state.write().await;
                guard.extend(body.seconds);
                let snapshot = guard.snapshot();
                json_response(StatusCode::OK, &snapshot)
            }
            Err(err) => err,
        },
        _ => error_response(StatusCode::NOT_FOUND, "route not found"),
    };

    Ok(response)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let socket_path =
        std::env::var("TIMER_SOCK").unwrap_or_else(|_| DEFAULT_SOCKET_PATH.to_string());

    if Path::new(&socket_path).exists() {
        std::fs::remove_file(&socket_path)
            .with_context(|| format!("failed to remove existing socket at {}", socket_path))?;
    }

    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("failed to bind unix socket at {}", socket_path))?;

    let state: SharedState = Arc::new(RwLock::new(TimerState::default()));

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let state = Arc::clone(&state);

        tokio::spawn(async move {
            let service = service_fn(move |req| handle_request(req, Arc::clone(&state)));

            if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                eprintln!("connection error: {err}");
            }
        });
    }
}
