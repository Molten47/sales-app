# sales-app

A Rust backend for order intake with **authenticated, real-time, persisted notifications** and **transactional email**. When an order is placed, the customer and every staff user are notified over WebSocket within tens of milliseconds, anyone who was offline still gets the notification the next time they ask for it, and the customer receives an email.

Users sign in with a one-time code (OTP) sent by email and receive a JWT. Customers can only place and read their own orders; staff can read all of them and mark orders delivered.

Built with Axum, SQLx, PostgreSQL and Resend.

## Why it's built this way

| Problem | Decision |
|---|---|
| Don't lose events if the app crashes between "order saved" and "notification sent" | **Transactional outbox**: the order and its event are written in the same DB transaction |
| Don't poll the database every few seconds | Postgres **LISTEN/NOTIFY** wakes the poller instantly, with a 30 s polling fallback as a safety net |
| Offline users must still see what they missed | Notifications are **persisted first, pushed second** |
| A crash can redeliver an event | A unique `(user_id, event_id)` constraint makes notification inserts **idempotent** |
| One slow WebSocket client must not stall everyone | Hub uses non-blocking `try_send`; slow or closed clients are dropped |
| No passwords to store or leak | **OTP login**: short-lived, single-use, attempt-limited codes; only a keyed hash is stored |
| A client must never choose who it is | The user id and role come from the **verified JWT**, never from the request body or URL |
| Don't lose or duplicate emails, and don't hit provider limits | **Email queue in Postgres**: jobs are written in the same transaction as the change, claimed with `SKIP LOCKED`, retried with backoff, and deduplicated |
| Endpoints that send email can be abused | **Per-email rate limit** on `request-otp`, with an identical response either way |

## Architecture

```
POST /api/v1/orders  (Bearer token -> customer id)
        |
        v
  orders + outbox_events + email_jobs     (one transaction)
        |  NOTIFY outbox_new                        |
        v                                           v
  outbox poller                              email worker
  (LISTEN, FOR UPDATE SKIP LOCKED,           (SKIP LOCKED, priority order,
   retry with backoff)                        retry with backoff, Resend)
        |  mpsc channel
        v
  router (workers)
        |-- 1. INSERT notifications  (customer + all staff, one statement, idempotent)
        '-- 2. push over WebSocket   (hub, keyed by user id)

POST /api/v1/orders/{id}/deliver  (staff) -> orders + outbox_events + email_jobs

GET  /api/v1/notifications          <- offline users catch up here
POST /api/v1/notifications/{id}/read
GET  /ws/connect?token=<jwt>        <- live delivery
```

Because step 1 happens before step 2, `delivered` can be lower than `recipients` in the log without anything being lost.

## Authentication

```
POST /auth/request-otp  { email }        -> 202 always (never reveals whether the email exists)
POST /auth/verify-otp   { email, code }  -> { token, token_type, expires_in, user_id, role }
```

