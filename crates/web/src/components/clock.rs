use leptos::prelude::*;

#[cfg(feature = "hydrate")]
use {js_sys::Date, leptos_use::use_interval_fn};

#[component]
#[cfg_attr(not(feature = "hydrate"), allow(unused_variables))]
pub fn Clock(
    snapshot_ms: Signal<i64>,
    snapshot_sent_at_ms: Signal<i64>,
    is_active: Signal<bool>,
) -> impl IntoView {
    let displayed_ms = RwSignal::new(snapshot_ms.get_untracked());

    #[cfg(feature = "hydrate")]
    use_interval_fn(
        move || {
            let snap = snapshot_ms.get_untracked();
            if is_active.get_untracked() {
                // Use server's sent_at as the time origin. Assumes browser and
                // server wall clocks are NTP-synced (~50ms in practice).
                // This accounts for network lag so the displayed countdown
                // matches server reality, instead of lagging behind by RTT.
                let elapsed = Date::now() as i64 - snapshot_sent_at_ms.get_untracked();
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
