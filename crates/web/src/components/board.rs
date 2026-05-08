use leptos::{prelude::*, server};

#[server]
pub async fn get_board_state() -> Result<(), ServerFnError> {
    // Implementation for fetching board state
    Ok(())
}

#[component]
pub fn Board() -> impl IntoView {
    view! {
        <div class="flex items-center justify-center w-full h-[calc(100vh-3.5rem)]">
            <div class="grid grid-cols-8 grid-rows-8 w-[min(100vw,calc(100vh-11.5rem))] aspect-square">
                {(0..8).into_iter().map(|row| {
                    (0..8).into_iter().map(move |col| {
                        view! {
                            <div
                                class="w-full h-full"
                                class:bg-white=move || (row + col) % 2 == 0
                                class:bg-gray-800=move || (row + col) % 2 != 0
                            >
                            </div>
                        }
                    }).collect_view()
                }).collect_view()}
            </div>
        </div>
    }
}
