CREATE TABLE email_jobs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    kind text NOT NULL,
    to_email text NOT NULL,
    payload jsonb NOT NULL,
    priority smallint NOT NULL DEFAULT 5,
    status text NOT NULL DEFAULT 'pending',
    attempts int NOT NULL DEFAULT 0,
    next_attempt_at timestamptz NOT NULL DEFAULT now(),
    last_error text,
    dedupe_key text UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now(),
    sent_at timestamptz
);
CREATE INDEX email_jobs_due ON email_jobs (priority, next_attempt_at) WHERE status = 'pending';