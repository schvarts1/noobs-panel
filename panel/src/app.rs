//! The panel application: routing, request handling and session handling.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::auth::{self, SESSION_COOKIE};
use crate::config::Config;
use crate::db::Database;
use crate::instances::InstanceManager;
use crate::models::{Instance, Role, User};
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
        ("GET", ["instances", "new"]) => new_instance_page(app, request),
        ("POST", ["instances", "new"]) => create_instance(app, request),
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

/// The page with the sign in form.
fn login_page(app: &App, request: &WebRequest) -> Response {
    // A user who is already signed in has no use for the login form.
    if identity(app, request).is_some() {
        return web::redirect("/dashboard");
    }
    // A csrf token is issued in a cookie and repeated in the form, so the
    // form cannot be submitted from another site.
    let csrf = auth::generate_csrf_token();
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

/// Check the csrf token of a form that was shown before signing in.
fn check_form_csrf(request: &WebRequest) -> bool {
    let expected = request.cookie(auth::CSRF_COOKIE).unwrap_or_default();
    let provided = request.trimmed_field("_csrf");
    auth::csrf_matches(expected, &provided)
}

/// Handle the sign in form.
fn login_submit(app: &App, request: &WebRequest) -> Response {
    if !check_form_csrf(request) {
        return web::html(
            200,
            views::login_page(
                "",
                Some("The form was not submitted correctly. Please try again."),
            ),
        );
    }

    let username = request.trimmed_field("username");
    let password = request.field("password").unwrap_or_default().to_string();

    let stored = match app.db.password_hash(&username) {
        Ok(Some(hash)) => hash,
        _ => {
            return web::html(
                200,
                views::login_page("", Some("Wrong username or password.")),
            )
        }
    };

    if !auth::verify_password(&password, &stored) {
        return web::html(
            200,
            views::login_page("", Some("Wrong username or password.")),
        );
    }

    let user = match app.db.user_by_username(&username) {
        Ok(Some(user)) => user,
        _ => {
            return web::html(
                200,
                views::login_page("", Some("Wrong username or password.")),
            )
        }
    };

    if !user.is_active {
        return web::html(
            200,
            views::login_page("", Some("This account has been disabled.")),
        );
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
    let csrf = auth::generate_csrf_token();
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

/// Handle the registration form.
fn signup_submit(app: &App, request: &WebRequest) -> Response {
    if !check_form_csrf(request) {
        return web::html(
            200,
            views::signup_page(
                "",
                Some("The form was not submitted correctly. Please try again."),
            ),
        );
    }

    let username = request.trimmed_field("username");
    let email = request.trimmed_field("email");
    let password = request.field("password").unwrap_or_default().to_string();

    if let Err(message) = auth::validate_username(&username) {
        return web::html(200, views::signup_page("", Some(&message)));
    }
    if let Err(message) = auth::validate_email(&email) {
        return web::html(200, views::signup_page("", Some(&message)));
    }
    if let Err(message) = auth::validate_password(&password) {
        return web::html(200, views::signup_page("", Some(&message)));
    }

    if let Ok(Some(_)) = app.db.user_by_username(&username) {
        return web::html(
            200,
            views::signup_page("", Some("That username is already taken.")),
        );
    }
    if let Ok(Some(_)) = app.db.user_by_email(&email) {
        return web::html(
            200,
            views::signup_page("", Some("That email address is already registered.")),
        );
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

    web::html(
        200,
        views::dashboard_page(
            &identity.user,
            &identity.csrf,
            &instances,
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
        cpu += instance.cpu_limit_seconds;
        disk += instance.disk_limit_mb;
        let _ = app;
    }
    (memory, cpu, disk)
}

/// The page for creating an instance.
fn new_instance_page(app: &App, request: &WebRequest) -> Response {
    let identity = match require_identity(app, request) {
        Ok(identity) => identity,
        Err(response) => return response,
    };

    let instances = app
        .db
        .list_instances(Some(identity.user.id))
        .unwrap_or_default();
    let (memory, cpu, disk) = used_resources(app, &instances);

    web::html(
        200,
        views::new_instance_page(&identity.user, &identity.csrf, None, memory, cpu, disk),
    )
}

/// Handle the create instance form.
fn create_instance(app: &App, request: &WebRequest) -> Response {
    let identity = match require_identity(app, request) {
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

    let memory = request.number_field("memory").unwrap_or(0);
    let cpu = request.number_field("cpu").unwrap_or(0);
    let disk = request.number_field("disk").unwrap_or(0);

    let user = &identity.user;

    // The requested limits must be positive and fit inside the quota of the
    // account.
    if memory < 16 || cpu < 1 || disk < 16 {
        return render_new_instance(app, &identity, Some("Those resource limits are too small."));
    }
    if memory > user.memory_limit_mb || cpu > user.cpu_limit_seconds || disk > user.disk_limit_mb {
        return render_new_instance(
            app,
            &identity,
            Some("Those limits are larger than the resources your account has."),
        );
    }

    let instances = app.db.list_instances(Some(user.id)).unwrap_or_default();
    if instances.len() as i64 >= user.instance_limit {
        return render_new_instance(
            app,
            &identity,
            Some("You have reached the maximum number of instances."),
        );
    }

    let (used_memory, used_cpu, used_disk) = used_resources(app, &instances);
    if used_memory + memory > user.memory_limit_mb
        || used_cpu + cpu > user.cpu_limit_seconds
        || used_disk + disk > user.disk_limit_mb
    {
        return render_new_instance(
            app,
            &identity,
            Some("You do not have enough resources left for that instance."),
        );
    }

    // Find a port that is not in use by an instance and not taken on the
    // network.
    let used_ports: Vec<u16> = instances.iter().map(|i| i.port).collect();
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

    if let Err(err) = app
        .db
        .create_instance(&name, user.id, port, memory, cpu, disk, now())
    {
        return render_new_instance(
            app,
            &identity,
            Some(&format!("Cannot create the instance: {err}")),
        );
    }

    web::redirect("/dashboard")
}

/// Render the create instance page with an error message.
fn render_new_instance(app: &App, identity: &Identity, error: Option<&str>) -> Response {
    let instances = app
        .db
        .list_instances(Some(identity.user.id))
        .unwrap_or_default();
    let (memory, cpu, disk) = used_resources(app, &instances);
    web::html(
        200,
        views::new_instance_page(&identity.user, &identity.csrf, error, memory, cpu, disk),
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

    web::html(
        200,
        views::instance_page(
            &identity.user,
            &instance,
            &identity.csrf,
            usage.memory_kb,
            usage.cpu_seconds,
            disk,
            identity.user.is_admin(),
        ),
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
    let mut html = views::instance_page(
        &identity.user,
        instance,
        &identity.csrf,
        usage.memory_kb,
        usage.cpu_seconds,
        disk,
        identity.user.is_admin(),
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
