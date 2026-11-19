-- Acknowledgement of reconciliation alerts (plan 6). An alert is open until a row exists for
-- its key. Keys: `audit:<audit_log.id>`, `order:<order id>:<reason>`,
-- `shipment:<shipment id or courier label>:<status>:<attention>` and
-- `handover:<order id>:<newest lockout audit id>`. Append-only, like the audit log.
CREATE TABLE alert_acks (
    alert_key text PRIMARY KEY CHECK (length(alert_key) BETWEEN 3 AND 200),
    acked_by  uuid NOT NULL REFERENCES users (id),
    acked_at  timestamptz NOT NULL DEFAULT now(),
    note      text NOT NULL CHECK (length(note) BETWEEN 1 AND 500)
);

CREATE FUNCTION alert_acks_reject_change() RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'alert_acks is append-only (% not allowed)', TG_OP;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER alert_acks_no_update BEFORE UPDATE ON alert_acks
    FOR EACH ROW EXECUTE FUNCTION alert_acks_reject_change();
CREATE TRIGGER alert_acks_no_delete BEFORE DELETE ON alert_acks
    FOR EACH ROW EXECUTE FUNCTION alert_acks_reject_change();
CREATE TRIGGER alert_acks_no_truncate BEFORE TRUNCATE ON alert_acks
    FOR EACH STATEMENT EXECUTE FUNCTION alert_acks_reject_change();
