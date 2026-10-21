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
| [`benchmarker`](benchmarker/) | The workload that checks the limits of an instance. |

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
| `NOOBS_INSTANCE_BIN` | Path of the `hello-server` binary |
| `NOOBS_BENCHMARKER_BIN` | Path of the `benchmarker` binary |

### Accounts

The first account that registers becomes an administrator. From then on the
admin area is used to promote accounts, disable them, reset passwords and
change the resource limits that are granted to them.

Administrators see every instance and can manage any of them. Regular users
only see and manage their own. The page `/instances` shows every instance of
the machine to an administrator, with the account it belongs to and what it
is using.

### Sites

The **Hello World** and **My site** presets serve the files that are in the
directory of the instance. The user of the instance writes them in the
editor on `/instances/<id>/site`, which holds three files:

| File | What it holds |
| ---- | ------------- |
| `index.html` | The page of the site |
| `style.css` | The style of the site |
| `app.js` | The script of the site |

The editor writes the files straight to the directory of the instance, and
the instance serves them on the next request, so a change is live at once
and does not need a restart. A **My site** instance starts with a small
example, a **Hello World** instance keeps its own page until the user
writes one.

### Instances

Only an administrator creates instances. The form asks for the account the
instance is for, so an instance is always handed out to somebody. The
administrator can move an instance to another account at any time, from the
page of the instance or from the admin area. A regular user can start, stop
and delete the instances that belong to them, but cannot create new ones or
take them away.

An instance runs a preset. There are three of them:

| Preset | What it does |
| ------ | ------------ |
| **Hello World** | A small web server that greets every visitor. Use it to see that an instance runs and answers. |
| **Benchmarker** | A web server that uses as much memory, processor time and disk as it can, so the limits of the instance can be checked. |
| **My site** | An empty website. The user of the instance writes the html, the style and the script of it themselves, in the editor of the instance. |

The benchmarker serves a page that shows how much of the granted resources
is in use, and it can be asked to push against the limits:

| Address | What it does |
| ------- | ------------ |
| `/` | The page with the usage and the limits the kernel gave the process |
| `/stress/memory` | Takes as much memory as it can, until the kernel refuses. The memory is kept, so the dashboard shows the instance using it. |
| `/stress/disk` | Writes a file until the kernel refuses |
| `/stress/cpu` | Uses the processor as much as it can. The panel holds the instance to its share of the processor. |

The processor test keeps the instance busy in the background, so the page of
the benchmarker still answers and the dashboard shows what the instance is
using. The memory test holds on to the memory it took, so the spike stays
visible on the dashboard instead of vanishing right away.

Creating an instance asks for a name and three limits:

| Limit | Meaning |
| ----- | ------- |
| Memory | How much memory the instance may use, in megabytes |
| CPU | How much of a processor the instance may use, in percent. One instance may use at most one core, so the highest value is a hundred percent. |
| Disk | How large a file the instance may write, in megabytes |

The processor limit of an account is the sum of the shares of the instances
it holds, so an account with a limit of two hundred percent may hold four
instances of fifty percent, or two of a hundred percent.

The limits of an instance are drawn from the limits of its owner, so an
account can never hand out more than it has. The panel refuses a request
that would exceed the quota of the account.

The memory and the disk limits are enforced by the kernel with `setrlimit`,
so they hold even if the instance misbehaves:

- `RLIMIT_DATA` caps the memory of the process
- `RLIMIT_FSIZE` caps the size of a written file

The processor limit cannot be done by the kernel, because the kernel only
knows processor time and not the share of a processor. The panel watches
every instance and pauses it for a while with `SIGSTOP` when it uses more
of the processor than it may, and lets it run again with `SIGCONT` when it
has earned the time back. `RLIMIT_CPU` is set as well, as a backstop of one
day for a process that ignores the pauses.

The panel reads the live usage of an instance from `/proc`, and shows it on
the page of the instance. The dashboard asks the panel for the usage of the
instances every two seconds, so the numbers on the page are the ones of
right now.

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
an instance, which is really started and really serves a request. The
benchmarker is tested as well, and it really runs into the limits it is
given.

The processor limit is checked for real: the benchmarker uses the processor
as much as it can, and the test measures the share of the core the instance
was given. The memory of the benchmarker is checked as well, because the
panel has to show the instance using it.

## Layout

| Path | Contents |
| ---- | -------- |
| `panel/src/app.rs` | Routing and request handling |
| `panel/src/site.rs` | The files of the site of an instance |
| `panel/src/auth.rs` | Password hashing, sessions, csrf |
| `panel/src/config.rs` | Configuration |
| `panel/src/db.rs` | SQLite storage |
| `panel/src/instances.rs` | Starting, stopping and limiting instances |
| `panel/src/presets.rs` | The kinds of workload an instance can run |
| `panel/src/models.rs` | Users, instances and their types |
| `panel/src/views.rs` | The HTML pages |
| `panel/src/web.rs` | Request parsing and responses |
