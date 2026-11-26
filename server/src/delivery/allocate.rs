//! Which branch(es) of a store fill a buyer's cart (part 7e-1, task 7).
//!
//! A cart line is never split across branches. When one branch can fill every line,
//! up to three such branches are returned to be quoted. Otherwise the lines are
//! grouped greedily into at most three branches, and lines no group can fill whole
//! are reported as unavailable.

use std::collections::HashMap;

use anyhow::Context;
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{delivery::area::AreaCouncil, error::AppError, stock};

/// Most branches quoted, and most groups made, per store.
pub const MAX_GROUPS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub variant_id: Uuid,
    pub quantity: i32,
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub branch_id: Uuid,
    /// `None` when the branch has no area council set; it never ranks as the buyer's.
    pub area: Option<AreaCouncil>,
    pub position: i32,
    /// Units available per variant; a missing variant means none.
    pub available: HashMap<Uuid, i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupPlan {
    pub branch_id: Uuid,
    pub lines: Vec<Line>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Allocation {
    /// Full-fill case: up to 3 branches to quote, best first; caller picks by fee then date.
    pub full_fill: Vec<Uuid>,
    /// Split case (full_fill empty): up to 3 groups.
    pub groups: Vec<GroupPlan>,
    /// Lines no chosen group can fill whole, with the full line quantity.
    pub unavailable: Vec<Line>,
}

fn fills(c: &Candidate, l: &Line) -> bool {
    c.available.get(&l.variant_id).copied().unwrap_or(0) >= l.quantity
}

/// Same council as the buyer first, then lower position.
fn rank(c: &Candidate, buyer: AreaCouncil) -> (u8, i32) {
    (u8::from(c.area != Some(buyer)), c.position)
}

pub fn allocate(lines: &[Line], candidates: &[Candidate], buyer_area: AreaCouncil) -> Allocation {
    let mut ranked: Vec<&Candidate> = candidates.iter().collect();
    ranked.sort_by_key(|c| rank(c, buyer_area));
    if lines.is_empty() {
        return Allocation::default();
    }

    let full_fill: Vec<Uuid> = ranked
        .iter()
        .filter(|c| lines.iter().all(|l| fills(c, l)))
        .take(MAX_GROUPS)
        .map(|c| c.branch_id)
        .collect();
    if !full_fill.is_empty() {
        return Allocation {
            full_fill,
            ..Allocation::default()
        };
    }

    let mut remaining: Vec<Line> = lines.to_vec();
    let mut groups = Vec::new();
    let mut used: Vec<Uuid> = Vec::new();
    while groups.len() < MAX_GROUPS && !remaining.is_empty() {
        // `ranked` is in tie-break order and `max_by_key` keeps the last maximum,
        // so scan it reversed to keep the best-ranked of equals.
        let best = ranked
            .iter()
            .rev()
            .filter(|c| !used.contains(&c.branch_id))
            .map(|c| (remaining.iter().filter(|l| fills(c, l)).count(), *c))
            .filter(|(n, _)| *n > 0)
            .max_by_key(|(n, _)| *n);
        let Some((_, c)) = best else { break };
        used.push(c.branch_id);
        let (take, keep): (Vec<Line>, Vec<Line>) = remaining.into_iter().partition(|l| fills(c, l));
        remaining = keep;
        groups.push(GroupPlan {
            branch_id: c.branch_id,
            lines: take,
        });
    }
    Allocation {
        full_fill: Vec::new(),
        groups,
        unavailable: remaining,
    }
}

/// SHA-256 (lowercase hex) of `"<branch uuid>"` followed by one `"\n<variant uuid>:<quantity>"`
/// per line, lines sorted by variant id, so the order of `lines` does not matter.
pub fn group_fingerprint(branch: Uuid, lines: &[Line]) -> String {
    let mut sorted: Vec<&Line> = lines.iter().collect();
    sorted.sort_by_key(|l| l.variant_id);
    let mut text = branch.to_string();
    for l in sorted {
        text.push_str(&format!("\n{}:{}", l.variant_id, l.quantity));
    }
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The store's open branches with what each can sell of `variants`. Branches of a
/// deleted store are never returned.
pub async fn load_candidates(
    conn: &mut PgConnection,
    store: Uuid,
    variants: &[Uuid],
) -> Result<Vec<Candidate>, AppError> {
    let branches: Vec<(Uuid, Option<AreaCouncil>, i32)> = sqlx::query_as(
        "SELECT b.id, b.pickup_area_council, b.position FROM store_branches b \
         JOIN stores s ON s.id = b.store_id \
         WHERE b.store_id = $1 AND b.closed_at IS NULL AND s.deleted_at IS NULL \
         ORDER BY b.position",
    )
    .bind(store)
    .fetch_all(&mut *conn)
    .await
    .context("load candidate branches")?;
    let sql = format!(
        "SELECT bs.branch_id, bs.variant_id, {} FROM branch_stock bs \
         JOIN store_branches b ON b.id = bs.branch_id \
         WHERE b.store_id = $1 AND b.closed_at IS NULL AND bs.variant_id = ANY($2)",
        stock::BRANCH_AVAILABLE_SQL
    );
    let rows: Vec<(Uuid, Uuid, i32)> = sqlx::query_as(&sql)
        .bind(store)
        .bind(variants)
        .fetch_all(&mut *conn)
        .await
        .context("load candidate stock")?;
    Ok(branches
        .into_iter()
        .map(|(branch_id, area, position)| Candidate {
            branch_id,
            area,
            position,
            available: rows
                .iter()
                .filter(|r| r.0 == branch_id)
                .map(|r| (r.1, r.2.max(0)))
                .collect(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }
    fn line(v: u128, q: i32) -> Line {
        Line {
            variant_id: id(v),
            quantity: q,
        }
    }
    fn cand(b: u128, area: AreaCouncil, position: i32, avail: &[(u128, i32)]) -> Candidate {
        Candidate {
            branch_id: id(b),
            area: Some(area),
            position,
            available: avail.iter().map(|(v, q)| (id(*v), *q)).collect(),
        }
    }
    const A: AreaCouncil = AreaCouncil::Amac;
    const B: AreaCouncil = AreaCouncil::Bwari;

    #[test]
    fn one_branch_fills_all() {
        let r = allocate(
            &[line(1, 2), line(2, 1)],
            &[cand(10, A, 0, &[(1, 5), (2, 1)])],
            A,
        );
        assert_eq!(r.full_fill, vec![id(10)]);
        assert!(r.groups.is_empty() && r.unavailable.is_empty());
    }

    #[test]
    fn same_council_ranked_first() {
        let c = [
            cand(10, B, 0, &[(1, 5)]),
            cand(11, A, 1, &[(1, 5)]),
            cand(12, A, 2, &[(1, 5)]),
        ];
        let r = allocate(&[line(1, 1)], &c, A);
        assert_eq!(r.full_fill, vec![id(11), id(12), id(10)]);
    }

    #[test]
    fn at_most_three_full_fill_candidates() {
        let c: Vec<Candidate> = (0..5)
            .map(|i| cand(10 + i, A, i as i32, &[(1, 5)]))
            .collect();
        let r = allocate(&[line(1, 1)], &c, A);
        assert_eq!(r.full_fill, vec![id(10), id(11), id(12)]);
    }

    #[test]
    fn split_across_two() {
        let c = [cand(10, A, 0, &[(1, 5)]), cand(11, A, 1, &[(2, 5)])];
        let r = allocate(&[line(1, 1), line(2, 1)], &c, A);
        assert!(r.full_fill.is_empty() && r.unavailable.is_empty());
        assert_eq!(
            r.groups,
            vec![
                GroupPlan {
                    branch_id: id(10),
                    lines: vec![line(1, 1)]
                },
                GroupPlan {
                    branch_id: id(11),
                    lines: vec![line(2, 1)]
                },
            ]
        );
    }

    #[test]
    fn split_capped_at_three_groups_rest_unavailable() {
        let c: Vec<Candidate> = (0..4)
            .map(|i| cand(10 + i, A, i as i32, &[(1 + i, 5)]))
            .collect();
        let lines: Vec<Line> = (1..=4).map(|v| line(v, 1)).collect();
        let r = allocate(&lines, &c, A);
        assert_eq!(r.groups.len(), 3);
        assert_eq!(r.unavailable, vec![line(4, 1)]);
    }

    #[test]
    fn line_never_split() {
        let c = [cand(10, A, 0, &[(1, 2)]), cand(11, A, 1, &[(1, 2)])];
        let r = allocate(&[line(1, 3)], &c, A);
        assert!(r.full_fill.is_empty() && r.groups.is_empty());
        assert_eq!(r.unavailable, vec![line(1, 3)]);
    }

    #[test]
    fn fingerprint_stable_under_line_order() {
        let a = group_fingerprint(id(9), &[line(1, 2), line(2, 3)]);
        let b = group_fingerprint(id(9), &[line(2, 3), line(1, 2)]);
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert_ne!(a, group_fingerprint(id(9), &[line(1, 2), line(2, 4)]));
        assert_ne!(a, group_fingerprint(id(8), &[line(1, 2), line(2, 3)]));
    }
}
