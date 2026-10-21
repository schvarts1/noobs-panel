//! The panel application: routing, request handling and session handling.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::auth::{self, SESSION_COOKIE};
use crate::config::Config;
use crate::db::Database;
use crate::instances::InstanceManager;
use crate::models::{Instance, Role, Usage, User};
use crate::presets::Preset;
use crate::site;
use crate::views;
use crate::web::{self, WebRequest};

/// The application state shared by every request.
pub struct App {
    /// The panel configuration.
    pub config: Config,
    /// The database handle.
    pub db: Database,
    /// The running instance processes.
    pub manager: Arc<InstanceManager>,
}

/// The result of handling a request.
pub type Response = tiny_http::Response<std::io::Cursor<Vec<u8>>>;

/// The share of one core an instance may use at most, in percent.
const MAX_INSTANCE_CPU_PERCENT: i64 = 100;

/// The share of the processor an account may hold at most, in percent.
///
/// An account may hold more than a whole processor, because it may have
/// more than one instance.
const MAX_ACCOUNT_CPU_PERCENT: i64 = 800;

/// The current time in seconds since the epoch.
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// The identity of the caller, if they are signed in.
pub struct Identity {
    /// The signed in account.
    pub user: User,
    /// The csrf token of their session.
    pub csrf: String,
}

/// Handle a request.
pub fn handle(app: &App, request: &WebRequest) -> Response {
    let segments = request.segments();
    let method = request.method().to_string();

    match (method.as_str(), segments.as_slice()) {
        ("GET", []) => web::redirect("/dashboard"),
        ("GET", ["login"]) => login_page(app, request),
        ("POST", ["login"]) => login_submit(app, request),
        ("GET", ["signup"]) => signup_page(app, request),
        ("POST", ["signup"]) => signup_submit(app, request),
        ("POST", ["logout"]) => logout(app, request),
        ("GET", ["dashboard"]) => dashboard(app, request),
        ("GET", ["api", "usage"]) => api_usage(app, request),
        ("GET", ["instances"]) => admin_instances(app, request),
        ("GET", ["instances", "new"]) => new_instance_page(app, request),
        ("POST", ["instances", "new"]) => create_instance(app, request),
        ("POST", ["instances", id, "owner"]) => assign_instance(app, request, id),
        ("GET", ["instances", id, "site"]) => site_editor(app, request, id),
        ("POST", ["instances", id, "site"]) => site_save(app, request, id),
        ("GET", ["instances", id]) => instance_detail(app, request, id),
        ("POST", ["instances", id, "start"]) => start_instance(app, request, id),
        ("POST", ["instances", id, "stop"]) => stop_instance(app, request, id),
        ("POST", ["instances", id, "delete"]) => delete_instance(app, request, id),
        ("GET", ["admin"]) => admin_page(app, request),
        ("GET", ["admin", "users", id]) => admin_user_page(app, request, id),
        ("POST", ["admin", "users", id, "role"]) => admin_set_role(app, request, id),
        ("POST", ["admin", "users", id, "active"]) => admin_set_active(app, request, id),
        ("POST", ["admin", "users", id, "limits"]) => admin_set_limits(app, request, id),
        ("POST", ["admin", "users", id, "password"]) => admin_set_password(app, request, id),
        _ => not_found(app, request),
    }
}

/// The signed in user of a request, if there is one.
pub fn identity(app: &App, request: &WebRequest) -> Option<Identity> {
    let token = request.cookie(SESSION_COOKIE)?;
    let (user_id, csrf) = app.db.session_user(token, now()).ok()??;
    let user = app.db.user_by_id(user_id).ok()??;
    if !user.is_active {
        return None;
    }
    Some(Identity { user, csrf })
}

/// A response for a request that needs a signed in user.
fn require_identity<'a>(app: &'a App, request: &'a WebRequest) -> Result<Identity, Response> {
    match identity(app, request) {
        Some(identity) => Ok(identity),
        None => Err(web::html_with_cookie(
            200,
            views::login_required_page("Please sign in to continue."),
            SESSION_COOKIE,
            "",
            0,
            true,
        )),
    }
}

/// A response for a request that needs an administrator.
fn require_admin<'a>(app: &'a App, request: &'a WebRequest) -> Result<Identity, Response> {
    let identity = require_identity(app, request)?;
    if !identity.user.is_admin() {
        return Err(web::html(
            403,
            views::error_page(
                Some(&identity.user),
                &identity.csrf,
                403,
                "You do not have permission to view this page.",
            ),
        ));
    }
    Ok(identity)
}

