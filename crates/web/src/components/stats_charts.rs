//! The two charts on the stats page.
//!
//! Both are `leptos-chartistry`, which draws SVG from Leptos signals — so
//! the series are ordinary reactive data and there is no canvas, no JS
//! bridge and nothing to marshal across it. Colours are passed in as
//! literals matching the design tokens rather than read from CSS, because
//! the chart renders its own `<svg>` attributes and cannot resolve a
//! `var(--accent)` through them.

use leptos::prelude::*;
use leptos_chartistry::*;
use shared::{ColorRecord, RatingPoint};

const WIN_COLOUR: &str = "#10b981";
const DRAW_COLOUR: &str = "#71717a";
const LOSS_COLOUR: &str = "#ef4444";

/// Tick labels for the time axis, formatted with `fmt`.
///
/// A single period, not the generator's own choice of several: left to itself
/// it mixed periods and labelled a two-month span with weekday names ("Sat
/// Sun Mon Sep Wed"), and allowing Day *and* Month then injected a
/// month-boundary tick among evenly spaced day ticks so two labels collided
/// ("1 Sep6 Sep"). One period samples uniformly at any span, and a rating
/// history is measured in days to years, never hours.
fn date_ticks(fmt: &'static str) -> TickLabels<chrono::DateTime<chrono::Utc>> {
    TickLabels::from_generator(Timestamps::from_periods(&[Period::Day][..]))
        .with_format(move |tick: &chrono::DateTime<chrono::Utc>, _| tick.format(fmt).to_string())
}

/// One plotted rating, in the shape chartistry wants: an x it can place on a
/// time axis and a y it can scale.
#[derive(Clone, PartialEq)]
struct Point {
    at: chrono::DateTime<chrono::Utc>,
    rating: f64,
}

impl Point {
    fn from_shared(p: &RatingPoint) -> Option<Self> {
        Some(Self {
            at: chrono::DateTime::from_timestamp_millis(p.at)?,
            rating: p.rating as f64,
        })
    }
}

/// Rating over time for one category.
///
/// Fewer than two points cannot be a line, so that renders as a note rather
/// than an empty axis — which is the normal state for a category someone has
/// never played rated.
#[component]
pub fn RatingChart(#[prop(into)] points: Signal<Vec<RatingPoint>>) -> impl IntoView {
    let data = Signal::derive(move || {
        points
            .get()
            .iter()
            .filter_map(Point::from_shared)
            .collect::<Vec<_>>()
    });
    let enough = Signal::derive(move || data.get().len() >= 2);

    let series = Series::new(|p: &Point| p.at)
        // 0x4d7c4f is `--accent` from `style/main.css`. Spelled as
        // components because the chart writes its own SVG attributes and
        // cannot resolve a CSS variable through them.
        .line(
            Line::new(|p: &Point| p.rating)
                .with_width(5.0)
                .with_name("Rating")
                .with_colour(Colour::from_rgb(16, 185, 129)),
        );

    view! {
        <Show
            when=move || enough.get()
            fallback=move || view! {
                <p class="text-sm text-zinc-500 py-8 text-center">
                    "Not enough rated games in this time control yet to chart a trend."
                </p>
            }
        >
            <div class="h-64 w-full">
                <Chart
                    aspect_ratio=AspectRatio::from_env()
                    series=series.clone()
                    data=data
                    left=TickLabels::aligned_floats()
                    bottom=date_ticks("%-d %b").with_min_chars(7)
                    inner=vec![
                        AxisMarker::left_edge().into_inner(),
                        AxisMarker::bottom_edge().into_inner(),
                        XGridLine::default().into_inner(),
                        YGridLine::default().into_inner(),
                        XGuideLine::over_data().into_inner(),
                    ]
                    // The tooltip carries its own x formatter, separate
                    // from the axis above, and its default prints a full
                    // timestamp down to milliseconds. Given the year here,
                    // since the tooltip is where you look to place a point
                    // precisely.
                    tooltip=Tooltip::new(
                        TooltipPlacement::LeftCursor,
                        date_ticks("%-d %b %Y"),
                        TickLabels::aligned_floats(),
                    )
                />
            </div>
        </Show>
    }
}

/// Win/draw/loss as White against the same as Black, so the two are directly
/// comparable. Rendered as proportional bars rather than a chartistry series:
/// it is two rows of three numbers, and a bar chart of six values carries no
/// more information than the bars themselves while costing an axis, a legend
/// and a tooltip to read.
#[component]
pub fn ColorSplit(white: ColorRecord, black: ColorRecord) -> impl IntoView {
    view! {
        <div class="flex flex-col gap-5">
            <ColorBar label="As White" record=white/>
            <ColorBar label="As Black" record=black/>
        </div>
    }
}

#[component]
fn ColorBar(label: &'static str, record: ColorRecord) -> impl IntoView {
    let total = record.total();
    // Widths as percentages of the row, so the three segments always fill it
    // exactly and a lopsided record reads at a glance.
    let pct = move |n: i64| {
        if total == 0 {
            0.0
        } else {
            n as f64 / total as f64 * 100.0
        }
    };
    let score = record.score_rate();
    // Hoisted rather than written inline: a bare `>` inside a `view!`
    // attribute expression closes the tag, and parenthesising it to get
    // past that then trips `unused_parens`.
    let has_games = move || total > 0;

    view! {
        <div class="flex flex-col gap-2">
            <div class="flex items-baseline justify-between gap-3">
                <span class="eyebrow-sm text-zinc-500">{label}</span>
                <span class="text-sm text-zinc-400">
                    {match score {
                        // Draws count as half, the usual convention, so the
                        // two colours stay comparable when their draw rates
                        // differ.
                        Some(s) => format!("{s:.1}% score"),
                        None => "\u{2014}".to_string(),
                    }}
                </span>
            </div>
            <Show
                when=has_games
                fallback=|| view! {
                    <>
                        <div class="h-2.5 rounded-full bg-zinc-800"></div>
                        <span class="text-xs text-zinc-600">"No games yet"</span>
                    </>
                }
            >
                <div class="flex h-2.5 w-full overflow-hidden rounded-full bg-zinc-800">
                    <div style=format!("width:{:.4}%;background-color:{WIN_COLOUR}", pct(record.wins))></div>
                    <div style=format!("width:{:.4}%;background-color:{DRAW_COLOUR}", pct(record.draws))></div>
                    <div style=format!("width:{:.4}%;background-color:{LOSS_COLOUR}", pct(record.losses))></div>
                </div>
                <div class="flex items-center gap-4 text-xs text-zinc-500">
                    <Tally colour=WIN_COLOUR label="won" n=record.wins/>
                    <Tally colour=DRAW_COLOUR label="drawn" n=record.draws/>
                    <Tally colour=LOSS_COLOUR label="lost" n=record.losses/>
                    <span class="ml-auto text-zinc-600">{total} " games"</span>
                </div>
            </Show>
        </div>
    }
}

#[component]
fn Tally(colour: &'static str, label: &'static str, n: i64) -> impl IntoView {
    view! {
        <span class="flex items-center gap-1.5">
            <span
                class="inline-block w-2 h-2 rounded-full"
                style=format!("background-color:{colour}")
            ></span>
            <span class="text-zinc-300 font-semibold">{n}</span>
            {label}
        </span>
    }
}
