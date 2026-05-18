use leptos::prelude::*;
use server_fn::{codec::JsonEncoding, BoxedStream, Websocket};
use shared::{MatchmakingClientMessage, MatchmakingServerMessage};

#[server(protocol = Websocket<JsonEncoding, JsonEncoding>)]
pub async fn matchmaking_websocket(
    input: BoxedStream<MatchmakingClientMessage, ServerFnError>,
) -> Result<BoxedStream<MatchmakingServerMessage, ServerFnError>, ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::state::AppState;
    use axum_login::AuthSession;
    use shared::Side;
    use tokio_stream::StreamExt as _;

    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    let player_id = auth
        .user
        .ok_or_else(|| ServerFnError::new("unauthenticated"))?
        .id;
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
                bucket,
                rating_mode,
            })) = input.next().await
            else {
                let _ = tx.unbounded_send(Err(ServerFnError::new("expected Join")));
                return Err(());
            };

            let key = bucket.id(rating_mode);
            queued_key = Some(key.clone());

            let player_rating = match auth
                .backend
                .get_user_rating(&player_id, bucket.mode())
                .await
            {
                Ok(r) => r,
                Err(_) => {
                    let _ = tx
                        .unbounded_send(Err(ServerFnError::new("unable to get player rating")));
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

            let _ = tx.unbounded_send(Ok(MatchmakingServerMessage::Queued { bucket }));

            match state
                .redis_client
                .find_pair(&key, player_id, player_rating, 100)
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
                    let game_id = state.create_game(white, black).await;
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
            let _ = state
                .redis_client
                .remove_from_bucket(&key, player_id)
                .await;
        }
    });

    Ok(rx.into())
}
