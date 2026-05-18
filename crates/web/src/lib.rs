#![recursion_limit = "256"]

pub mod app;
pub mod components;
pub mod game;
pub mod matchmaking;
pub mod pages;
pub mod websocket;

#[cfg(feature = "ssr")]
pub mod auth;
#[cfg(feature = "ssr")]
pub mod game_room;
#[cfg(feature = "ssr")]
pub mod state;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    use crate::app::*;
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_lazy(App);
}
