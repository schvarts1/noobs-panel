# The plan for noobs-panel

The plan for the panel, the instances and the safety and the speed of both.
Every phase is one commit with its own tests, so the panel keeps working and
every step can be taken back on its own.

## The limits of the machine

The panel runs on a phone, in Termux, as a normal application. That decides
what is possible and what is not:

- The panel is **not root**. `unshare` fails with `EINVAL`, so there are no
  user namespaces, and `/sys/fs/cgroup` cannot be written to. There are no
  containers and no cgroup limits.
- An instance is therefore only held back by `setrlimit` and by the panel
  itself. It runs with the same rights as the panel.
- The machine has eight cores, and the panel is meant to be usable from a
  phone browser.
- `tiny_http` runs one handler at a time, and a `rusqlite` connection is
  `Send` but not `Sync`.

The rule that follows from this: **the panel may not run a piece of user
code that the panel itself would not be willing to run.** Anything that
executes user code is left to an administrator, and says so in the
interface.

## Phase 0 — Stop lying to the user

Bugs that show something else than what is true.

| # | Change | Where | Why it matters |
|---|--------|-------|----------------|
| 0.1 | The watcher writes `Crashed` and the real exit code, and reaps the child | `panel/src/instances.rs`, `panel/src/db.rs` | A dead instance shows **Running** today, the exit code is never written, and the children stay as zombies |
| 0.2 | An oversized body is refused with `413`, and every route has its own limit | `panel/src/web.rs`, `panel/src/site.rs` | The site editor **silently truncates** past about 48 KB, because the body cap is 64 KB and the file limit is 256 KB. The user is told the file was saved. |
| 0.3 | A free port is looked for on the machine when an instance is created, and the port is checked again after the fallback | `panel/src/app.rs`, `panel/src/instances.rs` | `find_free_port` exists but is only used by the tests, so a fallback port can run into `UNIQUE(port)` and the start fails |
| 0.4 | On startup an orphan is taken over or killed, instead of losing its number | `panel/src/instances.rs` | A living orphan gets `pid = NULL`, so it can never be stopped from the interface |
| 0.5 | `used_resources` is renamed to `allocated_resources`, and `Usage::disk_kb` is filled in | `panel/src/app.rs`, `panel/src/instances.rs` | The name says usage, the sum is of limits |

## Phase 1 — The panel must never block

The speed of the panel, and the safety that comes with it. **This phase is
done.**

1. **A pool of workers** instead of one request at a time. The pool has a
   queue with a limit, and answers `503` when it is full, so a flood of
   requests cannot eat all the memory of the phone.
2. **The database is shared safely.** The connection goes behind a mutex and
   the database is handed out with `Arc`, so the workers can use it.
3. **No handler waits.** `usage()` measured the processor by sleeping for a
   quarter of a second inside the request, while the dashboard asks for it
   every two seconds. The watcher already knows the numbers, so the handler
   only reads them.
4. **The queries are done by the database.** The list of instances was
   fetched whole and filtered in Rust, so the index on the owner was never
   used. The number of administrators was counted by loading every account.
5. **The database is set up for more than one thread.** `busy_timeout` and
   `synchronous` are set, and the statements that are used often are kept
   ready.
6. **The size of the directory is measured by the watcher**, every few
   seconds, instead of by walking the whole tree every time the page of an
   instance is opened.
7. **The port of an instance is read from the end of the log**, instead of
   reading the whole file every fifty milliseconds.
8. **A janitor thread** throws away the sessions and the csrf tokens that
   have expired. Both tables grow until the panel is restarted today.
9. **A panic in a handler no longer takes the panel down.** The worker
   catches it, answers `500`, and goes on with the next request.
10. **The files of a site are sent with a fingerprint.** The server sends an
    `ETag` and a `Last-Modified`, answers `304` when nothing changed, and
    asks the browser to check again. An edit is still seen at once.

## Phase 2 — Account safety