- Codes are 6 digits, valid for **5 minutes**, **single use**, and locked after **5 wrong attempts** (request a new code to unlock).
- Only an HMAC-SHA256 of the code, keyed with `JWT_SECRET`, is stored. The attempt is counted atomically *before* the comparison, so parallel guesses can't exceed the limit.
- Every failure returns the same `401`, so responses reveal nothing about which step failed.
- `request-otp` allows **5 requests per email per hour**. Over the limit it still answers `202` and sends nothing.
- Tokens are HS256 JWTs valid for **8 hours**, carrying `sub` (user id), `role` and `exp`.
- Send the token as `Authorization: Bearer <token>` on REST calls, and as `?token=<jwt>` on the WebSocket (browsers can't set headers on a WebSocket).
- **Delivery:** the code is emailed directly through Resend (not queued, so it arrives in seconds). With `EMAIL_PROVIDER=log` it is printed to the server log instead. In `resend` mode codes are never logged. The email is sent in the background so response time doesn't reveal whether the address exists.

### Roles

| Role | Can do |
|---|---|
| anonymous | sign up as a customer, request and verify OTPs |
| customer | place orders, read **own** orders, read and mark **own** notifications |
| staff | everything a customer can, plus read **any** order, mark orders delivered, receive every order's notification and the daily digest, and create other staff accounts |

Public signup always creates a customer. Asking for `role: "staff"` there returns `403`. Staff accounts can only be created by an existing staff user via `POST /staff`. The very first staff user is seeded once with SQL (see below).

## Email

Emails are sent through [Resend](https://resend.com) over its HTTP API.

| Kind | Delivery | Priority |
|---|---|---|
| OTP login code | Sent directly, never queued | n/a |
| Order created | Queued | 3 |
| Order delivered | Queued | 3 |
| Daily digest (staff) | Queued | 8 |

### Email queue

Order and digest emails go through an `email_jobs` table in Postgres.

- **Atomic:** the job is inserted in the same transaction as the order or status change, so an email is never lost or sent for a rolled-back change.
- **Worker:** claims one due job at a time with `FOR UPDATE SKIP LOCKED`, so several instances never double-send. Jobs are ordered by priority, so analytics never delays order mail. A 2-minute lease means a crashed worker's job becomes due again on its own.
- **Retries:** exponential backoff from 30 s, up to 6 attempts, then `dead`.
- **Permanent errors:** 4xx responses from Resend (except 408 and 429) go straight to `dead`.
- **Idempotency:** each job has a `dedupe_key` (a repeated key is ignored) and its id is sent to Resend as an `Idempotency-Key`.
- **Rate:** about 1.6 sends per second, under Resend's 2 per second limit.

Inspect the queue:

```sql
SELECT kind, to_email, status, attempts, last_error
FROM email_jobs ORDER BY created_at DESC;
```

### Daily digest

Once the UTC hour reaches 07:00 (08:00 in Lagos), the worker queues a summary of the previous full UTC day for every `staff` user: orders and revenue, one line per currency. The dedupe key `digest:{date}:{email}` guarantees one digest per staff member per day, even across restarts.

## Getting started

### Prerequisites

- Rust (stable)
- PostgreSQL (a Docker container works well)
- [`sqlx-cli`](https://crates.io/crates/sqlx-cli): `cargo install sqlx-cli --no-default-features --features postgres,rustls`
- A [Resend](https://resend.com) API key (only for `EMAIL_PROVIDER=resend`)

### Configure

Create a `.env` file in the project root (it is git-ignored, never commit it):

```env
DATABASE_URL=postgres://user:password@localhost:5432/sales_app
PORT=8081
JWT_SECRET=<long random string>

# log (default) prints emails and OTP codes to the server log
EMAIL_PROVIDER=log
# required when EMAIL_PROVIDER=resend
# RESEND_API_KEY=re_...
# EMAIL_FROM=Sales App <onboarding@resend.dev>
```

| Variable | Required | Notes |
|---|---|---|
| `DATABASE_URL` | yes | Postgres connection string |
| `PORT` | no | Defaults to `8080` |
| `JWT_SECRET` | yes | Signs tokens and keys the OTP hash |
| `EMAIL_PROVIDER` | no | `log` (default) or `resend` |
| `RESEND_API_KEY` | if `resend` | Starts with `re_` |
| `EMAIL_FROM` | if `resend` | Sender, e.g. `Sales App <onboarding@resend.dev>` |

Generate a secret in PowerShell:

```powershell
$b = New-Object byte[] 48; [System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($b); [Convert]::ToBase64String($b)
```

Changing `JWT_SECRET` invalidates all issued tokens and any outstanding OTPs. `.env` is only read at startup, so restart after editing it.

### Run

```powershell
cargo run
```

Migrations are applied automatically at startup. To apply or inspect them manually:

```powershell
sqlx migrate run
sqlx migrate info
```

### Seed the first staff user

There is no staff user on a fresh database, and staff can only be created by staff, so seed one directly (replace the placeholders with your container's values):

```powershell
docker exec <container> psql -U <db-user> -d <db-name> -c "INSERT INTO users (email, name, role) VALUES ('admin@example.com', 'Admin', 'staff') ON CONFLICT (email) DO NOTHING;"
```

Then sign in as that user (below) and create further staff through `POST /api/v1/staff`.

### Checks

```powershell
cargo fmt --all
cargo clippy --all-targets -- -D warnings
```

## API

Base URL: `http://localhost:8081`

| Method | Path | Auth | Description |
|---|---|---|---|
| GET | `/health` | none | Liveness |
| GET | `/health/ready` | none | Readiness (checks the database) |
| POST | `/api/v1/users` | none | Sign up (always a customer) |
| POST | `/api/v1/staff` | staff | Create a staff account |
| POST | `/api/v1/auth/request-otp` | none | Request a login code (rate limited) |
| POST | `/api/v1/auth/verify-otp` | none | Exchange a code for a JWT |
| POST | `/api/v1/orders` | any user | Create an order for the logged-in user, queues the "order received" email |
| GET | `/api/v1/orders/{id}` | owner or staff | Fetch an order (404 if it isn't yours) |
| POST | `/api/v1/orders/{id}/deliver` | staff | Mark delivered, queues the "order delivered" email. `404` if missing or already delivered |
| GET | `/api/v1/notifications` | any user | Unread notifications, newest first (max 100) |
| POST | `/api/v1/notifications/{id}/read` | any user | Mark one of your notifications read (204) |
| GET | `/ws/connect?token=<jwt>` | token in query | WebSocket for live notifications |

Error responses are JSON: `{"error": "..."}`. Statuses used: `400` invalid input, `401` missing or invalid credentials, `403` not allowed for your role, `404` not found (also used for other people's resources, so ids can't be probed), `409` duplicate email.

### Sign up

```powershell
Invoke-RestMethod -Method Post -Uri http://localhost:8081/api/v1/users -ContentType "application/json" `
  -Body (@{ email = "ada@example.com"; name = "Ada" } | ConvertTo-Json)
```

Emails are lowercased. A `role` of `staff` returns `403`.

### Sign in

```powershell
# 1. request a code (emailed, or printed in the server log in log mode)
Invoke-RestMethod -Method Post -Uri http://localhost:8081/api/v1/auth/request-otp -ContentType "application/json" `
  -Body (@{ email = "ada@example.com" } | ConvertTo-Json)

# 2. exchange it for a token (keep the code a quoted string: codes can start with 0)
$login = Invoke-RestMethod -Method Post -Uri http://localhost:8081/api/v1/auth/verify-otp -ContentType "application/json" `
  -Body (@{ email = "ada@example.com"; code = "123456" } | ConvertTo-Json)
$h = @{ Authorization = "Bearer $($login.token)" }
```

### Create, read and deliver orders

```powershell
$order = Invoke-RestMethod -Method Post -Uri http://localhost:8081/api/v1/orders -Headers $h -ContentType "application/json" `
  -Body (@{ total_minor = 500000 } | ConvertTo-Json)

Invoke-RestMethod "http://localhost:8081/api/v1/orders/$($order.id)" -Headers $h

# staff only
Invoke-RestMethod -Method Post "http://localhost:8081/api/v1/orders/$($order.id)/deliver" -Headers $staffHeaders
```

- `total_minor` is in minor units (500000 = 5,000.00) and must be greater than zero.
- `currency` is optional: a 3-letter uppercase code, defaulting to `NGN`.
- There is no `customer_id` field. The order always belongs to the token's user, and an unexpected `customer_id` in the body is ignored.
- Delivering twice returns `404` and sends no second email. A customer calling it gets `403`.

### Create a staff account (staff only)

```powershell
Invoke-RestMethod -Method Post -Uri http://localhost:8081/api/v1/staff -Headers $staffHeaders -ContentType "application/json" `
  -Body (@{ email = "grace@example.com"; name = "Grace" } | ConvertTo-Json)
```

### Notifications

```powershell
# unread for the logged-in user
Invoke-RestMethod http://localhost:8081/api/v1/notifications -Headers $h

# mark one read (idempotent; 404 if it doesn't exist or belongs to someone else)
Invoke-RestMethod -Method Post "http://localhost:8081/api/v1/notifications/<id>/read" -Headers $h
```

### Live delivery over WebSocket

Connect to `ws://localhost:8081/ws/connect?token=<jwt>`. A missing or invalid token is rejected with `401` before the upgrade. Each order produces a message like:

```json
{
  "type": "order.created",
  "event_id": "fab8cba4-e31e-4f0a-83c8-420c5348ec71",
  "notification_id": "d3d1ba7a-fe1e-4b8b-81a2-798bf7e6c210",
  "data": {
    "order_id": "27ba3135-94de-41d6-ac56-cf52257948c9",
    "customer_id": "2cc84625-2567-4d10-9443-576181b379e8",
    "total_minor": 500000,
    "currency": "NGN"
  }
}
```

A client can pass `notification_id` straight to the `/read` endpoint to mark a toast as read.

## Try the full flow

`dev.ps1` defines small PowerShell helpers (`Login`, `Status`) for manual testing. Load it with a leading dot:

```powershell
. .\dev.ps1

$cust  = Login "ada@example.com"      # prompts for the code (emailed, or in the server log in log mode)
$staff = Login "admin@example.com"
$h  = @{ Authorization = "Bearer $($cust.token)" }
$sh = @{ Authorization = "Bearer $($staff.token)" }

# the customer places an order; the customer and all staff are notified
$o = Invoke-RestMethod -Method Post -Uri "$base/api/v1/orders" -Headers $h -ContentType $json -Body (@{ total_minor = 500000 } | ConvertTo-Json)

# staff marks it delivered
Invoke-RestMethod -Method Post -Uri "$base/api/v1/orders/$($o.id)/deliver" -Headers $sh

# no WebSocket connected: both still see unread notifications
Invoke-RestMethod "$base/api/v1/notifications" -Headers $h
Invoke-RestMethod "$base/api/v1/notifications" -Headers $sh

# security checks, each prints a status code
Status { Invoke-RestMethod "$base/api/v1/notifications" }                                   # 401 no token
Status { Invoke-RestMethod -Method Post -Uri "$base/api/v1/staff" -Headers $h -ContentType $json -Body (@{ email="x@example.com"; name="X" } | ConvertTo-Json) }  # 403 customer can't create staff
Status { Invoke-RestMethod -Method Post -Uri "$base/api/v1/orders/$($o.id)/deliver" -Headers $h }  # 403 customer can't deliver
```

The log line for that order reads `order routed recipients=2 delivered=0`. Connect a WebSocket with the customer's token first and it reads `delivered=1`. In `log` mode the worker also prints each email (`EMAIL to ... | Order xxxx received`).

## Performance

Measured on a local dev machine:

- Order created to routed over the hub: about **30 ms** (previously about 4.8 s with polling only).
- Order created to notification rows persisted: about **100 ms**, including the database write.

## Design notes and known limitations

- **Delivery is at-least-once, notifications are idempotent.** If the app crashes after the router handles an event but before the poller commits, the event is redelivered. The unique `(user_id, event_id)` constraint with `ON CONFLICT DO NOTHING` absorbs the duplicate, so no extra rows are created. A redelivered event is not pushed live a second time; the persisted notification is still available from `GET /api/v1/notifications`. Rows created before this constraint have a `NULL` `event_id`.
- **Transient insert failures are retried** in the router (4 attempts with backoff), because the poller marks an event published as soon as it is handed over.
- **Outbox retries** use exponential backoff (2 s, 4 s, 8 s, capped at 5 min) and mark an event `failed` after 8 attempts.
- **Resend's default sender is a sandbox.** `onboarding@resend.dev` only delivers to the Resend account's own email. Other recipients fail with `422` and the job goes `dead` until a domain is verified.
- **A failed email never changes an API response.** `request-otp` always answers `202`; failures are logged and the stored code simply expires. The free plan also has daily send caps, so check Resend's current limits.
- **OTP rate limiting is per email only,** not per IP, and `POST /users` has no limit. Old `otp_requests` rows are never cleaned up.
- **The digest hour is fixed** at 07:00 UTC.
- **Bounces and complaints are not tracked** (no Resend webhook yet).
- **Tokens last 8 hours** with no refresh and no revocation, and the role is baked into the token, so a role change takes effect at the next login.
- **The WebSocket token is checked at connect time only**, and it travels in the query string, so it can end up in server or proxy logs. An open socket is not closed when its token expires.
- **Customers only order for themselves.** Staff placing orders on a customer's behalf is not supported yet.
- **Single instance for WebSockets.** The hub is in-memory, so running several app instances would need a shared pub/sub layer. The email queue is already safe across instances.

## Project layout

```
src/
  main.rs            app wiring, routes, startup
  config.rs          environment config (database, JWT, email provider)
  auth.rs            JWT issue/verify, OTP hashing, AuthUser and StaffUser extractors
  email.rs           Mailer (Resend or log), Email enum and templates
  api/               HTTP handlers (auth, users, orders, notifications, errors)
  domain/            queries and types (users, orders, notifications, otp, email_jobs)
  outbox/            outbox event type and LISTEN/NOTIFY poller
  workers/           event router (persist, then push), email worker, daily digest
  ws/                WebSocket handler and per-user hub
migrations/          SQLx migrations
dev.ps1              PowerShell helpers for manual testing
```

## Roadmap

- [x] Orders with transactional outbox
- [x] LISTEN/NOTIFY wake-up for the poller
- [x] Users table and create-user endpoint
- [x] Persisted in-app notifications
- [x] Idempotent notifications (`event_id` + unique constraint)
- [x] OTP login with JWT
- [x] Authenticated orders and notifications (customer taken from the token)
- [x] Staff accounts creatable only by staff
- [x] Rate limiting on `request-otp`
- [x] Real OTP delivery (Resend)
- [x] Email queue: order created, order delivered, daily digest
- [ ] CORS layer
- [ ] Integration tests and CI
- [ ] Rate limiting on signup and per IP
- [ ] Verified sending domain and bounce webhook
- [ ] Deployment (Neon, Render)
- [ ] Next.js frontend