/// Check the csrf token of a form.
fn check_csrf(identity: &Identity, request: &WebRequest) -> Result<(), Response> {
    let provided = request.trimmed_field("_csrf");
    if !auth::csrf_matches(&identity.csrf, &provided) {
        return Err(web::html(
            403,
            views::error_page(
                Some(&identity.user),
                &identity.csrf,
                403,
                "The form was not submitted correctly. Please try again.",
            ),
        ));
    }
    Ok(())
}

/// Hand out a new csrf token and remember it.
fn issue_csrf(app: &App) -> String {
    let token = auth::generate_csrf_token();
    let _ = app
        .db
        .save_csrf_token(&token, now(), now() + crate::config::SESSION_TTL_SECONDS);
    token
}

/// The csrf token of a form that is shown before signing in.
///
/// A token that the panel handed out before is reused, so that a page that
/// was opened earlier still matches the cookie. A form therefore keeps
/// working when the page was reloaded, when it was opened in a second tab,
/// or when the back button was used. A token that the panel does not know
/// is replaced by a fresh one.
fn anonymous_csrf(app: &App, request: &WebRequest) -> String {
    if let Some(token) = request.cookie(auth::CSRF_COOKIE) {
        if auth::is_csrf_token(token) && app.db.csrf_token_is_valid(token, now()).unwrap_or(false) {
            return token.to_string();
        }
    }
    issue_csrf(app)
}

/// Check the csrf token of a form that was shown before signing in.
///
/// The cookie and the form have to carry the same token, and the panel has
/// to know the token, so a cookie that was put in the browser by somebody
/// else is not enough.
fn check_form_csrf(app: &App, request: &WebRequest) -> bool {
    let expected = request.cookie(auth::CSRF_COOKIE).unwrap_or_default();
    let provided = request.trimmed_field("_csrf");
    if !auth::csrf_matches(expected, &provided) {
        return false;
    }
    app.db.csrf_token_is_valid(expected, now()).unwrap_or(false)
}

/// The live usage of the instances of the caller, as json.
///
/// The dashboard asks for this every couple of seconds, so the numbers on
/// the page are the ones of right now.
fn api_usage(app: &App, request: &WebRequest) -> Response {
    let identity = match require_identity(app, request) {
        Ok(identity) => identity,
        Err(response) => return response,
    };

    // An administrator sees the usage of every instance, a normal user only
    // the ones that belong to them.
    let instances = if identity.user.is_admin() {
        app.db.list_instances(None).unwrap_or_default()
    } else {
        app.db
            .list_instances(Some(identity.user.id))
            .unwrap_or_default()
    };

    let mut items = Vec::new();
    for instance in &instances {
        let usage = app.manager.usage(instance);
        items.push(format!(
            "{{\"id\":{},\"memory_kb\":{},\"cpu_percent\":{}}}",
            instance.id, usage.memory_kb, usage.cpu_percent
        ));
    }

    web::json(200, format!("[{}]", items.join(",")))
}

/// The page with the sign in form.
fn login_page(app: &App, request: &WebRequest) -> Response {
    // A user who is already signed in has no use for the login form.
    if identity(app, request).is_some() {
        return web::redirect("/dashboard");
    }
    // A csrf token is issued in a cookie and repeated in the form, so the
    // form cannot be submitted from another site.
    let csrf = anonymous_csrf(app, request);
    let body = views::login_page(&csrf, None);
    web::html_with_cookie(
        200,
        body,
        auth::CSRF_COOKIE,
        &csrf,
        crate::config::SESSION_TTL_SECONDS,
        true,
    )
}

/// The sign in page with an error, which can be submitted again.
///
/// The token the browser already has is used, so the form and the cookie
/// agree and the user can simply press the button again instead of being
/// stuck with a form that cannot be submitted.
fn login_page_again(app: &App, request: &WebRequest, error: &str) -> Response {
    let csrf = anonymous_csrf(app, request);
    let body = views::login_page(&csrf, Some(error));
    web::html_with_cookie(
        200,
        body,
        auth::CSRF_COOKIE,
        &csrf,
        crate::config::SESSION_TTL_SECONDS,
        true,
    )
}

