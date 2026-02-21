use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};

const DEFAULT_SOCKET_PATH: &str = "/tmp/timer.sock";

#[derive(Debug, Parser)]
#[command(name = "timer", about = "CLI wrapper for timersock")]
struct Cli {
    #[arg(long, env = "TIMER_SOCK", default_value = DEFAULT_SOCKET_PATH)]
    socket: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Status,
    Start { seconds: u64 },
    Pause,
    Resume,
    Toggle,
    Extend { seconds: u64 },
}

#[derive(Debug, Deserialize)]
struct TimerSnapshot {
    time_left_secs: u64,
    time_left_hms: String,
    running: bool,
}

#[derive(Debug, Serialize)]
struct SecondsBody {
    seconds: u64,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Status => {
            let snapshot: TimerSnapshot = send(&cli.socket, "GET", "/time_left", None)?;
            print_snapshot(&snapshot);
        }
        Command::Start { seconds } => {
            let snapshot: TimerSnapshot = send(
                &cli.socket,
                "POST",
                "/start",
                Some(serde_json::to_string(&SecondsBody { seconds })?),
            )?;
            print_snapshot(&snapshot);
        }
        Command::Pause => {
            let snapshot: TimerSnapshot = send(&cli.socket, "POST", "/pause", None)?;
            print_snapshot(&snapshot);
        }
        Command::Resume => {
            let snapshot: TimerSnapshot = send(&cli.socket, "POST", "/resume", None)?;
            print_snapshot(&snapshot);
        }
        Command::Toggle => {
            let snapshot: TimerSnapshot = send(&cli.socket, "POST", "/toggle", None)?;
            print_snapshot(&snapshot);
        }
        Command::Extend { seconds } => {
            let snapshot: TimerSnapshot = send(
                &cli.socket,
                "POST",
                "/extend",
                Some(serde_json::to_string(&SecondsBody { seconds })?),
            )?;
            print_snapshot(&snapshot);
        }
    }

    Ok(())
}

fn send<T: for<'de> Deserialize<'de>>(
    socket_path: &PathBuf,
    method: &str,
    path: &str,
    payload: Option<String>,
) -> Result<T> {
    let mut stream = UnixStream::connect(socket_path)
        .with_context(|| format!("failed to connect to socket {}", socket_path.display()))?;

    let body = payload.unwrap_or_default();
    let content_header = if body.is_empty() {
        String::new()
    } else {
        format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        )
    };

    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{content_header}\r\n{body}"
    );

    stream
        .write_all(request.as_bytes())
        .context("failed writing request")?;
    stream.flush().context("failed flushing request")?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .context("failed reading response")?;

    let (status_line, rest) = response
        .split_once("\r\n")
        .context("malformed HTTP response: missing status line")?;
    let status_code = parse_status_code(status_line)?;

    let (_, body) = rest
        .split_once("\r\n\r\n")
        .context("malformed HTTP response: missing body separator")?;

    if !(200..300).contains(&status_code) {
        bail!("server returned {}: {}", status_code, body);
    }

    let parsed = serde_json::from_str::<T>(body).context("failed to parse JSON response")?;
    Ok(parsed)
}

fn parse_status_code(status_line: &str) -> Result<u16> {
    let mut parts = status_line.split_whitespace();
    let _http_version = parts
        .next()
        .context("malformed status line: missing HTTP version")?;
    let code = parts
        .next()
        .context("malformed status line: missing status code")?
        .parse::<u16>()
        .context("malformed status line: invalid status code")?;
    Ok(code)
}

fn print_snapshot(snapshot: &TimerSnapshot) {
    println!(
        "time_left={} time_left_secs={} running={}",
        snapshot.time_left_hms, snapshot.time_left_secs, snapshot.running
    );
}
