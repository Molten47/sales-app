CREATE TABLE orders (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_id   UUID NOT NULL,
    status        TEXT NOT NULL DEFAULT 'pending'
                  CHECK (status IN ('pending','paid','dispatched','delivered','cancelled')),
    total_minor   BIGINT NOT NULL CHECK (total_minor >= 0),  -- kobo, never floats
    currency      CHAR(3) NOT NULL DEFAULT 'NGN',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE outbox_events (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    aggregate_type TEXT NOT NULL,
    aggregate_id   UUID NOT NULL,
    event_type     TEXT NOT NULL,
    payload        JSONB NOT NULL,
    status         TEXT NOT NULL DEFAULT 'pending'
                   CHECK (status IN ('pending','published','failed')),
    attempts       INT NOT NULL DEFAULT 0,
    last_error     TEXT,
    available_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    published_at   TIMESTAMPTZ
);

CREATE INDEX idx_outbox_pending
    ON outbox_events (available_at, created_at)
    WHERE status = 'pending';