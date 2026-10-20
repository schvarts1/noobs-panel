# noobs-panel

A small panel for managing web server instances, in the spirit of Pterodactyl
but much simpler.

- Accounts with sign up and sign in
- Administrator accounts with their own area
- Instances that are real web server processes
- Resource limits per account and per instance, enforced by the kernel

## Projects

| Project | Description |
| ------- | ----------- |
| [`panel`](panel/) | The panel itself: accounts, administrators and instances. |
| [`hello-server`](hello-server/) | The tiny web server that instances run. |

## Quick start

```sh
cargo build --release

# The panel finds hello-server next to its own binary.
./target/release/noobs-panel
```

Open the url it prints, register an account, and the first account becomes
an administrator.

## The panel

```sh
noobs-panel [OPTIONS]

OPTIONS:
    -p, --port <PORT>       Port the panel listens on [default: 8080]
    -b, --host <HOST>       Address the panel binds to [default: 127.0.0.1]
    -d, --data-dir <DIR>    Where the database and instance data are stored
    -h, --help              Print this help and exit
    -V, --version           Print version information and exit
```

If the requested port is busy, the panel moves to the nearest free one, the
same way `hello-server` does.

### Environment

| Variable | Meaning |
| -------- | ------- |
| `NOOBS_PANEL_PORT` | Same as `--port` |
| `NOOBS_PANEL_HOST` | Same as `--host` |
| `NOOBS_PANEL_DATA` | Same as `--data-dir` |
| `NOOBS_PANEL_INSTANCE_PORTS` | Port range for instances, as `START-END` |
| `NOOBS_INSTANCE_BIN` | Path of the binary used to run instances |

### Accounts

The first account that registers becomes an administrator. From then on the
admin area is used to promote accounts, disable them, reset passwords and
change the resource limits that are granted to them.

Administrators see every instance and can manage any of them. Regular users
only see and manage their own.

### Instances

An instance is a `hello-server` process. Creating one asks for a name and
three limits:

| Limit | Meaning |
| ----- | ------- |
| Memory | How much memory the instance may use, in megabytes |
| CPU time | How much processor time the instance may consume, in seconds |
| Disk | How large a file the instance may write, in megabytes |

The limits of an instance are drawn from the limits of its owner, so an
account can never hand out more than it has. The panel refuses a request
that would exceed the quota of the account.

The limits are enforced by the kernel with `setrlimit`, so they hold even if
the instance misbehaves:

- `RLIMIT_DATA` caps the memory of the process
- `RLIMIT_CPU` caps the processor time
- `RLIMIT_FSIZE` caps the size of a written file

The panel reads the live usage of an instance from `/proc`, and shows it on
the page of the instance.

> On Android the memory limit uses `RLIMIT_DATA` rather than `RLIMIT_AS`,
> because the dynamic linker reserves a very large shadow memory region and
> would fail to start at all under an address space limit.

## Tests

```sh
cargo test
```

The suite covers the argument parsing, the port search, the database, the
password hashing, and the whole panel over HTTP: signing up, signing in, the
csrf protection, the role checks, the resource quotas and the life cycle of
an instance, which is really started and really serves a request.

## Layout

| Path | Contents |
| ---- | -------- |
| `panel/src/app.rs` | Routing and request handling |
| `panel/src/auth.rs` | Password hashing, sessions, csrf |
| `panel/src/config.rs` | Configuration |
| `panel/src/db.rs` | SQLite storage |
| `panel/src/instances.rs` | Starting, stopping and limiting instances |
| `panel/src/models.rs` | Users, instances and their types |
| `panel/src/views.rs` | The HTML pages |
| `panel/src/web.rs` | Request parsing and responses |
