use leptos::{html::Div, prelude::*};

/// The shared chrome for every modal dialog in the app: the dimmed backdrop,
/// the card, and the accessibility behavior that goes with them.
///
/// The three modals that predate this (`GameOverModal`, `MatchmakingModal`,
/// and the friends-list challenge dialog) each hand-rolled the same
/// `fixed inset-0 z-50 … bg-black/60` backdrop, and each ended up with a
/// different subset of the behavior: some closed on an outside click, none
/// closed on Escape, none announced themselves as dialogs, and none moved
/// focus. A keyboard-only user could not dismiss the game-over modal at all.
/// `PromotionPicker` got this right on its own and is deliberately left
/// alone — it is not a backdrop-and-card modal, it is a popover anchored to
/// a board square.
///
/// What this provides:
///   - `role="dialog"` + `aria-modal="true"`, so assistive tech announces it
///     as a dialog and treats the content behind it as inert.
///   - `aria-labelledby` pointing at the card's own heading, via a caller-
///     supplied `labelled_by` id. The heading stays in the caller's
///     `children` so each modal keeps its own layout.
///   - Escape to close, matching `PromotionPicker`.
///   - Click outside the card to close, when `dismissible` (the default).
///   - Focus moves into the card on mount and returns to whatever had it
///     before, on unmount — otherwise a keyboard user's focus is stranded
///     behind the backdrop after the dialog goes away.
///
/// `dismissible=false` keeps Escape and outside clicks from closing it, for a
/// dialog whose only exits should be explicit buttons.
#[component]
#[cfg_attr(not(feature = "hydrate"), allow(unused_variables))]
pub fn ModalShell(
    /// Element id of the heading inside `children` that names this dialog.
    #[prop(into)]
    labelled_by: String,
    #[prop(into)] on_close: Callback<()>,
    /// Extra classes for the card, e.g. `"max-w-xs"` or `"text-center"`.
    #[prop(optional, default = "")]
    card_class: &'static str,
    #[prop(optional, default = true)] dismissible: bool,
    children: Children,
) -> impl IntoView {
    let card_ref = NodeRef::<Div>::new();

    #[cfg(feature = "hydrate")]
    {
        use leptos::ev;
        use wasm_bindgen::JsCast as _;

        // Remember who had focus so it can be handed back on close. Read at
        // mount time, before focus moves into the card below.
        let previously_focused = StoredValue::new_local(
            web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.active_element()),
        );

        Effect::new(move |_| {
            if let Some(card) = card_ref.get() {
                // `focus()` on a container needs it to be focusable; the
                // card carries `tabindex="-1"` below for exactly this.
                let _ = card.focus();
            }
        });

        if dismissible {
            let handle = window_event_listener(ev::keydown, move |e: web_sys::KeyboardEvent| {
                if e.key() == "Escape" {
                    on_close.run(());
                }
            });
            on_cleanup(move || handle.remove());

            let stop = leptos_use::on_click_outside(card_ref, move |_| on_close.run(()));
            on_cleanup(stop);
        }

        on_cleanup(move || {
            if let Some(el) = previously_focused.get_value() {
                if let Some(el) = el.dyn_ref::<web_sys::HtmlElement>() {
                    let _ = el.focus();
                }
            }
        });
    }

    view! {
        <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
            <div
                node_ref=card_ref
                role="dialog"
                aria-modal="true"
                aria-labelledby=labelled_by
                tabindex="-1"
                class=format!(
                    "relative w-full mx-4 surface-card shadow-xl outline-none {card_class}"
                )
            >
                {children()}
            </div>
        </div>
    }
}
