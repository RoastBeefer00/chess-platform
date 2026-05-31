use leptos::prelude::*;
use server_fn::{codec::JsonEncoding, BoxedStream, Websocket};
use shared::{Category, MatchmakingClientMessage, MatchmakingServerMessage};
use uuid::Uuid;

/// Rating window for matchmaking pairing (±points). Wider = matches faster
/// but less skill-balanced. Lichess uses adaptive widening starting around
/// ±50; we hardcode a single wider window for now since concurrent users
/// are low. Revisit when queue depth grows.
#[cfg(feature = "ssr")]
const RATING_WINDOW: u32 = 500;

#[server]
pub async fn get_user_rating(id: Uuid, category: Category) -> Result<u32, ServerFnError> {
    use crate::state::AppState;

    let state = expect_context::<AppState>();
    state
        .rating_store
        .get_rating(&id, category)
        .await
        .map_err(ServerFnError::new)
}

#[server(protocol = Websocket<JsonEncoding, JsonEncoding>)]
pub async fn matchmaking_websocket(
    input: BoxedStream<MatchmakingClientMessage, ServerFnError>,
) -> Result<BoxedStream<MatchmakingServerMessage, ServerFnError>, ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::state::AppState;
    use axum_login::AuthSession;
    use shared::{GameConfig, Side, Variant};
    use tokio_stream::StreamExt as _;

    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    let user = auth
        .user
        .ok_or_else(|| ServerFnError::new("unauthenticated"))?;
    // Defense in depth: the home page redirects no-username users to the
    // onboarding form before they can click matchmaking, but a hand-crafted
    // WS request could still try. Without this gate, a no-username user gets
    // queued and their opponent ends up in a game against "Anonymous".
    if user.username.is_none() {
        return Err(ServerFnError::new("complete onboarding first"));
    }
    let player_id = user.id;
    let state = expect_context::<AppState>();

    let (tx, rx) =
        futures::channel::mpsc::unbounded::<Result<MatchmakingServerMessage, ServerFnError>>();
    state.add_match_inbox(player_id, tx.clone()).await;

    tokio::spawn(async move {
        let mut input = input;
        // Holds the bucket key once we know it, so cleanup can ZREM regardless of path.
        let mut queued_key: Option<String> = None;

        let _ = async {
            // First message MUST be Join.
            let Some(Ok(MatchmakingClientMessage::Join {
                time_control,
                rating_mode,
            })) = input.next().await
            else {
                let _ = tx.unbounded_send(Err(ServerFnError::new("expected Join")));
                return Err(());
            };

            let key = time_control.bucket(rating_mode);
            queued_key = Some(key.clone());

            let player_rating = match state
                .rating_store
                .get_rating(&player_id, time_control.category())
                .await
            {
                Ok(r) => r,
                Err(_) => {
                    let _ =
                        tx.unbounded_send(Err(ServerFnError::new("unable to get player rating")));
                    return Err(());
                }
            };

            if let Err(e) = state
                .redis_client
                .add_to_bucket(&key, player_id, player_rating)
                .await
            {
                let _ = tx.unbounded_send(Err(ServerFnError::new(e.to_string())));
                return Err(());
            }

            let _ = tx.unbounded_send(Ok(MatchmakingServerMessage::Queued {
                time_control: time_control.clone(),
            }));

            match state
                .redis_client
                .find_pair(&key, player_id, player_rating, RATING_WINDOW)
                .await
            {
                Err(e) => {
                    let _ = tx.unbounded_send(Err(ServerFnError::new(e.to_string())));
                    return Err(());
                }
                Ok(Some(opponent_id)) => {
                    // Coin flip for side. Loser of the flip plays black.
                    let (white, black, my_side) = if rand::random::<bool>() {
                        (player_id, opponent_id, Side::White)
                    } else {
                        (opponent_id, player_id, Side::Black)
                    };
                    let game_config = GameConfig {
                        time_control,
                        variant: Variant::Standard,
                        rated: rating_mode,
                    };
                    let game_id = match state.create_game(game_config, white, black, (0.0, 0.0)).await {
                        Ok(id) => id,
                        Err(e) => {
                            tracing::warn!(?e, "matchmaking: create_game failed");
                            let _ = tx.unbounded_send(Err(ServerFnError::new(e.to_string())));
                            return Err(());
                        }
                    };
                    // Notify opponent via their inbox.
                    state
                        .notify_match(
                            opponent_id,
                            MatchmakingServerMessage::Matched {
                                game: game_id,
                                side: my_side.opposite(),
                            },
                        )
                        .await;
                    // Notify self.
                    let _ = tx.unbounded_send(Ok(MatchmakingServerMessage::Matched {
                        game: game_id,
                        side: my_side,
                    }));
                }
                Ok(None) => {
                    // No opponent yet — wait for either the client to disconnect
                    // or another player's matcher to push into our inbox (which
                    // the framework drains via `rx`, not us).
                }
            }

            // Block until the client disconnects (or sends anything — currently ignored).
            while input.next().await.is_some() {}
            Ok(())
        }
        .await;

        // Always-runs cleanup, regardless of which exit path was taken.
        state.remove_match_inbox(&player_id).await;
        if let Some(key) = queued_key {
            let _ = state.redis_client.remove_from_bucket(&key, player_id).await;
        }
    });

    Ok(rx.into())
}