/// Handle the sign in form.
fn login_submit(app: &App, request: &WebRequest) -> Response {
    if !check_form_csrf(app, request) {
        return login_page_again(
            app,
            request,
            "The form was not submitted correctly. Please try again.",
        );
    }

    let username = request.trimmed_field("username");
    let password = request.field("password").unwrap_or_default().to_string();

    let stored = match app.db.password_hash(&username) {
        Ok(Some(hash)) => hash,
        _ => return login_page_again(app, request, "Wrong username or password."),
    };

    if !auth::verify_password(&password, &stored) {
        return login_page_again(app, request, "Wrong username or password.");
    }

    let user = match app.db.user_by_username(&username) {
        Ok(Some(user)) => user,
        _ => return login_page_again(app, request, "Wrong username or password."),
    };

    if !user.is_active {
        return login_page_again(app, request, "This account has been disabled.");
    }

    let session =
        match auth::create_session(&app.db, user.id, now(), crate::config::SESSION_TTL_SECONDS) {
            Ok(session) => session,
            Err(err) => {
                return web::html(
                    500,
                    views::error_page(None, "", 500, &format!("Cannot start a session: {err}")),
                )
            }
        };

    web::html_with_cookie(
        303,
        String::new(),
        SESSION_COOKIE,
        &session.token,
        crate::config::SESSION_TTL_SECONDS,
        true,
    )
    .with_header(tiny_http::Header::from_bytes(&b"Location"[..], &b"/dashboard"[..]).unwrap())
}

/// The page with the registration form.
fn signup_page(app: &App, request: &WebRequest) -> Response {
    if identity(app, request).is_some() {
        return web::redirect("/dashboard");
    }
    let csrf = anonymous_csrf(app, request);
    let body = views::signup_page(&csrf, None);
    web::html_with_cookie(
        200,
        body,
        auth::CSRF_COOKIE,
        &csrf,
        crate::config::SESSION_TTL_SECONDS,
        true,
    )
}

/// The registration page with an error, which can be submitted again.
///
/// The token the browser already has is used, so the form and the cookie
/// agree and the user can simply press the button again instead of being
/// stuck with a form that cannot be submitted.
fn signup_page_again(app: &App, request: &WebRequest, error: &str) -> Response {
    let csrf = anonymous_csrf(app, request);
    let body = views::signup_page(&csrf, Some(error));
    web::html_with_cookie(
        200,
        body,
        auth::CSRF_COOKIE,
        &csrf,
        crate::config::SESSION_TTL_SECONDS,
        true,
    )
}

/// Handle the registration form.
fn signup_submit(app: &App, request: &WebRequest) -> Response {
    if !check_form_csrf(app, request) {
        return signup_page_again(
            app,
            request,
            "The form was not submitted correctly. Please try again.",
        );
    }

    let username = request.trimmed_field("username");
    let email = request.trimmed_field("email");
    let password = request.field("password").unwrap_or_default().to_string();

    if let Err(message) = auth::validate_username(&username) {
        return signup_page_again(app, request, &message);
    }
    if let Err(message) = auth::validate_email(&email) {
        return signup_page_again(app, request, &message);
    }
    if let Err(message) = auth::validate_password(&password) {
        return signup_page_again(app, request, &message);
    }

    if let Ok(Some(_)) = app.db.user_by_username(&username) {
        return signup_page_again(app, request, "That username is already taken.");
    }
    if let Ok(Some(_)) = app.db.user_by_email(&email) {
        return signup_page_again(app, request, "That email address is already registered.");
    }

    // The very first account on a fresh panel becomes an administrator, so
    // that there is always a way into the admin area.
    let role = if app.config.first_user_is_admin && app.db.user_count().unwrap_or(0) == 0 {
        Role::Admin
    } else {
        Role::User
    };

    let hash = match auth::hash_password(&password) {
        Ok(hash) => hash,
        Err(err) => {
            return web::html(
                500,
                views::error_page(None, "", 500, &format!("Cannot hash the password: {err}")),
            )
        }
    };

    if let Err(err) = app.db.create_user(&username, &email, &hash, role, now()) {
        return web::html(
            500,
            views::error_page(None, "", 500, &format!("Cannot create the account: {err}")),
        );
    }

    web::html_with_cookie(303, String::new(), SESSION_COOKIE, "", 0, true).with_header(
        tiny_http::Header::from_bytes(&b"Location"[..], &b"/login?created=1"[..]).unwrap(),
    )
}

/// Handle the sign out form.
fn logout(app: &App, request: &WebRequest) -> Response {
    if let Some(identity) = identity(app, request) {
        if let Err(err) = check_csrf(&identity, request) {
            return err;
        }
    }
    if let Some(token) = request.cookie(SESSION_COOKIE) {
        let _ = app.db.delete_session(token);
    }
    web::html_with_cookie(303, String::new(), SESSION_COOKIE, "", 0, true)
        .with_header(tiny_http::Header::from_bytes(&b"Location"[..], &b"/login"[..]).unwrap())
}

