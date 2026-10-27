ALTER TABLE notifications ADD COLUMN event_id UUID;

-- One notification per (user, event). Old rows have NULL event_id,
-- and NULLs never conflict in Postgres, so existing data is fine.
CREATE UNIQUE INDEX notifications_user_event_uq
    ON notifications (user_id, event_id);