| # | Change | Why |
|---|--------|-----|
| 2.1 | **Rate limiting** on `/login`, `/signup` and the api, per address and per account, with `429` and `Retry-After` | There is nothing at all against guessing a password today, and that is the biggest hole if the panel is ever reachable from the internet |
| 2.2 | A new password ends the other sessions of that account, and there is a "sign out everywhere" | The sessions live on for seven days after a password is changed |
| 2.3 | A sign in always costs the same time, also when the account does not exist | The answer comes back much faster for an account that does not exist, so the accounts can be listed |
| 2.4 | The session ends when it is idle, and the cookie gets the `Secure` mark under https | The session lives seven days, whether it is used or not |
| 2.5 | The security headers: a policy with a mark per page, `nosniff`, `Referrer-Policy`, `frame-ancestors` | There is not one header today |
| 2.6 | A journal of what the administrators did | There is no trace of who lowered a limit or disabled an account |
| 2.7 | A configuration for a proxy in front, and https in the panel itself | It is not decided yet how the panel will be reached, so all of it has to fit |
| 2.8 | *Optional:* a password that can be reset by mail | There is no way back into an account that lost its password |

## Phase 3 — Looking after the instances

- **Start again after a crash**, with a growing wait between the tries and a
  limit on the number of tries.
- **The disk quota is a total, not a file.** `RLIMIT_FSIZE` is per file, so
  an instance can write any number of files. The watcher measures the
  directory and stops the instance when it is too big. The log of the
  instance is turned over instead of growing for ever.
- **The health of an instance is checked**, on its own address, and an
  instance that does not answer is started again.
- **The console of an instance.** The child keeps a pipe on its input, so the
  panel can send it a command and show what it answers. A `java -jar` server
  needs this.
- **The panel listens for the signals.** On `SIGINT` and `SIGTERM` the
  children are stopped, and a child that is paused is let run again. Today a
  panel that dies between `SIGSTOP` and `SIGCONT` leaves a child frozen for
  ever.
- **Every instance has its own environment** and its own directory, and can
  be restarted gently instead of stopped and started.

## Phase 4 — Code of the user, and only an administrator may allow it

1. A new preset **custom**: an administrator gives an account the right to
   run its own code, and the account uploads a program or a jar.
2. Every instance has **its own command line and environment**, and they
   are kept in the database.
3. The program is put in the directory of the instance with the mark `0500`,
   and the sum of its bytes is written down.
4. A program may not be the panel itself, and may not write outside the
   directory of the instance.
5. **A warning that never goes away**: a program of a user runs with the
   rights of the panel, because nothing on this machine can hold it back.
   The administrator is the one who answers for it.

## Phase 5 — What the panel can do

1. **The log of an instance in the panel**, with the tail of it, a page that
   follows it, and a way to fetch it as a file.
2. **The history of the use.** The watcher writes a sample every few
   seconds, so the dashboard can show a line, and the administrator can see
   which account uses the most.
3. **The account page of the user**: the profile, a new password, the
   sessions, and a way to end them.
4. **The life of an account**, with the `PENDING_TTL_SECONDS` that is in the
   code but never used: an account that waits, and a code to invite people.
5. **The files of a site**, more than the three there are now, with upload
   and delete.
6. **Copies**: a snapshot of the directory of an instance, and a way to put
   it back.
7. **Keys for the api**, so that a script can start and stop instances
   without a session.
8. **A layout that fits a phone**, because that is where the panel will be
   used.
9. **What the administrator wants to know**: the use per account, the
   biggest eaters, and the crashes.

## The risks that are not fixed

- **Nothing can be held apart on this machine.** No root, no user
  namespaces, no writable cgroups. A program of a user can read the
  database of the panel with the sessions and the password hashes in it.
  Phase 4 decides who may do that. It does not stop them.
- **The limit on the processor stays an estimate.** The panel pauses the
  child in pieces of a quarter of a second, so a burst of about two seconds
  is allowed.
- **The https in the panel is possible but heavy.** A proxy in front of it
  is the better way, and the configuration is ready for both.