/// The dashboard of a signed in user.
fn dashboard(app: &App, request: &WebRequest) -> Response {
    let identity = match require_identity(app, request) {
        Ok(identity) => identity,
        Err(response) => return response,
    };

    let instances = app
        .db
        .list_instances(Some(identity.user.id))
        .unwrap_or_default();
    let (memory, cpu, disk) = used_resources(app, &instances);

    // What the instances are using right now.
    let usage: Vec<Usage> = instances
        .iter()
        .map(|instance| app.manager.usage(instance))
        .collect();

    web::html(
        200,
        views::dashboard_page(
            &identity.user,
            &identity.csrf,
            &instances,
            &usage,
            memory,
            cpu,
            disk,
        ),
    )
}

/// The resources used by a set of instances.
fn used_resources(app: &App, instances: &[Instance]) -> (i64, i64, i64) {
    let mut memory = 0;
    let mut cpu = 0;
    let mut disk = 0;
    for instance in instances {
        memory += instance.memory_limit_mb;
        cpu += instance.cpu_limit_percent;
        disk += instance.disk_limit_mb;
        let _ = app;
    }
    (memory, cpu, disk)
}

/// The page for creating an instance.
fn new_instance_page(app: &App, request: &WebRequest) -> Response {
    // Only an administrator creates instances and hands them out.
    let identity = match require_admin(app, request) {
        Ok(identity) => identity,
        Err(response) => return response,
    };

    let users = app.db.all_users().unwrap_or_default();
    let instances = app
        .db
        .list_instances(Some(identity.user.id))
        .unwrap_or_default();
    let (memory, cpu, disk) = used_resources(app, &instances);

    web::html(
        200,
        views::new_instance_page(
            &identity.user,
            &identity.csrf,
            None,
            memory,
            cpu,
            disk,
            &users,
        ),
    )
}

/// Handle the create instance form.
fn create_instance(app: &App, request: &WebRequest) -> Response {
    // Only an administrator creates instances and hands them out.
    let identity = match require_admin(app, request) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    if let Err(response) = check_csrf(&identity, request) {
        return response;
    }

    let name = request.trimmed_field("name");
    if let Err(message) = auth::validate_instance_name(&name) {
        return render_new_instance(app, &identity, Some(&message));
    }

    // The preset decides what the instance runs.
    let preset = match Preset::from_slug(&request.trimmed_field("preset")) {
        Some(preset) => preset,
        None => return render_new_instance(app, &identity, Some("Please choose a preset.")),
    };

    let memory = request.number_field("memory").unwrap_or(0);
    let cpu = request.number_field("cpu").unwrap_or(0);
    let disk = request.number_field("disk").unwrap_or(0);

    if memory < 16 || cpu < 1 || disk < 16 {
        return render_new_instance(app, &identity, Some("Those resource limits are too small."));
    }
    // An instance uses at most one core, so it may not be given more than a
    // hundred percent.
    if cpu > MAX_INSTANCE_CPU_PERCENT {
        return render_new_instance(
            app,
            &identity,
            Some("An instance may use at most one core."),
        );
    }

    // The instance belongs to the account the administrator picks, and that
    // account has to have room for it.
    let owner = match resolve_owner(app, request) {
        Ok(owner) => owner,
        Err(message) => return render_new_instance(app, &identity, Some(&message)),
    };
    if let Err(message) = quota_allows(app, &owner, memory, cpu, disk, None) {
        return render_new_instance(app, &identity, Some(&message));
    }

    // Find a port that is not in use by an instance and not taken on the
    // network. The ports of every instance count, not only the ones of the
    // owner, because a port is used on the whole machine.
    let all = app.db.list_instances(None).unwrap_or_default();
    let used_ports: Vec<u16> = all.iter().map(|i| i.port).collect();
    let port = match (app.config.instance_ports.0..=app.config.instance_ports.1)
        .find(|port| !used_ports.contains(port))
    {
        Some(port) => port,
        None => {
            return render_new_instance(
                app,
                &identity,
                Some("No free port is available for a new instance."),
            )
        }
    };

    let id = match app.db.create_instance(
        &name,
        owner.id,
        port,
        memory,
        cpu,
        disk,
        preset.slug(),
        now(),
    ) {
        Ok(id) => id,
        Err(err) => {
            return render_new_instance(
                app,
                &identity,
                Some(&format!("Cannot create the instance: {err}")),
            )
        }
    };

    // A site starts with the files the user can change.
    if preset.starts_with_files() {
        if let Err(err) = site::seed(&app.config.data_dir, id) {
            return render_new_instance(
                app,
                &identity,
                Some(&format!("Cannot create the files of the site: {err}")),
            );
        }
    }

    // The page of the new instance is more useful than the dashboard,
    // because the instance belongs to somebody else.
    web::redirect(&format!("/instances/{id}"))
}

