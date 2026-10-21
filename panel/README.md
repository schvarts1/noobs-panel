# panel

The noobs-panel control panel: accounts, administrators and instances.

See the [workspace README](../README.md) for the full documentation.

## Running

```sh
cargo run --release -p panel
```

Options:

```sh
noobs-panel --port 8080 --host 127.0.0.1 --data-dir ~/.noobs-panel
```

The panel stores its database and the working directories of the instances in
the data directory. The first account that registers becomes an
administrator.

Only an administrator creates instances and hands them out to an account.
An instance runs a preset: the hello world web server, the benchmarker that
checks the limits of the instance, or a site the user writes themselves.

The processor limit of an instance is a share of a core, in percent. The
panel watches every instance and pauses it when it uses more of the
processor than it may. The dashboard asks for the usage of the instances
every two seconds, so the numbers on the page are the ones of right now.

The hello world and the site preset serve the files of the directory of the
instance, so the user can write the html, the style and the script of their
site in the editor at `/instances/<id>/site`.

## Layout

| File | Contents |
| ---- | -------- |
| `src/app.rs` | Routing and request handling |
| `src/auth.rs` | Password hashing, sessions, csrf |
| `src/config.rs` | Configuration |
| `src/db.rs` | SQLite storage |
| `src/instances.rs` | Starting, stopping and limiting instances |
| `src/presets.rs` | The kinds of workload an instance can run |
| `src/site.rs` | The files of the site of an instance |
| `src/models.rs` | Users, instances and their types |
| `src/views.rs` | The HTML pages |
| `src/web.rs` | Request parsing and responses |
