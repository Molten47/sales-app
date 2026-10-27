# sales-app

A Rust backend for order intake with **real-time, persisted notifications**. When an order is placed, the customer and every staff user are notified over WebSocket within tens of milliseconds, and anyone who was offline still gets the notification the next time they ask for it.

Built with Axum, SQLx and PostgreSQL.

## Why it's built this way

| Problem | Decision |
|---|---|
| Don't lose events if the app crashes between "order saved" and "notification sent" | **Transactional outbox**: the order and its event are written in the same DB transaction |
| Don't poll the database every few seconds | Postgres **LISTEN/NOTIFY** wakes the poller instantly, with a 30 s polling fallback as a safety net |
| Offline users must still see what they missed | Notifications are **persisted first, pushed second** |
| One slow WebSocket client must not stall everyone | Hub uses non-blocking `try_send`; slow or closed clients are dropped |

## Architecture

```
POST /api/v1/orders
        |
        v
  orders + outbox_events          (one transaction)
        |  NOTIFY outbox_new
        v
  outbox poller                   (LISTEN, FOR UPDATE SKIP LOCKED, retry with backoff)
        |  mpsc channel
        v
  router (workers)
        |-- 1. INSERT notifications  (customer + all staff, one statement)
        '-- 2. push over WebSocket   (hub, keyed by user id)

GET  /api/v1/notifications          <- offline users catch up here
POST /api/v1/notifications/{id}/read
```

Because step 1 happens before step 2, `delivered` can be lower than `recipients` in the log without anything being lost.

## Getting started

### Prerequisites

- Rust (stable)
- PostgreSQL
- [`sqlx-cli`](https://crates.io/crates/sqlx-cli): `cargo install sqlx-cli --no-default-features --features postgres,rustls`

### Configure

Create a `.env` file in the project root:

```env
DATABASE_URL=postgres://user:password@localhost:5432/sales_app
PORT=8081
```

### Run

```powershell
cargo run
```

Migrations are applied automatically at startup. To apply or inspect them manually:

```powershell
sqlx migrate run
sqlx migrate info
```

### Checks

```powershell
cargo fmt --all
cargo clippy --all-targets -- -D warnings
```

## API

Base URL: `http://localhost:8081`

| Method | Path | Description |
|---|---|---|
| GET | `/health` | Liveness |
| GET | `/health/ready` | Readiness (checks the database) |
| POST | `/api/v1/users` | Create a user |
| POST | `/api/v1/orders` | Create an order |
| GET | `/api/v1/notifications?user_id=<uuid>` | Unread notifications, newest first (max 100) |
| POST | `/api/v1/notifications/{id}/read?user_id=<uuid>` | Mark one notification read (204) |
| GET | `/ws/connect?user_id=<uuid>` | WebSocket for live notifications |

### Create a user

```powershell
Invoke-RestMethod -Method Post -Uri http://localhost:8081/api/v1/users -ContentType "application/json" `
  -Body (@{ email = "ada@example.com"; name = "Ada"; role = "staff" } | ConvertTo-Json)
```

- `role` is `customer` (default) or `staff`.
- Emails are lowercased.
- Duplicate email returns **409**; an invalid role returns **400**.

### Create an order

```powershell
Invoke-RestMethod -Method Post -Uri http://localhost:8081/api/v1/orders -ContentType "application/json" `
  -Body (@{ customer_id = "<customer-uuid>"; total_minor = 500000 } | ConvertTo-Json)
```

- `total_minor` is in minor units (500000 = 5,000.00 NGN).
- An unknown `customer_id` returns **400**.

### Notifications

```powershell
# unread for a user
Invoke-RestMethod "http://localhost:8081/api/v1/notifications?user_id=<uuid>"

# mark one read (idempotent; 404 if it doesn't exist or belongs to someone else)
Invoke-RestMethod -Method Post "http://localhost:8081/api/v1/notifications/<id>/read?user_id=<uuid>"
```

### Live delivery over WebSocket

Connect to `ws://localhost:8081/ws/connect?user_id=<uuid>`. Each order produces a message like:

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

```powershell
$staff = Invoke-RestMethod -Method Post -Uri http://localhost:8081/api/v1/users -ContentType "application/json" -Body (@{ email="staff@example.com"; name="Staff"; role="staff" } | ConvertTo-Json)
$cust  = Invoke-RestMethod -Method Post -Uri http://localhost:8081/api/v1/users -ContentType "application/json" -Body (@{ email="cust@example.com"; name="Cust" } | ConvertTo-Json)

Invoke-RestMethod -Method Post -Uri http://localhost:8081/api/v1/orders -ContentType "application/json" -Body (@{ customer_id=$cust.id; total_minor=500000 } | ConvertTo-Json)

# no WebSocket connected: both users still see one unread notification
Invoke-RestMethod "http://localhost:8081/api/v1/notifications?user_id=$($staff.id)"
Invoke-RestMethod "http://localhost:8081/api/v1/notifications?user_id=$($cust.id)"
```

The log line for that order reads `order routed recipients=2 delivered=0`. Connect a WebSocket as the customer first and it reads `delivered=1`.

## Performance

Measured on a local dev machine:

- Order created to routed over the hub: about **30 ms** (previously about 4.8 s with polling only).
- Order created to notification rows persisted: about **100 ms**, including the database write.

## Design notes and known limitations

- **Auth is a later brick.** `user_id` is taken from the query string on the REST endpoints and on `/ws/connect`, so anyone can read anyone's notifications. This is a deliberate stopgap, to be replaced by OTP login with JWT.
- **Delivery is at-least-once.** If the app crashes after the router handles an event but before the poller commits, the event is redelivered and can create duplicate notification rows. The planned fix is an `event_id` column with a unique `(user_id, event_id)` constraint and `ON CONFLICT DO NOTHING`. Clients can also dedupe on `event_id`.
- **Transient insert failures are retried** in the router (4 attempts with backoff), because the poller marks an event published as soon as it is handed over.
- **Outbox retries** use exponential backoff (2 s, 4 s, 8 s, capped at 5 min) and mark an event `failed` after 8 attempts.
- **Single instance.** The WebSocket hub is in-memory, so running several app instances would need a shared pub/sub layer.

## Project layout

```
src/
  main.rs            app wiring, routes, startup
  config.rs          environment config
  api/               HTTP handlers (users, orders, notifications, errors)
  domain/            queries and types (users, orders, notifications)
  outbox/            outbox event type and LISTEN/NOTIFY poller
  workers/           event router: persist, then push
  ws/                WebSocket handler and per-user hub
migrations/          SQLx migrations
```

## Roadmap

- [x] Orders with transactional outbox
- [x] LISTEN/NOTIFY wake-up for the poller
- [x] Users table and create-user endpoint
- [x] Persisted in-app notifications
- [ ] Idempotent notifications (`event_id` + unique constraint)
- [ ] OTP login with JWT, replacing the `user_id` scaffolding