/// The account an instance is created for or handed to.
///
/// An administrator may pick any account, and the account has to be one
/// that can use the machine.
fn resolve_owner(app: &App, request: &WebRequest) -> std::result::Result<User, String> {
    let Some(id) = request.number_field("owner") else {
        return Err("Please choose the account the instance is for.".into());
    };
    match app.db.user_by_id(id) {
        Ok(Some(user)) if user.is_active => Ok(user),
        _ => Err("That account does not exist.".into()),
    }
}

/// Whether an account has room for an instance with these limits.
///
/// The instance that is being moved is left out of the count, so that an
/// instance can be handed to an account that already holds it.
fn quota_allows(
    app: &App,
    owner: &User,
    memory: i64,
    cpu: i64,
    disk: i64,
    skip: Option<i64>,
) -> std::result::Result<(), String> {
    let instances = app.db.list_instances(Some(owner.id)).unwrap_or_default();
    let instances: Vec<Instance> = instances
        .into_iter()
        .filter(|instance| Some(instance.id) != skip)
        .collect();

    if instances.len() as i64 >= owner.instance_limit {
        return Err("That account has reached the maximum number of instances.".into());
    }

    let (used_memory, used_cpu, used_disk) = used_resources(app, &instances);
    if used_memory + memory > owner.memory_limit_mb
        || used_cpu + cpu > owner.cpu_limit_percent
        || used_disk + disk > owner.disk_limit_mb
    {
        return Err("That account does not have enough resources left for this instance.".into());
    }

    Ok(())
}

/// Hand an instance to another account.
fn assign_instance(app: &App, request: &WebRequest, id: &str) -> Response {
    // Only an administrator hands instances out.
    let identity = match require_admin(app, request) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    if let Err(response) = check_csrf(&identity, request) {
        return response;
    }

    let id: i64 = match id.parse() {
        Ok(id) => id,
        Err(_) => return not_found(app, request),
    };
    let instance = match app.db.instance_by_id(id) {
        Ok(Some(instance)) => instance,
        _ => return not_found(app, request),
    };

    let owner = match resolve_owner(app, request) {
        Ok(owner) => owner,
        Err(message) => return render_instance_error(app, &identity, &instance, &message),
    };

    // The new owner has to have room for the instance.
    if let Err(message) = quota_allows(
        app,
        &owner,
        instance.memory_limit_mb,
        instance.cpu_limit_percent,
        instance.disk_limit_mb,
        Some(instance.id),
    ) {
        return render_instance_error(app, &identity, &instance, &message);
    }

    if let Err(err) = app.db.set_instance_owner(instance.id, owner.id) {
        return render_instance_error(
            app,
            &identity,
            &instance,
            &format!("Cannot hand over the instance: {err}"),
        );
    }

    web::redirect(&format!("/instances/{}", instance.id))
}

/// Render the create instance page with an error message.
fn render_new_instance(app: &App, identity: &Identity, error: Option<&str>) -> Response {
    let users = app.db.all_users().unwrap_or_default();
    let instances = app
        .db
        .list_instances(Some(identity.user.id))
        .unwrap_or_default();
    let (memory, cpu, disk) = used_resources(app, &instances);
    web::html(
        200,
        views::new_instance_page(
            &identity.user,
            &identity.csrf,
            error,
            memory,
            cpu,
            disk,
            &users,
        ),
    )
}

/// The detail page of an instance.
fn instance_detail(app: &App, request: &WebRequest, id: &str) -> Response {
    let identity = match require_identity(app, request) {
        Ok(identity) => identity,
        Err(response) => return response,
    };

    let id: i64 = match id.parse() {
        Ok(id) => id,
        Err(_) => return not_found(app, request),
    };
    let instance = match app.db.instance_by_id(id) {
        Ok(Some(instance)) => instance,
        _ => return not_found(app, request),
    };

    // A user may only look at their own instances, unless they are an
    // administrator.
    if instance.owner_id != identity.user.id && !identity.user.is_admin() {
        return web::html(
            403,
            views::error_page(
                Some(&identity.user),
                &identity.csrf,
                403,
                "That instance does not belong to you.",
            ),
        );
    }

    let usage = app.manager.usage(&instance);
    let disk = app.manager.disk_usage(&app.config.data_dir, &instance);

    let users = app.db.all_users().unwrap_or_default();
    web::html(
        200,
        views::instance_page(
            &identity.user,
            &instance,
            &identity.csrf,
            usage.memory_kb,
            usage.cpu_percent,
            disk,
            identity.user.is_admin(),
            &users,
        ),
    )
}

