//! `require` reads the owner and the live member rows on every call.

use server::{
    error::AppError,
    stores::access::{Action, Role, covers_branch_sql, is_member_sql, require},
};
use sqlx::PgPool;
use uuid::Uuid;

async fn user(pool: &PgPool, n: u8) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO users (phone_enc, phone_idx, display_name, password_hash) \
         VALUES (decode('01', 'hex'), decode($1, 'hex'), 'U', 'x') RETURNING id",
    )
    .bind(format!("{n:02x}"))
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn store(pool: &PgPool, owner: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO stores (owner_id, name, slug, pickup_address_enc, pickup_state, \
            delivery_courier, delivery_self, status) \
         VALUES ($1, 'Shop', 'shop', decode('aabb', 'hex'), 'FCT', true, true, 'active') \
         RETURNING id",
    )
    .bind(owner)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn add_member(pool: &PgPool, store: Uuid, user: Uuid, role: &str, by: Uuid) {
    sqlx::query(
        "INSERT INTO store_members (store_id, user_id, role, added_by) \
         VALUES ($1, $2, $3::store_member_role, $4)",
    )
    .bind(store)
    .bind(user)
    .bind(role)
    .bind(by)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test]
async fn require_404_for_stranger_and_removed_member(pool: PgPool) {
    let owner = user(&pool, 1).await;
    let stranger = user(&pool, 2).await;
    let former = user(&pool, 3).await;
    let store = store(&pool, owner).await;
    add_member(&pool, store, former, "manager", owner).await;
    sqlx::query("UPDATE store_members SET removed_at = now(), removed_by = $1 WHERE user_id = $2")
        .bind(owner)
        .bind(former)
        .execute(&pool)
        .await
        .unwrap();

    let mut conn = pool.acquire().await.unwrap();
    for who in [stranger, former] {
        let err = require(&mut conn, store, who, Action::ViewStore)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound), "{err:?}");
    }
    // The owner is in, and a deleted store is gone even for the owner.
    let m = require(&mut conn, store, owner, Action::DeleteStore)
        .await
        .unwrap();
    assert_eq!(m.role, Role::Owner);
    sqlx::query("UPDATE stores SET deleted_at = now()")
        .execute(&pool)
        .await
        .unwrap();
    let err = require(&mut conn, store, owner, Action::ViewStore)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound), "{err:?}");
}

#[sqlx::test]
async fn require_403_for_staff_on_manager_action(pool: PgPool) {
    let owner = user(&pool, 1).await;
    let staff = user(&pool, 2).await;
    let manager = user(&pool, 3).await;
    let store = store(&pool, owner).await;
    add_member(&pool, store, staff, "staff", owner).await;
    add_member(&pool, store, manager, "manager", owner).await;

    let mut conn = pool.acquire().await.unwrap();
    let err = require(&mut conn, store, staff, Action::PublishProducts)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Forbidden("not_allowed")), "{err:?}");
    assert!(
        require(&mut conn, store, staff, Action::EditProducts)
            .await
            .is_ok()
    );
    assert!(
        require(&mut conn, store, manager, Action::PublishProducts)
            .await
            .is_ok()
    );
    let err = require(&mut conn, store, manager, Action::ManageTeam)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Forbidden("not_allowed")), "{err:?}");
}

#[sqlx::test]
async fn removal_takes_effect_on_next_call(pool: PgPool) {
    let owner = user(&pool, 1).await;
    let staff = user(&pool, 2).await;
    let store = store(&pool, owner).await;
    add_member(&pool, store, staff, "staff", owner).await;

    let mut conn = pool.acquire().await.unwrap();
    require(&mut conn, store, staff, Action::ViewStore)
        .await
        .unwrap();
    sqlx::query("UPDATE store_members SET removed_at = now(), removed_by = $1")
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    let err = require(&mut conn, store, staff, Action::ViewStore)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound), "{err:?}");
}

#[sqlx::test]
async fn is_member_sql_matches_owner_and_live_members_only(pool: PgPool) {
    let owner = user(&pool, 1).await;
    let staff = user(&pool, 2).await;
    let stranger = user(&pool, 3).await;
    let store = store(&pool, owner).await;
    add_member(&pool, store, staff, "staff", owner).await;

    let sql = format!(
        "SELECT s.id FROM stores s WHERE s.id = $1 AND {}",
        is_member_sql("$2")
    );
    for (who, expected) in [(owner, true), (staff, true), (stranger, false)] {
        let found: Option<Uuid> = sqlx::query_scalar(&sql)
            .bind(store)
            .bind(who)
            .fetch_optional(&pool)
            .await
            .unwrap();
        assert_eq!(found.is_some(), expected);
    }
}

#[sqlx::test]
async fn covers_branch_sql_follows_the_staff_tie(pool: PgPool) {
    let owner = user(&pool, 1).await;
    let manager = user(&pool, 2).await;
    let wide = user(&pool, 3).await;
    let tied = user(&pool, 4).await;
    let store = store(&pool, owner).await;
    add_member(&pool, store, manager, "manager", owner).await;
    add_member(&pool, store, wide, "staff", owner).await;
    let (mine, other): (Uuid, Uuid) = (Uuid::new_v4(), Uuid::new_v4());
    sqlx::query("INSERT INTO store_branches (id, store_id, name, pickup_address_enc, position) VALUES ($1, $2, 'A', decode('aa', 'hex'), 1)")
        .bind(mine)
        .bind(store)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO store_members (store_id, user_id, role, branch_id, added_by) \
         VALUES ($1, $2, 'staff', $3, $4)",
    )
    .bind(store)
    .bind(tied)
    .bind(mine)
    .bind(owner)
    .execute(&pool)
    .await
    .unwrap();

    let sql = format!(
        "SELECT s.id FROM stores s WHERE s.id = $1 AND {}",
        covers_branch_sql("$2", "$3")
    );
    for (who, branch, expected) in [
        (owner, other, true),
        (manager, other, true),
        (wide, other, true),
        (tied, mine, true),
        (tied, other, false),
    ] {
        let found: Option<Uuid> = sqlx::query_scalar(&sql)
            .bind(store)
            .bind(who)
            .bind(branch)
            .fetch_optional(&pool)
            .await
            .unwrap();
        assert_eq!(found.is_some(), expected);
    }
}
