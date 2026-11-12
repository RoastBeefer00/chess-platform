use leptos::prelude::*;

/// How many players are on the site right now.
///
/// This is the server-rendered read, and it does one more thing than read:
/// it counts the caller. That is what makes a guest sitting on the home page
/// part of the number — they have no friends socket heartbeating for them
/// (see `RedisClient::site_touch_online` for everything that feeds it).
///
/// The page does *not* call this again to stay fresh. Every server fn runs
/// behind the auth layer, which loads the user row, so polling it would be a
/// Postgres query per open tab per interval for as long as the tab stays
/// open — enough on its own to keep a serverless database from ever
/// suspending. Refreshes go to the plain `/api/online-count` route in
/// `main.rs` instead, which sits outside that layer and only reads Redis.
#[server]
pub async fn get_online_count() -> Result<u32, ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::state::AppState;
    use axum_login::AuthSession;

    let state = expect_context::<AppState>();
    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    if let Some(user) = auth.user {
        state.redis_client.site_touch_online(user.id).await;
    }
    Ok(state.redis_client.site_online_count().await)
}

/// Root-owned for the same reason as `MyRatingsResource`: the home page's
/// body is rebuilt on every streaming pass, and a resource created down
/// there would be fetched once per pass.
#[derive(Copy, Clone)]
pub struct OnlineCountResource(pub Resource<Option<u32>>);

/// Call once in the route that renders [`OnlineCount`].
pub fn provide_online_count() {
    provide_context(OnlineCountResource(Resource::new(
        || (),
        |_| async move { get_online_count().await.ok() },
    )));
}

/// How often the count refreshes while the page is open.
#[cfg(feature = "hydrate")]
const REFRESH_MS: u64 = 30_000;

/// `1234567` → `"1,234,567"`.
fn with_thousands(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "● 42 players online". Renders nothing until there is a number, and
/// nothing for zero — a signed-out visitor isn't counted, so on a quiet
/// site the landing page would otherwise announce that nobody is here.
#[component]
pub fn OnlineCount() -> impl IntoView {
    let initial = use_context::<OnlineCountResource>()
        .expect("provide_online_count must be called in the route")
        .0;
    // Overrides the server-rendered number once a refresh lands.
    let refreshed = RwSignal::new(None::<u32>);

    #[cfg(feature = "hydrate")]
    {
        use leptos_use::use_interval_fn;

        use_interval_fn(
            move || {
                leptos::task::spawn_local(async move {
                    let Ok(response) = gloo_net::http::Request::get("/api/online-count").send().await else {
                        return;
                    };
                    if !response.ok() {
                        return;
                    }
                    if let Some(n) = response.text().await.ok().and_then(|t| t.trim().parse().ok()) {
                        refreshed.set(Some(n));
                    }
                });
            },
            REFRESH_MS,
        );
    }

    view! {
        <Transition fallback=|| ()>
            {move || {
                let count = refreshed.get().or_else(|| initial.get().flatten()).filter(|n| *n > 0)?;
                Some(view! {
                    <span class="inline-flex items-center gap-2 text-xs font-medium text-zinc-400">
                        <span class="relative flex w-2 h-2">
                            <span class="absolute inline-flex w-full h-full rounded-full bg-emerald-500 opacity-60 animate-ping"></span>
                            <span class="relative inline-flex w-2 h-2 rounded-full bg-emerald-500"></span>
                        </span>
                        <span>
                            <span class="font-semibold text-zinc-200 tabular-nums">{with_thousands(count)}</span>
                            {if count == 1 { " player online" } else { " players online" }}
                        </span>
                    </span>
                })
            }}
        </Transition>
    }
}

#[cfg(test)]
mod tests {
    use super::with_thousands;

    #[test]
    fn groups_digits_in_threes() {
        assert_eq!(with_thousands(0), "0");
        assert_eq!(with_thousands(999), "999");
        assert_eq!(with_thousands(1_000), "1,000");
        assert_eq!(with_thousands(12_345), "12,345");
        assert_eq!(with_thousands(1_234_567), "1,234,567");
    }
}