/// The editor of the site of an instance.
fn site_editor(app: &App, request: &WebRequest, id: &str) -> Response {
    let identity = match require_identity(app, request) {
        Ok(identity) => identity,
        Err(response) => return response,
    };

    let id: i64 = match id.parse() {
        Ok(id) => id,
        Err(_) => return not_found(app, request),
    };
    let instance = match app.db.instance_by_id(id) {
        Ok(Some(instance)) => instance,
        _ => return not_found(app, request),
    };

    // A user may only change their own instances, unless they are an
    // administrator.
    if instance.owner_id != identity.user.id && !identity.user.is_admin() {
        return web::html(
            403,
            views::error_page(
                Some(&identity.user),
                &identity.csrf,
                403,
                "That instance does not belong to you.",
            ),
        );
    }

    if !instance.preset.is_editable() {
        return web::html(
            403,
            views::error_page(
                Some(&identity.user),
                &identity.csrf,
                403,
                "This instance does not run a site.",
            ),
        );
    }

    let files = site::read_all(&app.config.data_dir, instance.id);
    web::html(
        200,
        views::site_editor_page(
            &identity.user,
            &instance,
            &identity.csrf,
            &files,
            None,
            false,
        ),
    )
}

/// Save one file of the site of an instance.
fn site_save(app: &App, request: &WebRequest, id: &str) -> Response {
    let identity = match require_identity(app, request) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    if let Err(response) = check_csrf(&identity, request) {
        return response;
    }

    let id: i64 = match id.parse() {
        Ok(id) => id,
        Err(_) => return not_found(app, request),
    };
    let instance = match app.db.instance_by_id(id) {
        Ok(Some(instance)) => instance,
        _ => return not_found(app, request),
    };

    if instance.owner_id != identity.user.id && !identity.user.is_admin() {
        return web::html(
            403,
            views::error_page(
                Some(&identity.user),
                &identity.csrf,
                403,
                "That instance does not belong to you.",
            ),
        );
    }

    if !instance.preset.is_editable() {
        return web::html(
            403,
            views::error_page(
                Some(&identity.user),
                &identity.csrf,
                403,
                "This instance does not run a site.",
            ),
        );
    }

    let name = request.trimmed_field("file");
    let content = request.field("content").unwrap_or_default().to_string();

    let error = site::write(&app.config.data_dir, instance.id, &name, &content).err();

    let files = site::read_all(&app.config.data_dir, instance.id);
    web::html(
        200,
        views::site_editor_page(
            &identity.user,
            &instance,
            &identity.csrf,
            &files,
            error.as_deref(),
            error.is_none(),
        ),
    )
}

/// The page of an administrator with every instance of the machine.
fn admin_instances(app: &App, request: &WebRequest) -> Response {
    let identity = match require_admin(app, request) {
        Ok(identity) => identity,
        Err(response) => return response,
    };

    let instances = app.db.list_instances(None).unwrap_or_default();
    let usage: Vec<Usage> = instances
        .iter()
        .map(|instance| app.manager.usage(instance))
        .collect();

    web::html(
        200,
        views::admin_instances_page(&identity.user, &identity.csrf, &instances, &usage),
    )
}

/// Start an instance.
fn start_instance(app: &App, request: &WebRequest, id: &str) -> Response {
    let identity = match require_identity(app, request) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    if let Err(response) = check_csrf(&identity, request) {
        return response;
    }

    let id: i64 = match id.parse() {
        Ok(id) => id,
        Err(_) => return not_found(app, request),
    };
    let instance = match app.db.instance_by_id(id) {
        Ok(Some(instance)) => instance,
        _ => return not_found(app, request),
    };
    if instance.owner_id != identity.user.id && !identity.user.is_admin() {
        return forbidden(&identity);
    }

    if let Err(message) = app.manager.start(&app.db, &instance, &app.config.data_dir) {
        return render_instance_error(app, &identity, &instance, &message);
    }

    web::redirect(&format!("/instances/{}", instance.id))
}

