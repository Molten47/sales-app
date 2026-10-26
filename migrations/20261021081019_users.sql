CREATE TABLE users (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email      TEXT NOT NULL UNIQUE CHECK (email = lower(email)),
    name       TEXT NOT NULL,
    role       TEXT NOT NULL DEFAULT 'customer' CHECK (role IN ('customer', 'staff')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE orders
    ADD CONSTRAINT orders_customer_fk
    FOREIGN KEY (customer_id) REFERENCES users(id) NOT VALID;