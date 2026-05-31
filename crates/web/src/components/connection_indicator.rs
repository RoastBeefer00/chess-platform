use leptos::prelude::*;
use shared::rtt_bucket;

#[component]
pub fn ConnectionIndicator(
    #[prop(into)] rtt_ms: Signal<Option<u32>>,
    #[prop(into)] connected: Signal<bool>,
) -> impl IntoView {
    let dot_class = move || {
        if !connected.get() {
            "w-2.5 h-2.5 rounded-full bg-red-500 ring-2 ring-red-300 flex-shrink-0"
        } else {
            match rtt_ms.get() {
                None => "w-2.5 h-2.5 rounded-full bg-zinc-500 animate-pulse flex-shrink-0",
                Some(rtt) => match rtt_bucket(rtt) {
                    0 => "w-2.5 h-2.5 rounded-full bg-emerald-500 flex-shrink-0",
                    1 => "w-2.5 h-2.5 rounded-full bg-yellow-500 flex-shrink-0",
                    2 => "w-2.5 h-2.5 rounded-full bg-orange-500 flex-shrink-0",
                    _ => "w-2.5 h-2.5 rounded-full bg-red-500 flex-shrink-0",
                },
            }
        }
    };

    let title = move || {
        if !connected.get() {
            "disconnected".to_string()
        } else {
            match rtt_ms.get() {
                None => "connecting\u{2026}".to_string(),
                Some(rtt) => format!("ping {} ms", rtt),
            }
        }
    };

    view! {
        <div class=dot_class title=title></div>
    }
}