/// Stop an instance.
fn stop_instance(app: &App, request: &WebRequest, id: &str) -> Response {
    let identity = match require_identity(app, request) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    if let Err(response) = check_csrf(&identity, request) {
        return response;
    }

    let id: i64 = match id.parse() {
        Ok(id) => id,
        Err(_) => return not_found(app, request),
    };
    let instance = match app.db.instance_by_id(id) {
        Ok(Some(instance)) => instance,
        _ => return not_found(app, request),
    };
    if instance.owner_id != identity.user.id && !identity.user.is_admin() {
        return forbidden(&identity);
    }

    if let Err(message) = app.manager.stop(&app.db, &instance) {
        return render_instance_error(app, &identity, &instance, &message);
    }

    web::redirect(&format!("/instances/{}", instance.id))
}

/// Delete an instance.
fn delete_instance(app: &App, request: &WebRequest, id: &str) -> Response {
    let identity = match require_identity(app, request) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    if let Err(response) = check_csrf(&identity, request) {
        return response;
    }

    let id: i64 = match id.parse() {
        Ok(id) => id,
        Err(_) => return not_found(app, request),
    };
    let instance = match app.db.instance_by_id(id) {
        Ok(Some(instance)) => instance,
        _ => return not_found(app, request),
    };
    if instance.owner_id != identity.user.id && !identity.user.is_admin() {
        return forbidden(&identity);
    }

    let _ = app.manager.stop(&app.db, &instance);
    let _ = app.db.delete_instance(instance.id);
    let _ = std::fs::remove_dir_all(
        app.config
            .data_dir
            .join(format!("instance-{}", instance.id)),
    );

    if identity.user.is_admin() && instance.owner_id != identity.user.id {
        web::redirect("/admin")
    } else {
        web::redirect("/dashboard")
    }
}

/// The admin overview.
fn admin_page(app: &App, request: &WebRequest) -> Response {
    let identity = match require_admin(app, request) {
        Ok(identity) => identity,
        Err(response) => return response,
    };

    let users = app.db.all_users().unwrap_or_default();
    let instances = app.db.list_instances(None).unwrap_or_default();

    web::html(
        200,
        views::admin_page(&identity.user, &identity.csrf, &users, &instances),
    )
}

/// The admin page of a single account.
fn admin_user_page(app: &App, request: &WebRequest, id: &str) -> Response {
    let identity = match require_admin(app, request) {
        Ok(identity) => identity,
        Err(response) => return response,
    };

    let id: i64 = match id.parse() {
        Ok(id) => id,
        Err(_) => return not_found(app, request),
    };
    let user = match app.db.user_by_id(id) {
        Ok(Some(user)) => user,
        _ => return not_found(app, request),
    };
    let instances = app.db.list_instances(Some(id)).unwrap_or_default();

    web::html(
        200,
        views::admin_user_page(&identity.user, &user, &instances, &identity.csrf, None),
    )
}

/// Promote or demote an account.
fn admin_set_role(app: &App, request: &WebRequest, id: &str) -> Response {
    let identity = match require_admin(app, request) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    if let Err(response) = check_csrf(&identity, request) {
        return response;
    }

    let id: i64 = match id.parse() {
        Ok(id) => id,
        Err(_) => return not_found(app, request),
    };
    let user = match app.db.user_by_id(id) {
        Ok(Some(user)) => user,
        _ => return not_found(app, request),
    };

    // The last administrator must not be able to demote themselves, or the
    // panel could end up without anyone able to administer it.
    let new_role = if user.is_admin() {
        Role::User
    } else {
        Role::Admin
    };
    if user.is_admin() && new_role == Role::User && admin_count(app) <= 1 {
        return render_admin_user(
            app,
            &identity,
            &user,
            "There must be at least one administrator.",
        );
    }

    let _ = app.db.set_role(user.id, new_role);
    render_admin_user(app, &identity, &user, "The role was changed.")
}

/// Enable or disable an account.
fn admin_set_active(app: &App, request: &WebRequest, id: &str) -> Response {
    let identity = match require_admin(app, request) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    if let Err(response) = check_csrf(&identity, request) {
        return response;
    }

    let id: i64 = match id.parse() {
        Ok(id) => id,
        Err(_) => return not_found(app, request),
    };
    let user = match app.db.user_by_id(id) {
        Ok(Some(user)) => user,
        _ => return not_found(app, request),
    };

    // An administrator cannot disable their own account, otherwise they
    // would lock themselves out.
    if user.id == identity.user.id && user.is_active {
        return render_admin_user(
            app,
            &identity,
            &user,
            "You cannot disable your own account.",
        );
    }

    let _ = app.db.set_active(user.id, !user.is_active);
    render_admin_user(app, &identity, &user, "The account was updated.")
}

