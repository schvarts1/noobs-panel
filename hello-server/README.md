# hello-server

A tiny "Hello, World!" web server in Rust that **always finds a free port**.

- Defaults to port **8000**.
- Custom port via `--port` / `-p` (or the `PORT` environment variable).
- If the requested port is busy — whether it is the default 8000 or a custom
  one — the server automatically searches for the **nearest available port**,
  takes it, and prints the URL that is actually in use.

## Usage

```sh
cargo run --release

# custom port
cargo run --release -- --port 3000

# bind a specific interface
cargo run --release -- --host 127.0.0.1 --port 3000

# via environment variable
PORT=3000 cargo run --release
```

## Output

```
  Hello, World! server is running
  ---------------------------------
  URL:        http://localhost:8000
  Port:       8000
  Stop with:  Ctrl+C
```

When the preferred port is taken:

```
  Hello, World! server is running
  ---------------------------------
  URL:        http://localhost:8001
  Port:       8001
  Note:       port 8000 was busy, using 8001 instead
  Stop with:  Ctrl+C
```

Open the printed URL in a browser to see the page.

## Endpoints

| Path      | Response                          |
| --------- | --------------------------------- |
| `/`       | The Hello, World! page (HTML)     |
| `/health` | `ok` — handy for a quick check    |
| anything  | `404 Not Found`                   |

## How the port search works

1. Try to bind the preferred port. If it works, use it.
2. If it fails with `AddrInUse`, walk outwards from it, one step at a time:
   `port+1`, `port-1`, `port+2`, `port-2`, ... The higher port wins ties, so
   the result is genuinely the *nearest* free port.
3. The downward search stops at port 1024, and the upward search stops at
   65535. If nothing is found within 100 steps, the server exits with an error.

The socket is bound and then **kept** rather than being released and rebound
later, so there is no window in which another process could steal the port.

## Tests

```sh
cargo test
```

The suite covers argument parsing, the candidate ordering of the port search,
and end-to-end runs of the real binary over HTTP (including the fallback when
the preferred port is occupied).

## Layout

| File             | Contents                                        |
| ---------------- | ----------------------------------------------- |
| `src/args.rs`    | Command line and environment parsing            |
| `src/port.rs`    | Port search and fallback logic                  |
| `src/server.rs`  | The HTTP handler and the hello world page       |
| `src/main.rs`    | Wiring of the three                             |
