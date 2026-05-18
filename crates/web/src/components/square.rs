#[cfg(feature = "hydrate")]
use futures::channel::mpsc::Sender;
use leptos::{html::Img, prelude::*};
#[cfg(feature = "hydrate")]
use leptos_use::UseDraggableReturn;
#[cfg(feature = "hydrate")]
use shakmaty::Chess;
use shakmaty::{Color, Piece, Square};
#[cfg(feature = "hydrate")]
use shared::{GameClientMessage, PlayerRole};

use crate::components::BoardPerspective;

#[component]
pub fn Square(
    rank: usize,
    file: usize,
    piece: Signal<Option<Piece>>,
    perspective: Signal<BoardPerspective>,
) -> impl IntoView {
    let valid_move_targets = expect_context::<Signal<Vec<shakmaty::Square>>>();
    let selected_square = expect_context::<RwSignal<Option<shakmaty::Square>>>();
    let last_move = expect_context::<RwSignal<Option<(shakmaty::Square, shakmaty::Square)>>>();
    #[cfg(feature = "hydrate")]
    let tx = expect_context::<Sender<GameClientMessage>>();
    #[cfg(feature = "hydrate")]
    let (position, _) = expect_context::<(ReadSignal<Chess>, WriteSignal<Chess>)>();
    #[cfg(feature = "hydrate")]
    let player_role = expect_context::<ReadSignal<Option<PlayerRole>>>();

    let image_path = Signal::derive(move || {
        piece.get().map(|p| {
            let color = match p.color {
                Color::White => "w",
                Color::Black => "b",
            };
            let role = match p.role {
                shakmaty::Role::Pawn => "P",
                shakmaty::Role::Knight => "N",
                shakmaty::Role::Bishop => "B",
                shakmaty::Role::Rook => "R",
                shakmaty::Role::Queen => "Q",
                shakmaty::Role::King => "K",
            };
            format!("/piece/alpha/{}{}.svg", color, role)
        })
    });

    let el = NodeRef::<Img>::new();

    #[cfg(feature = "hydrate")]
    let tx_click = tx.clone();

    #[cfg(feature = "hydrate")]
    let drag_size = RwSignal::new((0.0_f64, 0.0_f64));

    #[cfg(feature = "hydrate")]
    let UseDraggableReturn {
        is_dragging, style, ..
    } = {
        use leptos_use::core::Position;
        use leptos_use::{
            use_draggable_with_options, UseDraggableCallbackArgs, UseDraggableOptions,
        };

        let pos = RwSignal::new(Position { x: 0.0, y: 0.0 });

        use_draggable_with_options(
            el,
            UseDraggableOptions::default()
                .initial_value(pos)
                .on_start(move |_: UseDraggableCallbackArgs| {
                    let can_drag = player_role
                        .get()
                        .and_then(|r| r.color())
                        .zip(piece.get())
                        .is_some_and(|(my_color, p)| my_color == p.color);
                    if can_drag {
                        selected_square.set(Some(Square::new((rank * 8 + file) as u32)));
                        if let Some(element) = el.get_untracked() {
                            let rect = element.get_bounding_client_rect();
                            pos.set(Position {
                                x: rect.left(),
                                y: rect.top(),
                            });
                            drag_size.set((rect.width(), rect.height()));
                        }
                        true
                    } else {
                        false
                    }
                })
                .on_end(move |args: UseDraggableCallbackArgs| {
                    let mut tx = tx.clone();
                    let (x, y) = (args.event.client_x() as f32, args.event.client_y() as f32);
                    let dom_element = web_sys::window()
                        .unwrap()
                        .document()
                        .unwrap()
                        .element_from_point(x, y)
                        .unwrap();

                    let data = dom_element.closest("[data-square]").unwrap();
                    if let Some(el) = data {
                        use std::str::FromStr;

                        if player_role
                            .get()
                            .is_some_and(|role| role != PlayerRole::Spectator)
                        {
                            let attr = el.get_attribute("data-square").unwrap();
                            leptos::logging::log!("dropped on square {}", attr);
                            if let Ok(dropped_square) = Square::from_str(&attr) {
                                if valid_move_targets.get().contains(&dropped_square) {
                                    use shakmaty::{Position as _, Role};
                                    let from_sq = selected_square.get_untracked().unwrap();
                                    let legal = position.get_untracked().legal_moves();
                                    // For promotions, default to queen
                                    if let Some(m) = legal.iter().find(|m| {
                                        m.from() == Some(from_sq)
                                            && m.to() == dropped_square
                                            && m.promotion().map_or(true, |r| r == Role::Queen)
                                    }) {
                                        let m = m.clone();
                                        selected_square.set(None);
                                        let _ = tx.try_send(GameClientMessage::MoveMade {
                                            uci: m
                                                .to_uci(shakmaty::CastlingMode::Standard)
                                                .to_string(),
                                        });
                                    }
                                }
                            }
                        }
                    } else {
                        leptos::logging::log!("dropped outside of board");
                    }
                }),
        )
    };

    fn rank_to_char(rank: usize) -> char {
        std::char::from_digit((rank + 1) as u32, 10).unwrap()
    }

    fn file_to_char(file: usize) -> char {
        (b'a' + file as u8) as char
    }

    #[cfg(not(feature = "hydrate"))]
    let (is_dragging, style, drag_size) = (
        Signal::derive(|| false),
        Signal::derive(|| String::new()),
        Signal::derive(|| (0.0_f64, 0.0_f64)),
    );

    #[cfg(feature = "hydrate")]
    let on_click = {
        let mut tx = tx_click;
        move |_: leptos::ev::MouseEvent| {
            let this_square = Square::new((rank * 8 + file) as u32);
            let Some(from_sq) = selected_square.get_untracked() else {
                return;
            };
            if !valid_move_targets.get_untracked().contains(&this_square) {
                return;
            }
            use shakmaty::{Position as _, Role};
            let legal = position.get_untracked().legal_moves();
            if let Some(m) = legal.iter().find(|m| {
                m.from() == Some(from_sq)
                    && m.to() == this_square
                    && m.promotion().map_or(true, |r| r == Role::Queen)
            }) {
                let m = m.clone();
                selected_square.set(None);
                let _ = tx.try_send(GameClientMessage::MoveMade {
                    uci: m.to_uci(shakmaty::CastlingMode::Standard).to_string(),
                });
            }
        }
    };
    #[cfg(not(feature = "hydrate"))]
    let on_click = |_: leptos::ev::MouseEvent| {};

    let this_sq = Square::new((rank * 8 + file) as u32);
    let is_highlighted = move || {
        selected_square.get() == Some(this_sq)
            || last_move
                .get()
                .is_some_and(|(f, t)| f == this_sq || t == this_sq)
    };

    view! {
        <div
            class="relative w-full h-full"
            class:bg-white=move || (rank + file) % 2 == 0 && !is_highlighted()
            class:bg-green-800=move || (rank + file) % 2 != 0 && !is_highlighted()
            class:bg-green-300=move || (rank + file) % 2 == 0 && is_highlighted()
            class:bg-green-600=move || (rank + file) % 2 != 0 && is_highlighted()
            data-square=format!("{}{}", file_to_char(file), rank_to_char(rank))
            on:click=on_click
        >
            <Show
                when=move || valid_move_targets.get().contains(&Square::new((rank * 8 + file) as u32))
            >
                {move || if piece.get().is_some() {
                    view! {
                        <div class="absolute inset-0 rounded-full ring-[6px] ring-inset ring-black opacity-20 pointer-events-none z-10"></div>
                    }.into_any()
                } else {
                    view! {
                        <div class="absolute inset-0 flex items-center justify-center pointer-events-none z-10">
                            <div class="w-1/3 h-1/3 rounded-full bg-black opacity-20"></div>
                        </div>
                    }.into_any()
                }}
            </Show>
            {move || image_path.get().map(|src| view! {
                <img
                    src={src}
                    node_ref=el
                    draggable="false"
                    class="w-full h-full cursor-grab"
                    class:cursor-grabbing=move || is_dragging.get()
                    style=move || if is_dragging.get() {
                        let (w, h) = drag_size.get();
                        format!("position: fixed; {} pointer-events: none; width: {}px; height: {}px; z-index: 50;", style.get(), w, h)
                    } else {
                        String::new()
                    }
                />
            })}
            <Show when=move || perspective.get() == BoardPerspective::White && rank == 0 || perspective.get() == BoardPerspective::Black && rank == 7>
                <span
                    class="absolute bottom-0 left-0.5 font-bold text-sm"
                    class:text-white=move || (rank + file) % 2 != 0
                    class:text-green-800=move || (rank + file) % 2 == 0
                >{file_to_char(file)}</span>
            </Show>
            <Show when=move || perspective.get() == BoardPerspective::White && file == 7 || perspective.get() == BoardPerspective::Black && file == 0>
                <span
                    class="absolute top-0 right-0.5 font-bold text-sm"
                    class:text-white=move || (rank + file) % 2 != 0
                    class:text-green-800=move || (rank + file) % 2 == 0
                >{rank_to_char(rank)}</span>
            </Show>
        </div>
    }
}
