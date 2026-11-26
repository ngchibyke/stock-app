//! What has to be finished before an account can be deleted. Codes only: the app turns
//! each into plain words.

use sqlx::PgConnection;
use uuid::Uuid;

use crate::error::AppError;

pub const OPEN_ORDER: &str = "open_order";
pub const OPEN_DISPUTE: &str = "open_dispute";
pub const OWNS_STORE: &str = "owns_store";
pub const MONEY_IN_FLIGHT: &str = "money_in_flight";

/// The blockers for `user`, in a fixed order. Empty means the account may be deleted.
///
/// A buyer's blockers are their own orders, disputes and refunds. A store owner's also
/// cover the store's orders, disputes and payouts, and the store itself. A team member
/// who is not the owner has none of the store's: they leave freely.
pub async fn blockers(conn: &mut PgConnection, user: Uuid) -> Result<Vec<&'static str>, AppError> {
    let (order, dispute, store, money): (bool, bool, bool, bool) = sqlx::query_as(
        "SELECT \
            EXISTS (SELECT 1 FROM orders o JOIN stores s ON s.id = o.store_id \
                    WHERE (o.buyer_id = $1 OR s.owner_id = $1) \
                      AND o.status IN ('pending_payment', 'paid', 'picked_up', \
                                       'out_for_delivery', 'delivered', 'refund_pending')), \
            EXISTS (SELECT 1 FROM disputes d JOIN stores s ON s.id = d.store_id \
                    WHERE (d.buyer_id = $1 OR s.owner_id = $1) \
                      AND d.status IN ('awaiting_seller', 'awaiting_return', 'awaiting_decision')), \
            EXISTS (SELECT 1 FROM stores WHERE owner_id = $1 AND deleted_at IS NULL), \
            EXISTS (SELECT 1 FROM payouts p JOIN orders o ON o.id = p.order_id \
                    JOIN stores s ON s.id = o.store_id \
                    WHERE s.owner_id = $1 \
                      AND p.status IN ('scheduled', 'sending', 'pending', 'unknown')) \
         OR EXISTS (SELECT 1 FROM refunds r JOIN orders o ON o.id = r.order_id \
                    WHERE o.buyer_id = $1 AND r.status IN ('requested', 'processing'))",
    )
    .bind(user)
    .fetch_one(conn)
    .await
    .map_err(anyhow::Error::from)?;
    let mut out = Vec::new();
    if order {
        out.push(OPEN_ORDER);
    }
    if dispute {
        out.push(OPEN_DISPUTE);
    }
    if store {
        out.push(OWNS_STORE);
    }
    if money {
        out.push(MONEY_IN_FLIGHT);
    }
    Ok(out)
}
