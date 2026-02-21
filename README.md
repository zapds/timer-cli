# Timer

A single Rust package with two binaries:
- `timer`: CLI client to control and read one timer.
- `timersock`: UNIX socket server that stores timer state.

## Server
Start the server (default socket: `/tmp/timer.sock`):

```bash
cargo run --bin timersock
```

Optional socket override:

```bash
TIMER_SOCK=/tmp/mytimer.sock cargo run --bin timersock
```

## CLI
Run commands against the server:

```bash
cargo run --bin timer -- status
cargo run --bin timer -- start 1500
cargo run --bin timer -- pause
cargo run --bin timer -- resume
cargo run --bin timer -- extend 60
```

CLI output includes both raw seconds and formatted time (`hh:mm:ss`).

Optional socket override:

```bash
cargo run --bin timer -- --socket /tmp/mytimer.sock status
```

## Install
Install both binaries from this package:

```bash
cargo install --path .
```

Install from GitHub:

```bash
cargo install --git https://github.com/zapds/timer-cli --bins
```
