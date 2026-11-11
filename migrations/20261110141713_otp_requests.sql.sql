CREATE TABLE otp_requests (
    id bigserial PRIMARY KEY,
    email text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX otp_requests_email_time ON otp_requests (email, created_at DESC);