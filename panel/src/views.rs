//! The HTML pages of the panel.

use crate::models::{Instance, Role, User};
use crate::presets::Preset;
use crate::web::escape;

/// The stylesheet shared by every page.
const STYLES: &str = r#"
:root {
  --bg: #0f1419;
  --panel: #161b22;
  --panel-2: #1c2330;
  --border: #2a3240;
  --text: #e6edf3;
  --muted: #8b949e;
  --accent: #4a9eff;
  --accent-2: #2f7fe0;
  --ok: #3fb950;
  --warn: #d29922;
  --err: #f85149;
}
* { box-sizing: border-box; }
body {
  margin: 0;
  font-family: system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
  background: var(--bg);
  color: var(--text);
  font-size: 15px;
  line-height: 1.5;
}
a { color: var(--accent); text-decoration: none; }
a:hover { text-decoration: underline; }
header.top {
  background: var(--panel);
  border-bottom: 1px solid var(--border);
  padding: .7rem 1.2rem;
  display: flex;
  align-items: center;
  gap: 1.2rem;
  flex-wrap: wrap;
}
header.top .brand { font-weight: 700; font-size: 1.05rem; letter-spacing: -.01em; }
header.top nav { display: flex; gap: 1rem; flex: 1; flex-wrap: wrap; }
header.top .who { color: var(--muted); font-size: .9rem; }
header.top .who strong { color: var(--text); }
main { max-width: 1080px; margin: 0 auto; padding: 1.6rem 1.2rem 4rem; }
h1 { font-size: 1.5rem; margin: 0 0 .3rem; letter-spacing: -.02em; }
h2 { font-size: 1.1rem; margin: 1.8rem 0 .8rem; }
p.sub { color: var(--muted); margin: 0 0 1.4rem; }
.card {
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 10px;
  padding: 1.1rem 1.2rem;
  margin-bottom: 1.1rem;
}
.grid { display: grid; gap: 1rem; grid-template-columns: repeat(auto-fill, minmax(290px, 1fr)); }
.stats { display: grid; gap: .8rem; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); margin-bottom: 1.4rem; }
.stat { background: var(--panel); border: 1px solid var(--border); border-radius: 10px; padding: .8rem 1rem; }
.stat .label { color: var(--muted); font-size: .78rem; text-transform: uppercase; letter-spacing: .04em; }
.stat .value { font-size: 1.3rem; font-weight: 600; margin-top: .15rem; }
label { display: block; margin-bottom: .3rem; font-size: .88rem; color: var(--muted); }
input, select {
  width: 100%; padding: .55rem .7rem; background: var(--bg); color: var(--text);
  border: 1px solid var(--border); border-radius: 7px; font-size: .95rem; font-family: inherit;
}
input:focus, select:focus { outline: 2px solid var(--accent-2); outline-offset: -1px; border-color: var(--accent); }
.field { margin-bottom: .9rem; }
select {
  width: 100%; padding: .5rem .6rem; border-radius: 7px; border: 1px solid var(--border);
  background: var(--panel-2); color: var(--text); font-size: .92rem; font-family: inherit;
}
.choices { display: flex; flex-direction: column; gap: .5rem; }
.choice {
  display: grid; grid-template-columns: auto 1fr; gap: .1rem .6rem; align-items: center;
  padding: .6rem .7rem; border: 1px solid var(--border); border-radius: 7px; cursor: pointer;
}
.choice:has(input:checked) { border-color: var(--accent); }
.choice-name { font-weight: 600; }
.choice-text { grid-column: 2; opacity: .7; font-size: .85rem; }
button, .btn {
  display: inline-block; padding: .5rem .95rem; border-radius: 7px; border: 1px solid var(--border);
  background: var(--panel-2); color: var(--text); cursor: pointer; font-size: .92rem; font-family: inherit;
}
button:hover, .btn:hover { border-color: var(--accent); text-decoration: none; }
button.primary, .btn.primary { background: var(--accent-2); border-color: var(--accent-2); color: #fff; }
button.primary:hover { background: var(--accent); }
button.danger { border-color: #5a2b2b; color: #ff9d95; }
button.danger:hover { background: #3d1d1d; border-color: var(--err); }
.row { display: flex; gap: .6rem; flex-wrap: wrap; align-items: center; }
.actions { display: flex; gap: .5rem; flex-wrap: wrap; margin-top: .9rem; }
table { width: 100%; border-collapse: collapse; }
th, td { text-align: left; padding: .6rem .5rem; border-bottom: 1px solid var(--border); font-size: .92rem; }
th { color: var(--muted); font-weight: 600; font-size: .8rem; text-transform: uppercase; letter-spacing: .04em; }
tr:last-child td { border-bottom: none; }
.badge { display: inline-block; padding: .1rem .5rem; border-radius: 999px; font-size: .76rem; font-weight: 600; }
.badge.running { background: rgba(63,185,80,.15); color: var(--ok); }
.badge.stopped { background: rgba(139,148,158,.15); color: var(--muted); }
.badge.crashed { background: rgba(248,81,73,.15); color: var(--err); }
.badge.starting { background: rgba(210,153,34,.15); color: var(--warn); }
.badge.admin { background: rgba(74,158,255,.15); color: var(--accent); }
.badge.user { background: rgba(139,148,158,.15); color: var(--muted); }
.notice { padding: .7rem .9rem; border-radius: 8px; margin-bottom: 1rem; font-size: .92rem; }
.notice.error { background: rgba(248,81,73,.12); border: 1px solid rgba(248,81,73,.35); color: #ffb3ad; }
.notice.ok { background: rgba(63,185,80,.12); border: 1px solid rgba(63,185,80,.35); color: #9ae6a8; }
.muted { color: var(--muted); }
.mono { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: .88rem; }
.bar { height: 6px; background: var(--panel-2); border-radius: 3px; overflow: hidden; margin-top: .3rem; }
.bar > span { display: block; height: 100%; background: var(--accent); }
.auth-wrap { max-width: 380px; margin: 4rem auto; }
.empty { color: var(--muted); text-align: center; padding: 2rem 1rem; }
code { background: var(--panel-2); padding: .1rem .35rem; border-radius: 4px; font-size: .88rem; }
"#;

/// Wrap page content in the shared layout.
fn layout(title: &str, user: Option<&User>, csrf: &str, content: &str) -> String {
    let nav = match user {
        Some(user) => {
            let mut nav = r#"<a href="/dashboard">Dashboard</a>"#.to_string();
            if user.is_admin() {
                nav.push_str(r#"<a href="/instances/new">New instance</a>"#);
            }
            if user.is_admin() {
                nav.push_str(r#"<a href="/admin">Admin</a>"#);
            }
            format!(
                r#"<nav>{nav}</nav><span class="who">Signed in as <strong>{}</strong> ({})</span>
                   <form method="post" action="/logout" style="margin:0">{csrf_field}<button type="submit">Sign out</button></form>"#,
                escape(&user.username),
                escape(user.role.as_db()),
                csrf_field = csrf_field(csrf),
            )
        }
        None => r#"<nav><a href="/login">Sign in</a><a href="/signup">Create account</a></nav>"#
            .to_string(),
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} - noobs-panel</title>
<style>{styles}</style>
</head>
<body>
<header class="top">
  <span class="brand">noobs-panel</span>
  {nav}
</header>
<main>
{content}
</main>
</body>
</html>"#,
        title = escape(title),
        styles = STYLES,
        nav = nav,
        content = content,
    )
}

/// The hidden csrf field that every form carries.
pub fn csrf_field(csrf: &str) -> String {
    format!(
        r#"<input type="hidden" name="_csrf" value="{}">"#,
        escape(csrf)
    )
}

/// The sign in page.
pub fn login_page(csrf: &str, error: Option<&str>) -> String {
    let notice = error
        .map(|msg| format!(r#"<div class="notice error">{}</div>"#, escape(msg)))
        .unwrap_or_default();
    let content = format!(
        r#"<div class="auth-wrap">
<h1>Sign in</h1>
<p class="sub">Welcome back to noobs-panel.</p>
{notice}
<div class="card">
<form method="post" action="/login">
<input type="hidden" name="_csrf" value="{csrf}">
<div class="field"><label for="username">Username</label>
<input id="username" name="username" autocomplete="username" required autofocus></div>
<div class="field"><label for="password">Password</label>
<input id="password" name="password" type="password" autocomplete="current-password" required></div>
<button class="primary" type="submit">Sign in</button>
</form>
</div>
<p class="muted">No account yet? <a href="/signup">Create one</a>.</p>
</div>"#,
        notice = notice,
        csrf = escape(csrf),
    );
    layout("Sign in", None, csrf, &content)
}

/// The registration page.
pub fn signup_page(csrf: &str, error: Option<&str>) -> String {
    let notice = error
        .map(|msg| format!(r#"<div class="notice error">{}</div>"#, escape(msg)))
        .unwrap_or_default();
    let content = format!(
        r#"<div class="auth-wrap">
<h1>Create account</h1>
<p class="sub">Register to start running web server instances.</p>
{notice}
<div class="card">
<form method="post" action="/signup">
<input type="hidden" name="_csrf" value="{csrf}">
<div class="field"><label for="username">Username</label>
<input id="username" name="username" autocomplete="username" required autofocus></div>
<div class="field"><label for="email">Email</label>
<input id="email" name="email" type="email" autocomplete="email" required></div>
<div class="field"><label for="password">Password</label>
<input id="password" name="password" type="password" autocomplete="new-password" required></div>
<button class="primary" type="submit">Create account</button>
</form>
</div>
<p class="muted">Already registered? <a href="/login">Sign in</a>.</p>
</div>"#,
        notice = notice,
        csrf = escape(csrf),
    );
    layout("Create account", None, csrf, &content)
}

/// The user dashboard.
pub fn dashboard_page(
    user: &User,
    csrf: &str,
    instances: &[Instance],
    used_memory: i64,
    used_cpu: i64,
    used_disk: i64,
) -> String {
    let mut cards = String::new();
    for instance in instances {
        cards.push_str(&instance_card(instance));
    }
    if cards.is_empty() {
        cards = if user.is_admin() {
            r#"<div class="card empty">You have no instances yet. <a href="/instances/new">Create one</a>.</div>"#.to_string()
        } else {
            r#"<div class="card empty">You have no instances yet. An administrator has to give you one.</div>"#.to_string()
        };
    }

    let content = format!(
        r#"<h1>Dashboard</h1>
<p class="sub">Welcome back, {username}.</p>
<div class="stats">
  <div class="stat"><div class="label">Instances</div><div class="value">{count} / {max}</div></div>
  <div class="stat"><div class="label">Memory allocated</div><div class="value">{memory} / {memory_max} MB</div></div>
  <div class="stat"><div class="label">CPU time allocated</div><div class="value">{cpu} / {cpu_max} s</div></div>
  <div class="stat"><div class="label">Disk allocated</div><div class="value">{disk} / {disk_max} MB</div></div>
</div>
<div class="card">
  <div class="row" style="justify-content:space-between">
    <h2 style="margin:0">Your instances</h2>
    {new_button}
  </div>
</div>
<div class="grid">{cards}</div>"#,
        username = escape(&user.username),
        count = instances.len(),
        max = user.instance_limit,
        memory = used_memory,
        memory_max = user.memory_limit_mb,
        cpu = used_cpu,
        cpu_max = user.cpu_limit_seconds,
        disk = used_disk,
        disk_max = user.disk_limit_mb,
        cards = cards,
        new_button = if user.is_admin() {
            r#"<a class="btn primary" href="/instances/new">New instance</a>"#
        } else {
            ""
        },
    );
    layout("Dashboard", Some(user), csrf, &content)
}

/// A single instance card.
fn instance_card(instance: &Instance) -> String {
    format!(
        r#"<div class="card">
  <div class="row" style="justify-content:space-between">
    <strong>{name}</strong>
    <span class="badge {state}">{label}</span>
  </div>
  <p class="muted mono" style="margin:.4rem 0 0">Port {port} &middot; {owner}</p>
  <div class="actions"><a class="btn" href="/instances/{id}">Manage</a></div>
</div>"#,
        name = escape(&instance.name),
        state = instance.state.as_db(),
        label = instance.state.label(),
        port = instance.port,
        owner = escape(&instance.owner_username),
        id = instance.id,
    )
}

/// The page for creating a new instance.
pub fn new_instance_page(
    user: &User,
    csrf: &str,
    error: Option<&str>,
    used_memory: i64,
    used_cpu: i64,
    used_disk: i64,
    users: &[User],
) -> String {
    let notice = error
        .map(|msg| format!(r#"<div class="notice error">{}</div>"#, escape(msg)))
        .unwrap_or_default();
    let presets = preset_options();
    let owners = owner_options(users, user.id);
    let content = format!(
        r#"<h1>New instance</h1>
<p class="sub">Create a web server and hand it to an account.</p>
{notice}
<div class="card">
<form method="post" action="/instances/new">
<input type="hidden" name="_csrf" value="{csrf}">
<div class="field"><label for="name">Name</label>
<input id="name" name="name" placeholder="my-web" required autofocus></div>
<div class="field"><label for="owner">For the account</label>
<select id="owner" name="owner" required>{owners}</select></div>
<div class="field"><label>Preset</label>
<div class="choices">{presets}</div></div>
<div class="field"><label for="memory">Memory limit (MB)</label>
<input id="memory" name="memory" type="number" min="16" max="{max_memory}" value="128" required></div>
<div class="field"><label for="cpu">CPU time limit (seconds)</label>
<input id="cpu" name="cpu" type="number" min="1" max="{max_cpu}" value="30" required></div>
<div class="field"><label for="disk">Disk limit (MB)</label>
<input id="disk" name="disk" type="number" min="16" max="{max_disk}" value="256" required></div>
<button class="primary" type="submit">Create instance</button>
</form>
</div>
<div class="card">
  <h2 style="margin-top:0">Your resources</h2>
  <table>
    <tr><th>Resource</th><th>Allocated</th><th>Limit</th></tr>
    <tr><td>Memory</td><td>{used_memory} MB</td><td>{max_memory} MB</td></tr>
    <tr><td>CPU time</td><td>{used_cpu} s</td><td>{max_cpu} s</td></tr>
    <tr><td>Disk</td><td>{used_disk} MB</td><td>{max_disk} MB</td></tr>
  </table>
</div>"#,
        notice = notice,
        csrf = escape(csrf),
        presets = presets,
        owners = owners,
        max_memory = user.memory_limit_mb,
        max_cpu = user.cpu_limit_seconds,
        max_disk = user.disk_limit_mb,
        used_memory = used_memory,
        used_cpu = used_cpu,
        used_disk = used_disk,
    );
    layout("New instance", Some(user), csrf, &content)
}

/// The presets, as a set of radio buttons.
fn preset_options() -> String {
    Preset::ALL
        .iter()
        .map(|preset| {
            format!(
                r#"<label class="choice"><input type="radio" name="preset" value="{slug}"{checked}>
<span class="choice-name">{label}</span><span class="choice-text">{text}</span></label>"#,
                slug = escape(preset.slug()),
                checked = if *preset == Preset::HelloWorld {
                    " checked"
                } else {
                    ""
                },
                label = escape(preset.label()),
                text = escape(preset.description()),
            )
        })
        .collect()
}

/// The accounts an instance can be handed to.
fn owner_options(users: &[User], selected: i64) -> String {
    users
        .iter()
        .filter(|user| user.is_active)
        .map(|user| {
            format!(
                r#"<option value="{id}"{selected}>{name}</option>"#,
                id = user.id,
                selected = if user.id == selected { " selected" } else { "" },
                name = escape(&user.username),
            )
        })
        .collect()
}

/// The detail page of a single instance.
#[allow(clippy::too_many_arguments)]
pub fn instance_page(
    user: &User,
    instance: &Instance,
    csrf: &str,
    memory_kb: u64,
    cpu_seconds: u64,
    disk_kb: u64,
    is_admin: bool,
    users: &[User],
) -> String {
    let content = format!(
        r#"<h1>{name}</h1>
<p class="sub">Owned by {owner} &middot; <span class="badge {state}">{label}</span></p>
<div class="stats">
  <div class="stat"><div class="label">Port</div><div class="value">{port}</div></div>
  <div class="stat"><div class="label">Memory</div><div class="value">{memory_mb} MB</div></div>
  <div class="stat"><div class="label">CPU time</div><div class="value">{cpu_limit} s</div></div>
  <div class="stat"><div class="label">Disk</div><div class="value">{disk_limit} MB</div></div>
</div>
<div class="card">
  <h2 style="margin-top:0">Controls</h2>
  <div class="actions">
    <form method="post" action="/instances/{id}/start" style="margin:0">
      <input type="hidden" name="_csrf" value="{csrf}">
      <button class="primary" type="submit">Start</button>
    </form>
    <form method="post" action="/instances/{id}/stop" style="margin:0">
      <input type="hidden" name="_csrf" value="{csrf}">
      <button type="submit">Stop</button>
    </form>
    <form method="post" action="/instances/{id}/delete" style="margin:0"
          onsubmit="return confirm('Delete this instance? This cannot be undone.');">
      <input type="hidden" name="_csrf" value="{csrf}">
      <button class="danger" type="submit">Delete</button>
    </form>
  </div>
</div>
<div class="card">
  <h2 style="margin-top:0">Live usage</h2>
  <table>
    <tr><th>Resource</th><th>In use</th><th>Limit</th></tr>
    <tr><td>Memory</td><td>{memory_kb} KB</td><td>{memory_limit_mb} MB</td></tr>
    <tr><td>CPU time</td><td>{cpu_seconds} s</td><td>{cpu_limit} s</td></tr>
    <tr><td>Disk</td><td>{disk_kb} KB</td><td>{disk_limit_mb} MB</td></tr>
  </table>
</div>
<div class="card">
  <h2 style="margin-top:0">Details</h2>
  <table>
    <tr><th>Url</th><td class="mono">{url}</td></tr>
    <tr><th>Process id</th><td class="mono">{pid}</td></tr>
    <tr><th>Preset</th><td>{preset_label}</td></tr>
    {owner_row}
  </table>
</div>
{assign_card}"#,
        name = escape(&instance.name),
        owner = escape(&instance.owner_username),
        state = instance.state.as_db(),
        label = instance.state.label(),
        port = instance.port,
        memory_mb = instance.memory_limit_mb,
        cpu_limit = instance.cpu_limit_seconds,
        disk_limit = instance.disk_limit_mb,
        id = instance.id,
        csrf = escape(csrf),
        memory_kb = memory_kb,
        cpu_seconds = cpu_seconds,
        disk_kb = disk_kb,
        memory_limit_mb = instance.memory_limit_mb,
        disk_limit_mb = instance.disk_limit_mb,
        url = escape(&instance.url()),
        pid = instance
            .pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "-".to_string()),
        preset_label = escape(instance.preset.label()),
        owner_row = if is_admin {
            format!(
                r#"<tr><th>Owner</th><td><a href="/admin/users/{owner_id}">{}</a></td></tr>"#,
                escape(&instance.owner_username),
                owner_id = instance.owner_id
            )
        } else {
            String::new()
        },
        assign_card = if is_admin {
            let options = owner_options(users, instance.owner_id);
            format!(
                r#"<div class="card">
  <h2 style="margin-top:0">Hand to another account</h2>
  <form method="post" action="/instances/{id}/owner">
    <input type="hidden" name="_csrf" value="{csrf}">
    <div class="field"><label for="owner-{id}">Owner</label>
    <select id="owner-{id}" name="owner">{options}</select></div>
    <button type="submit">Hand over</button>
  </form>
</div>"#,
                id = instance.id,
                csrf = escape(csrf),
                options = options,
            )
        } else {
            String::new()
        },
    );
    layout(&instance.name, Some(user), csrf, &content)
}

/// The admin overview.
pub fn admin_page(admin: &User, csrf: &str, users: &[User], instances: &[Instance]) -> String {
    let mut rows = String::new();
    for user in users {
        rows.push_str(&format!(
            r#"<tr>
  <td><a href="/admin/users/{id}">{username}</a></td>
  <td><span class="badge {role}">{role}</span></td>
  <td>{email}</td>
  <td>{memory} MB</td>
  <td>{cpu} s</td>
  <td>{disk} MB</td>
  <td>{instances} / {max}</td>
  <td>{state}</td>
</tr>"#,
            id = user.id,
            username = escape(&user.username),
            role = user.role.as_db(),
            email = escape(&user.email),
            memory = user.memory_limit_mb,
            cpu = user.cpu_limit_seconds,
            disk = user.disk_limit_mb,
            instances = user.instance_limit,
            max = user.instance_limit,
            state = if user.is_active { "Active" } else { "Disabled" },
        ));
    }

    // Every instance, with the account it belongs to and the account it
    // could be handed to.
    let mut instance_rows = String::new();
    for instance in instances {
        let options = owner_options(users, instance.owner_id);
        instance_rows.push_str(&format!(
            r#"<tr>
  <td><a href="/instances/{id}">{name}</a></td>
  <td>{preset}</td>
  <td><a href="/admin/users/{owner_id}">{owner}</a></td>
  <td>{port}</td>
  <td>{memory} MB</td>
  <td>{cpu} s</td>
  <td>{disk} MB</td>
  <td><span class="badge {state}">{label}</span></td>
  <td>
    <form method="post" action="/instances/{id}/owner" class="row" style="margin:0">
      <input type="hidden" name="_csrf" value="{csrf}">
      <select name="owner" style="width:auto">{options}</select>
      <button type="submit">Hand over</button>
    </form>
  </td>
</tr>"#,
            id = instance.id,
            name = escape(&instance.name),
            preset = escape(instance.preset.label()),
            owner_id = instance.owner_id,
            owner = escape(&instance.owner_username),
            port = instance.port,
            memory = instance.memory_limit_mb,
            cpu = instance.cpu_limit_seconds,
            disk = instance.disk_limit_mb,
            state = instance.state.as_db(),
            label = instance.state.label(),
            csrf = escape(csrf),
            options = options,
        ));
    }
    if instance_rows.is_empty() {
        instance_rows = r#"<tr><td colspan="9">There are no instances yet.</td></tr>"#.to_string();
    }

    let content = format!(
        r#"<h1>Administration</h1>
<p class="sub">Accounts and instances on this panel.</p>
<div class="stats">
  <div class="stat"><div class="label">Accounts</div><div class="value">{user_count}</div></div>
  <div class="stat"><div class="label">Admins</div><div class="value">{admin_count}</div></div>
  <div class="stat"><div class="label">Instances</div><div class="value">{instance_count}</div></div>
</div>
<div class="card">
  <h2 style="margin-top:0">Accounts</h2>
  <table>
    <tr><th>Username</th><th>Role</th><th>Email</th><th>Memory</th><th>CPU</th><th>Disk</th><th>Instances</th><th>State</th></tr>
    {rows}
  </table>
</div>
<div class="card">
  <div class="row" style="justify-content:space-between">
    <h2 style="margin:0">Instances</h2>
    <a class="btn primary" href="/instances/new">New instance</a>
  </div>
  <table>
    <tr><th>Name</th><th>Preset</th><th>Owner</th><th>Port</th><th>Memory</th><th>CPU</th><th>Disk</th><th>State</th><th>Hand over</th></tr>
    {instance_rows}
  </table>
</div>"#,
        user_count = users.len(),
        admin_count = users.iter().filter(|u| u.is_admin()).count(),
        instance_count = instances.len(),
        rows = rows,
        instance_rows = instance_rows,
    );
    layout("Administration", Some(admin), csrf, &content)
}

/// The admin page of a single account.
pub fn admin_user_page(
    admin: &User,
    user: &User,
    instances: &[Instance],
    csrf: &str,
    notice: Option<&str>,
) -> String {
    let banner = notice
        .map(|msg| format!(r#"<div class="notice ok">{}</div>"#, escape(msg)))
        .unwrap_or_default();
    let mut rows = String::new();
    for instance in instances {
        rows.push_str(&format!(
            r#"<tr><td><a href="/instances/{id}">{name}</a></td><td>{port}</td>
<td><span class="badge {state}">{label}</span></td><td>{memory} MB</td><td>{cpu} s</td></tr>"#,
            id = instance.id,
            name = escape(&instance.name),
            port = instance.port,
            state = instance.state.as_db(),
            label = instance.state.label(),
            memory = instance.memory_limit_mb,
            cpu = instance.cpu_limit_seconds,
        ));
    }

    let content = format!(
        r#"<h1>{username}</h1>
<p class="sub">Account details and resource limits.</p>
{banner}
<div class="card">
  <h2 style="margin-top:0">Account</h2>
  <table>
    <tr><th>Username</th><td>{username}</td></tr>
    <tr><th>Email</th><td>{email}</td></tr>
    <tr><th>Role</th><td><span class="badge {role}">{role}</span></td></tr>
    <tr><th>State</th><td>{state}</td></tr>
    <tr><th>Joined</th><td class="mono">{created}</td></tr>
  </table>
  <div class="actions">
    <form method="post" action="/admin/users/{id}/role" style="margin:0">
      <input type="hidden" name="_csrf" value="{csrf}">
      <button type="submit">{role_button}</button>
    </form>
    <form method="post" action="/admin/users/{id}/active" style="margin:0">
      <input type="hidden" name="_csrf" value="{csrf}">
      <button type="submit">{active_button}</button>
    </form>
    <form method="post" action="/admin/users/{id}/password" style="margin:0">
      <input type="hidden" name="_csrf" value="{csrf}">
      <input name="password" type="password" placeholder="New password" style="width:auto;display:inline-block;margin-right:.5rem" required>
      <button type="submit">Set password</button>
    </form>
  </div>
</div>
<div class="card">
  <h2 style="margin-top:0">Resource limits</h2>
  <form method="post" action="/admin/users/{id}/limits">
    <input type="hidden" name="_csrf" value="{csrf}">
    <div class="row">
      <div class="field" style="flex:1"><label>Memory (MB)</label>
        <input name="memory" type="number" min="16" value="{memory}"></div>
      <div class="field" style="flex:1"><label>CPU time (s)</label>
        <input name="cpu" type="number" min="1" value="{cpu}"></div>
      <div class="field" style="flex:1"><label>Disk (MB)</label>
        <input name="disk" type="number" min="16" value="{disk}"></div>
      <div class="field" style="flex:1"><label>Instances</label>
        <input name="instances" type="number" min="0" value="{max}"></div>
    </div>
    <button class="primary" type="submit">Save limits</button>
  </form>
</div>
<div class="card">
  <h2 style="margin-top:0">Instances</h2>
  <table>
    <tr><th>Name</th><th>Port</th><th>State</th><th>Memory</th><th>CPU</th></tr>
    {rows}
  </table>
</div>"#,
        username = escape(&user.username),
        email = escape(&user.email),
        role = user.role.as_db(),
        state = if user.is_active { "Active" } else { "Disabled" },
        created = user.created_at,
        id = user.id,
        csrf = escape(csrf),
        role_button = if user.is_admin() {
            "Remove admin"
        } else {
            "Make admin"
        },
        active_button = if user.is_active {
            "Disable account"
        } else {
            "Enable account"
        },
        memory = user.memory_limit_mb,
        cpu = user.cpu_limit_seconds,
        disk = user.disk_limit_mb,
        max = user.instance_limit,
        rows = rows,
    );
    layout(&user.username, Some(admin), csrf, &content)
}

/// A plain error page.
pub fn error_page(user: Option<&User>, csrf: &str, status: u16, message: &str) -> String {
    let content = format!(
        r#"<div class="auth-wrap">
<h1>{status}</h1>
<p class="sub">{message}</p>
<p><a href="/">Back to the panel</a></p>
</div>"#,
        status = status,
        message = escape(message),
    );
    layout(&format!("Error {status}"), user, csrf, &content)
}

/// A page that says the session is missing.
pub fn login_required_page(message: &str) -> String {
    login_page("", Some(message))
}

/// The role of a user as a badge, used by the admin tables.
pub fn role_badge(role: Role) -> String {
    format!(
        r#"<span class="badge {}">{}</span>"#,
        role.as_db(),
        role.as_db()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Instance, InstanceState};

    fn user() -> User {
        User {
            id: 1,
            username: "alice".into(),
            email: "alice@example.com".into(),
            role: Role::User,
            is_active: true,
            created_at: 0,
            memory_limit_mb: 256,
            cpu_limit_seconds: 60,
            disk_limit_mb: 1024,
            instance_limit: 5,
        }
    }

    fn instance() -> Instance {
        Instance {
            id: 7,
            name: "web".into(),
            owner_id: 1,
            owner_username: "alice".into(),
            port: 8100,
            state: InstanceState::Running,
            pid: Some(42),
            memory_limit_mb: 128,
            cpu_limit_seconds: 30,
            disk_limit_mb: 512,
            created_at: 0,
            exit_code: None,
            preset: Preset::HelloWorld,
        }
    }

    #[test]
    fn pages_escape_user_supplied_names() {
        let html = dashboard_page(&user(), "csrf", &[], 0, 0, 0);
        assert!(html.contains("alice"));
        assert!(!html.contains("<script"));

        let html = instance_page(&user(), &instance(), "csrf", 0, 0, 0, false, &[]);
        assert!(html.contains("8100"));
    }

    #[test]
    fn layout_marks_the_current_user() {
        let html = dashboard_page(&user(), "csrf", &[], 0, 0, 0);
        assert!(html.contains("Signed in as"));
        assert!(html.contains("alice"));
    }

    #[test]
    fn admin_links_only_appear_for_admins() {
        let mut admin = user();
        admin.role = Role::Admin;
        let html = dashboard_page(&admin, "csrf", &[], 0, 0, 0);
        assert!(html.contains(r#"href="/admin""#));
    }
}
