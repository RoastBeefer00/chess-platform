use leptos::prelude::*;

#[cfg(feature = "hydrate")]
use {js_sys::Date, leptos_use::use_interval_fn};

#[component]
#[cfg_attr(not(feature = "hydrate"), allow(unused_variables))]
pub fn Clock(
    snapshot_ms: Signal<i64>,
    snapshot_sent_at_ms: Signal<i64>,
    is_active: Signal<bool>,
    /// Client→server clock offset in ms (server ≈ client + offset), from the
    /// ping/pong handshake. Added to `Date::now()` so elapsed is measured in
    /// server time rather than the (possibly skewed) browser wall clock.
    /// Defaults to 0 — identical to pre-handshake behavior until it converges.
    #[prop(into, optional)]
    offset_ms: Signal<i64>,
) -> impl IntoView {
    let displayed_ms = RwSignal::new(snapshot_ms.get_untracked());

    #[cfg(feature = "hydrate")]
    use_interval_fn(
        move || {
            let snap = snapshot_ms.get_untracked();
            if is_active.get_untracked() {
                // Measure elapsed in *server* time: Date::now() + offset maps the
                // browser clock onto the server clock (offset from the ping/pong
                // handshake), so a skewed browser wall clock no longer throws the
                // countdown off and the visual 0:00 lines up with the server flag.
                let server_now = Date::now() as i64 + offset_ms.get_untracked();
                let elapsed = server_now - snapshot_sent_at_ms.get_untracked();
                displayed_ms.set((snap - elapsed).max(0));
            } else {
                displayed_ms.set(snap);
            }
        },
        100,
    );

    let state_classes = move || {
        let ms = displayed_ms.get();
        let active = is_active.get();
        if !active || ms == 0 {
            "text-zinc-300"
        } else if ms < 20_000 {
            "bg-red-500 text-white font-bold"
        } else {
            "bg-zinc-100 text-black"
        }
    };

    view! {
        <div class=move || format!("px-3 py-2 rounded font-mono text-lg transition-colors {}", state_classes())>
            {move || format_clock(displayed_ms.get())}
        </div>
    }
}

fn format_clock(ms: i64) -> String {
    let total_seconds = ms / 1000;
    let mins = total_seconds / 60;
    let secs = total_seconds % 60;
    if ms <= 20_000 {
        let tenths = (ms % 1000) / 100;
        format!("{secs:02}.{tenths}")
    } else if ms < 600_000 {
        format!("{mins:01}:{secs:02}")
    } else {
        format!("{mins:02}:{secs:02}")
    }
}
