-- Store team invariants (plan 7e-1, task 4).
-- Personal data: none new. A deleted account's live memberships end here, in the
-- same statement that deletes the account, so every deletion path ends them; the
-- audit entry holds ids only.

ALTER TABLE store_members
    ADD CONSTRAINT store_members_removed_together
        CHECK ((removed_at IS NULL) = (removed_by IS NULL));

ALTER TABLE store_invites
    ADD CONSTRAINT store_invites_one_ending
        CHECK (num_nonnulls(accepted_at, declined_at, revoked_at) <= 1),
    ADD CONSTRAINT store_invites_expires_after_created
        CHECK (expires_at > created_at);

-- A user's deletion ends their live memberships (removed by themselves) and nothing
-- else: invites to their phone stay with the store that sent them.
CREATE FUNCTION users_deleted_end_memberships() RETURNS trigger AS $$
BEGIN
    WITH ended AS (
        UPDATE store_members SET removed_at = now(), removed_by = NEW.id
        WHERE user_id = NEW.id AND removed_at IS NULL
        RETURNING store_id
    )
    INSERT INTO audit_log (actor_id, action, subject_type, subject_id, detail)
    SELECT NEW.id, 'store.member_removed', 'store', store_id,
           jsonb_build_object('user_id', NEW.id, 'reason', 'account_deleted')
    FROM ended;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER users_deleted_end_memberships
    AFTER UPDATE OF deleted_at ON users
    FOR EACH ROW WHEN (OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL)
    EXECUTE FUNCTION users_deleted_end_memberships();