/// Change the resource limits of an account.
fn admin_set_limits(app: &App, request: &WebRequest, id: &str) -> Response {
    let identity = match require_admin(app, request) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    if let Err(response) = check_csrf(&identity, request) {
        return response;
    }

    let id: i64 = match id.parse() {
        Ok(id) => id,
        Err(_) => return not_found(app, request),
    };
    let user = match app.db.user_by_id(id) {
        Ok(Some(user)) => user,
        _ => return not_found(app, request),
    };

    let memory = request.number_field("memory").unwrap_or(0);
    let cpu = request.number_field("cpu").unwrap_or(0);
    let disk = request.number_field("disk").unwrap_or(0);
    let max_instances = request.number_field("instances").unwrap_or(0);

    if memory < 16 || cpu < 1 || disk < 16 || max_instances < 0 {
        return render_admin_user(app, &identity, &user, "Those limits are not valid.");
    }
    // An account may hold more than a whole processor, because it may have
    // more than one instance, but not more than the machine has cores.
    if cpu > MAX_ACCOUNT_CPU_PERCENT {
        return render_admin_user(
            app,
            &identity,
            &user,
            "That is more processor than the machine has.",
        );
    }

    let _ = app.db.set_limits(user.id, memory, cpu, disk, max_instances);
    render_admin_user(app, &identity, &user, "The limits were saved.")
}

/// Set the password of an account.
fn admin_set_password(app: &App, request: &WebRequest, id: &str) -> Response {
    let identity = match require_admin(app, request) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    if let Err(response) = check_csrf(&identity, request) {
        return response;
    }

    let id: i64 = match id.parse() {
        Ok(id) => id,
        Err(_) => return not_found(app, request),
    };
    let user = match app.db.user_by_id(id) {
        Ok(Some(user)) => user,
        _ => return not_found(app, request),
    };

    let password = request.field("password").unwrap_or_default().to_string();
    if let Err(message) = auth::validate_password(&password) {
        return render_admin_user(app, &identity, &user, &message);
    }

    let hash = match auth::hash_password(&password) {
        Ok(hash) => hash,
        Err(err) => {
            return render_admin_user(
                app,
                &identity,
                &user,
                &format!("Cannot hash the password: {err}"),
            )
        }
    };
    let _ = app.db.set_password(user.id, &hash);
    render_admin_user(app, &identity, &user, "The password was changed.")
}

/// The number of administrators on the panel.
fn admin_count(app: &App) -> usize {
    app.db
        .all_users()
        .unwrap_or_default()
        .iter()
        .filter(|user| user.is_admin())
        .count()
}

/// Render the admin page of an account with a notice.
fn render_admin_user(app: &App, identity: &Identity, user: &User, notice: &str) -> Response {
    let fresh = app
        .db
        .user_by_id(user.id)
        .ok()
        .flatten()
        .unwrap_or_else(|| user.clone());
    let instances = app.db.list_instances(Some(fresh.id)).unwrap_or_default();
    web::html(
        200,
        views::admin_user_page(
            &identity.user,
            &fresh,
            &instances,
            &identity.csrf,
            Some(notice),
        ),
    )
}

/// Render an error on the instance page.
fn render_instance_error(
    app: &App,
    identity: &Identity,
    instance: &Instance,
    message: &str,
) -> Response {
    let usage = app.manager.usage(instance);
    let disk = app.manager.disk_usage(&app.config.data_dir, instance);
    let users = app.db.all_users().unwrap_or_default();
    let mut html = views::instance_page(
        &identity.user,
        instance,
        &identity.csrf,
        usage.memory_kb,
        usage.cpu_percent,
        disk,
        identity.user.is_admin(),
        &users,
    );
    // The error is shown above the page content.
    let notice = format!(
        r#"<div class="notice error">{}</div>"#,
        web::escape(message)
    );
    html = html.replace("<main>\n", &format!("<main>\n{notice}\n"));
    web::html(200, html)
}

/// A forbidden response.
fn forbidden(identity: &Identity) -> Response {
    web::html(
        403,
        views::error_page(
            Some(&identity.user),
            &identity.csrf,
            403,
            "You do not have permission to do that.",
        ),
    )
}

/// The response for an unknown page.
fn not_found(app: &App, request: &WebRequest) -> Response {
    match identity(app, request) {
        Some(identity) => web::html(
            404,
            views::error_page(
                Some(&identity.user),
                &identity.csrf,
                404,
                "That page does not exist.",
            ),
        ),
        None => web::html(
            404,
            views::error_page(None, "", 404, "That page does not exist."),
        ),
    }
}
