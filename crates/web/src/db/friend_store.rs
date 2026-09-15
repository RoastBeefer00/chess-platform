use shared::{FriendRelation, FriendSummary};
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::AuthError;

#[derive(Clone, Debug)]
pub struct FriendStore {
    pool: PgPool,
}

/// Why `send_request` didn't just insert a pending row. Lets the caller tell
/// "you're now friends" (the other side had already asked) apart from "request
/// sent" without a second round trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendRequestOutcome {
    Created,
    AutoAccepted,
    AlreadyPending,
    AlreadyFriends,
}

impl FriendStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Sends a friend request from `requester` to `addressee`. If `addressee`
    /// already has a pending request in to `requester` (the reverse
    /// direction), this auto-accepts it instead of erroring — mirrors how
    /// lichess/most social graphs treat "we both asked" as "we're friends".
    #[tracing::instrument(skip(self), fields(%requester, %addressee))]
    pub async fn send_request(
        &self,
        requester: Uuid,
        addressee: Uuid,
    ) -> Result<SendRequestOutcome, AuthError> {
        let mut tx = self.pool.begin().await?;

        // 1. Reverse-pending exists → this is an acceptance, not a new request.
        let auto_accepted = sqlx::query!(
            r#"UPDATE friendships SET status = 'accepted', updated_at = now()
               WHERE requester_id = $1 AND addressee_id = $2 AND status = 'pending'
               RETURNING id"#,
            addressee,
            requester,
        )
        .fetch_optional(&mut *tx)
        .await?;
        if auto_accepted.is_some() {
            tx.commit().await?;
            return Ok(SendRequestOutcome::AutoAccepted);
        }

        // 2. Otherwise try to insert a fresh pending row.
        let created = sqlx::query!(
            r#"INSERT INTO friendships (requester_id, addressee_id, status)
               VALUES ($1, $2, 'pending')
               ON CONFLICT DO NOTHING
               RETURNING id"#,
            requester,
            addressee,
        )
        .fetch_optional(&mut *tx)
        .await?;
        if created.is_some() {
            tx.commit().await?;
            return Ok(SendRequestOutcome::Created);
        }

        // 3. Conflict: a row already exists between this pair. Classify it.
        let existing = sqlx::query!(
            r#"SELECT status FROM friendships
               WHERE (LEAST(requester_id, addressee_id), GREATEST(requester_id, addressee_id))
                   = (LEAST($1::uuid, $2::uuid), GREATEST($1::uuid, $2::uuid))"#,
            requester,
            addressee,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(match existing.status.as_str() {
            "accepted" => SendRequestOutcome::AlreadyFriends,
            _ => SendRequestOutcome::AlreadyPending,
        })
    }

    /// `addressee` accepting a request `requester` sent them.
    #[tracing::instrument(skip(self), fields(%addressee, %requester))]
    pub async fn accept_request(&self, addressee: Uuid, requester: Uuid) -> Result<bool, AuthError> {
        let result = sqlx::query!(
            r#"UPDATE friendships SET status = 'accepted', updated_at = now()
               WHERE requester_id = $1 AND addressee_id = $2 AND status = 'pending'"#,
            requester,
            addressee,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// `addressee` declining a request `requester` sent them.
    #[tracing::instrument(skip(self), fields(%addressee, %requester))]
    pub async fn decline_request(&self, addressee: Uuid, requester: Uuid) -> Result<bool, AuthError> {
        let result = sqlx::query!(
            r#"DELETE FROM friendships
               WHERE requester_id = $1 AND addressee_id = $2 AND status = 'pending'"#,
            requester,
            addressee,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// `requester` withdrawing their own outgoing request.
    #[tracing::instrument(skip(self), fields(%requester, %addressee))]
    pub async fn cancel_request(&self, requester: Uuid, addressee: Uuid) -> Result<bool, AuthError> {
        let result = sqlx::query!(
            r#"DELETE FROM friendships
               WHERE requester_id = $1 AND addressee_id = $2 AND status = 'pending'"#,
            requester,
            addressee,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Removes an accepted friendship, either direction.
    #[tracing::instrument(skip(self), fields(%user, %other))]
    pub async fn remove_friend(&self, user: Uuid, other: Uuid) -> Result<bool, AuthError> {
        let result = sqlx::query!(
            r#"DELETE FROM friendships
               WHERE status = 'accepted'
                 AND ((requester_id = $1 AND addressee_id = $2)
                   OR (requester_id = $2 AND addressee_id = $1))"#,
            user,
            other,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    #[tracing::instrument(skip(self), fields(%a, %b))]
    pub async fn are_friends(&self, a: Uuid, b: Uuid) -> Result<bool, AuthError> {
        Ok(sqlx::query_scalar!(
            r#"SELECT 1 AS "one!" FROM friendships
               WHERE status = 'accepted'
                 AND ((requester_id = $1 AND addressee_id = $2)
                   OR (requester_id = $2 AND addressee_id = $1))"#,
            a,
            b,
        )
        .fetch_optional(&self.pool)
        .await?
        .is_some())
    }

    /// `viewer`'s relationship to `target` — the single-pair version of the
    /// per-row classification `search_users` does in bulk.
    #[tracing::instrument(skip(self), fields(%viewer, %target))]
    pub async fn relation(&self, viewer: Uuid, target: Uuid) -> Result<FriendRelation, AuthError> {
        let row = sqlx::query!(
            r#"SELECT status, requester_id FROM friendships
               WHERE (LEAST(requester_id, addressee_id), GREATEST(requester_id, addressee_id))
                   = (LEAST($1::uuid, $2::uuid), GREATEST($1::uuid, $2::uuid))"#,
            viewer,
            target,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(match row {
            None => FriendRelation::None,
            Some(r) if r.status == "accepted" => FriendRelation::Friends,
            Some(r) if r.requester_id == viewer => FriendRelation::PendingOutgoing,
            Some(_) => FriendRelation::PendingIncoming,
        })
    }

    /// Accepted friends, joined with `users` for display, ordered by
    /// username.
    #[tracing::instrument(skip(self), fields(%user_id))]
    pub async fn list_friends(&self, user_id: Uuid) -> Result<Vec<FriendSummary>, AuthError> {
        let rows = sqlx::query!(
            r#"SELECT u.id, u.username, u.avatar_url
               FROM friendships f
               JOIN users u ON u.id = CASE WHEN f.requester_id = $1 THEN f.addressee_id ELSE f.requester_id END
               WHERE f.status = 'accepted' AND (f.requester_id = $1 OR f.addressee_id = $1)
               ORDER BY u.username"#,
            user_id,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| FriendSummary { id: r.id, username: r.username, avatar_url: r.avatar_url })
            .collect())
    }

    /// Ids only, no `users` join — the presence fan-out path, called on
    /// every socket open and close, so it shouldn't pay for a join.
    #[tracing::instrument(skip(self), fields(%user_id))]
    pub async fn list_friend_ids(&self, user_id: Uuid) -> Result<Vec<Uuid>, AuthError> {
        let rows = sqlx::query!(
            r#"SELECT CASE WHEN requester_id = $1 THEN addressee_id ELSE requester_id END AS "friend_id!"
               FROM friendships
               WHERE status = 'accepted' AND (requester_id = $1 OR addressee_id = $1)"#,
            user_id,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| r.friend_id).collect())
    }

    /// Pending requests `user_id` received (someone else asked them).
    #[tracing::instrument(skip(self), fields(%user_id))]
    pub async fn list_pending_received(&self, user_id: Uuid) -> Result<Vec<FriendSummary>, AuthError> {
        let rows = sqlx::query!(
            r#"SELECT u.id, u.username, u.avatar_url
               FROM friendships f
               JOIN users u ON u.id = f.requester_id
               WHERE f.addressee_id = $1 AND f.status = 'pending'
               ORDER BY f.created_at DESC"#,
            user_id,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| FriendSummary { id: r.id, username: r.username, avatar_url: r.avatar_url })
            .collect())
    }

    /// Pending requests `user_id` sent (they asked someone else).
    #[tracing::instrument(skip(self), fields(%user_id))]
    pub async fn list_pending_sent(&self, user_id: Uuid) -> Result<Vec<FriendSummary>, AuthError> {
        let rows = sqlx::query!(
            r#"SELECT u.id, u.username, u.avatar_url
               FROM friendships f
               JOIN users u ON u.id = f.addressee_id
               WHERE f.requester_id = $1 AND f.status = 'pending'
               ORDER BY f.created_at DESC"#,
            user_id,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| FriendSummary { id: r.id, username: r.username, avatar_url: r.avatar_url })
            .collect())
    }

    /// Prefix match on username, excluding `exclude` (the searching user),
    /// guests, and users with no username yet (mid-onboarding). Returns each
    /// match's relation to `exclude` so the UI can render Add / Pending /
    /// Friends per row without a second query.
    #[tracing::instrument(skip(self), fields(%exclude, query))]
    pub async fn search_users(
        &self,
        query: &str,
        exclude: Uuid,
        limit: i64,
    ) -> Result<Vec<(FriendSummary, FriendRelation)>, AuthError> {
        let rows = sqlx::query!(
            r#"SELECT u.id, u.username, u.avatar_url,
                      f.status AS "relation_status?", f.requester_id AS "relation_requester?"
               FROM users u
               LEFT JOIN friendships f
                 ON (f.requester_id = u.id AND f.addressee_id = $2)
                 OR (f.addressee_id = u.id AND f.requester_id = $2)
               WHERE u.id <> $2
                 AND u.is_guest = false
                 AND u.username IS NOT NULL
                 AND lower(u.username) LIKE lower($1) || '%'
               ORDER BY u.username
               LIMIT $3"#,
            query,
            exclude,
            limit,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let relation = match r.relation_status.as_deref() {
                    Some("accepted") => FriendRelation::Friends,
                    Some("pending") if r.relation_requester == Some(exclude) => {
                        FriendRelation::PendingOutgoing
                    }
                    Some("pending") => FriendRelation::PendingIncoming,
                    _ => FriendRelation::None,
                };
                (
                    FriendSummary { id: r.id, username: r.username, avatar_url: r.avatar_url },
                    relation,
                )
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    async fn insert_user(pool: &PgPool) -> Uuid {
        insert_named_user(pool, None).await
    }

    async fn insert_named_user(pool: &PgPool, username: Option<&str>) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO users (id, email, username) VALUES ($1, $2, $3)",
            id,
            format!("{}@test.invalid", id),
            username,
        )
        .execute(pool)
        .await
        .unwrap();
        id
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn send_request_creates_pending_row(pool: PgPool) {
        let store = FriendStore::new(pool.clone());
        let a = insert_user(&pool).await;
        let b = insert_user(&pool).await;

        let outcome = store.send_request(a, b).await.unwrap();
        assert_eq!(outcome, SendRequestOutcome::Created);
        assert!(!store.are_friends(a, b).await.unwrap());

        let received = store.list_pending_received(b).await.unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].id, a);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reverse_request_auto_accepts(pool: PgPool) {
        let store = FriendStore::new(pool.clone());
        let a = insert_user(&pool).await;
        let b = insert_user(&pool).await;

        store.send_request(a, b).await.unwrap();
        let outcome = store.send_request(b, a).await.unwrap();
        assert_eq!(outcome, SendRequestOutcome::AutoAccepted);

        assert!(store.are_friends(a, b).await.unwrap());
        assert!(store.are_friends(b, a).await.unwrap());

        let count: i64 = sqlx::query_scalar!("SELECT COUNT(*) AS \"count!\" FROM friendships")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1, "auto-accept must update the existing row, not add a second one");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn duplicate_same_direction_request_is_already_pending(pool: PgPool) {
        let store = FriendStore::new(pool.clone());
        let a = insert_user(&pool).await;
        let b = insert_user(&pool).await;

        store.send_request(a, b).await.unwrap();
        let outcome = store.send_request(a, b).await.unwrap();
        assert_eq!(outcome, SendRequestOutcome::AlreadyPending);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn request_after_already_friends_is_already_friends(pool: PgPool) {
        let store = FriendStore::new(pool.clone());
        let a = insert_user(&pool).await;
        let b = insert_user(&pool).await;

        store.send_request(a, b).await.unwrap();
        store.accept_request(b, a).await.unwrap();

        let outcome = store.send_request(a, b).await.unwrap();
        assert_eq!(outcome, SendRequestOutcome::AlreadyFriends);
        let outcome_reverse = store.send_request(b, a).await.unwrap();
        assert_eq!(outcome_reverse, SendRequestOutcome::AlreadyFriends);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn raw_reverse_direction_insert_hits_unique_index(pool: PgPool) {
        let a = insert_user(&pool).await;
        let b = insert_user(&pool).await;

        sqlx::query!(
            "INSERT INTO friendships (requester_id, addressee_id, status) VALUES ($1, $2, 'pending')",
            a,
            b,
        )
        .execute(&pool)
        .await
        .unwrap();

        let result = sqlx::query!(
            "INSERT INTO friendships (requester_id, addressee_id, status) VALUES ($1, $2, 'pending')",
            b,
            a,
        )
        .execute(&pool)
        .await;
        assert!(result.is_err(), "reverse-direction row must violate the pair-unique index");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn self_friend_request_hits_check_constraint(pool: PgPool) {
        let a = insert_user(&pool).await;

        let result = sqlx::query!(
            "INSERT INTO friendships (requester_id, addressee_id, status) VALUES ($1, $1, 'pending')",
            a,
        )
        .execute(&pool)
        .await;
        assert!(result.is_err(), "self-request must violate the no-self CHECK constraint");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn accept_decline_cancel_remove_change_state_correctly(pool: PgPool) {
        let store = FriendStore::new(pool.clone());
        let a = insert_user(&pool).await;
        let b = insert_user(&pool).await;
        let c = insert_user(&pool).await;

        // accept
        store.send_request(a, b).await.unwrap();
        assert!(store.accept_request(b, a).await.unwrap());
        assert!(store.are_friends(a, b).await.unwrap());
        // wrong-direction accept is a no-op
        assert!(!store.accept_request(a, b).await.unwrap());

        // decline
        store.send_request(a, c).await.unwrap();
        assert!(store.decline_request(c, a).await.unwrap());
        assert!(!store.are_friends(a, c).await.unwrap());
        assert!(store.list_pending_received(c).await.unwrap().is_empty());

        // cancel
        store.send_request(a, c).await.unwrap();
        assert!(store.cancel_request(a, c).await.unwrap());
        assert!(store.list_pending_sent(a).await.unwrap().is_empty());

        // remove_friend, both argument orders
        assert!(store.remove_friend(b, a).await.unwrap());
        assert!(!store.are_friends(a, b).await.unwrap());
        assert!(!store.remove_friend(a, b).await.unwrap(), "already removed, second call is a no-op");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn list_friends_returns_the_other_party_for_both_sides(pool: PgPool) {
        let store = FriendStore::new(pool.clone());
        let a = insert_named_user(&pool, Some("alice")).await;
        let b = insert_named_user(&pool, Some("bob")).await;

        store.send_request(a, b).await.unwrap();
        store.accept_request(b, a).await.unwrap();

        let a_view = store.list_friends(a).await.unwrap();
        assert_eq!(a_view.len(), 1);
        assert_eq!(a_view[0].id, b);

        let b_view = store.list_friends(b).await.unwrap();
        assert_eq!(b_view.len(), 1);
        assert_eq!(b_view[0].id, a);

        assert_eq!(store.list_friend_ids(a).await.unwrap(), vec![b]);
        assert_eq!(store.list_friend_ids(b).await.unwrap(), vec![a]);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn pending_received_and_sent_are_directional(pool: PgPool) {
        let store = FriendStore::new(pool.clone());
        let a = insert_user(&pool).await;
        let b = insert_user(&pool).await;

        store.send_request(a, b).await.unwrap();

        assert_eq!(store.list_pending_sent(a).await.unwrap().len(), 1);
        assert!(store.list_pending_received(a).await.unwrap().is_empty());
        assert_eq!(store.list_pending_received(b).await.unwrap().len(), 1);
        assert!(store.list_pending_sent(b).await.unwrap().is_empty());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn search_users_matches_prefix_case_insensitively(pool: PgPool) {
        let store = FriendStore::new(pool.clone());
        let me = insert_user(&pool).await;
        let alice = insert_named_user(&pool, Some("Alice42")).await;
        let _bob = insert_named_user(&pool, Some("Bob")).await;

        let results = store.search_users("alic", me, 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.id, alice);
        assert_eq!(results[0].1, FriendRelation::None);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn search_users_excludes_self_guests_and_unnamed(pool: PgPool) {
        let store = FriendStore::new(pool.clone());
        let me = insert_named_user(&pool, Some("zebra")).await;
        let _no_username = insert_user(&pool).await;

        let guest_id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO users (id, email, username, is_guest) VALUES ($1, $2, 'zebraguest', true)",
            guest_id,
            format!("{}@test.invalid", guest_id),
        )
        .execute(&pool)
        .await
        .unwrap();

        let results = store.search_users("zebra", me, 10).await.unwrap();
        assert!(results.is_empty(), "must exclude self, guests, and unnamed users");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn search_users_reports_relation_for_all_four_states(pool: PgPool) {
        let store = FriendStore::new(pool.clone());
        let me = insert_named_user(&pool, Some("me")).await;
        let stranger = insert_named_user(&pool, Some("relstranger")).await;
        let outgoing_target = insert_named_user(&pool, Some("reloutgoing")).await;
        let incoming_source = insert_named_user(&pool, Some("relincoming")).await;
        let friend = insert_named_user(&pool, Some("relfriend")).await;

        store.send_request(me, outgoing_target).await.unwrap();
        store.send_request(incoming_source, me).await.unwrap();
        store.send_request(me, friend).await.unwrap();
        store.accept_request(friend, me).await.unwrap();

        let results = store.search_users("rel", me, 10).await.unwrap();
        let relation_of = |id: Uuid| results.iter().find(|(u, _)| u.id == id).unwrap().1;

        assert_eq!(relation_of(stranger), FriendRelation::None);
        assert_eq!(relation_of(outgoing_target), FriendRelation::PendingOutgoing);
        assert_eq!(relation_of(incoming_source), FriendRelation::PendingIncoming);
        assert_eq!(relation_of(friend), FriendRelation::Friends);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn relation_reports_all_four_states_for_a_single_pair(pool: PgPool) {
        let store = FriendStore::new(pool.clone());
        let a = insert_user(&pool).await;
        let b = insert_user(&pool).await;

        assert_eq!(store.relation(a, b).await.unwrap(), FriendRelation::None);

        store.send_request(a, b).await.unwrap();
        assert_eq!(store.relation(a, b).await.unwrap(), FriendRelation::PendingOutgoing);
        assert_eq!(store.relation(b, a).await.unwrap(), FriendRelation::PendingIncoming);

        store.accept_request(b, a).await.unwrap();
        assert_eq!(store.relation(a, b).await.unwrap(), FriendRelation::Friends);
        assert_eq!(store.relation(b, a).await.unwrap(), FriendRelation::Friends);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn search_users_respects_limit(pool: PgPool) {
        let store = FriendStore::new(pool.clone());
        let me = insert_user(&pool).await;
        for i in 0..5 {
            insert_named_user(&pool, Some(&format!("limituser{i}"))).await;
        }

        let results = store.search_users("limituser", me, 2).await.unwrap();
        assert_eq!(results.len(), 2);
    }
}
