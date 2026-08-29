#![allow(dead_code, unused_imports)]

//! Headless streaming benchmark for the production transcript reducer, row projection,
//! list synchronization, and GPUI render/layout path.

use std::{
    collections::HashMap,
    io::{self, Write as _},
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{IntoElement as _, Render, TestApp, WeakEntity};
use serde_json::{Value, json};

mod app {
    #[derive(Clone, Debug, Eq, PartialEq, gpui::Action)]
    #[action(namespace = farcaster_bench, no_json)]
    pub(crate) struct RemoveProject {
        pub(crate) path: std::path::PathBuf,
    }

    pub(crate) struct FarcasterApp;

    impl FarcasterApp {
        pub(crate) fn jump_to_latest(&mut self, _: &mut gpui::Context<Self>) {}

        pub(crate) fn set_transcript_item_expanded(
            &mut self,
            _: usize,
            _: bool,
            _: &mut gpui::Context<Self>,
        ) {
        }

        pub(crate) fn open_file_editor_at_line(
            &mut self,
            _: std::path::PathBuf,
            _: Option<u64>,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) {
        }

        pub(crate) fn open_image_preview(
            &mut self,
            _: std::sync::Arc<gpui::Image>,
            _: usize,
            _: usize,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) {
        }
    }
}

#[path = "../src/assets.rs"]
mod assets;
#[path = "../src/conversation.rs"]
mod conversation;
#[path = "../src/performance.rs"]
mod performance;
#[path = "../src/persistent_vec.rs"]
mod persistent_vec;
#[path = "../src/primitives/mod.rs"]
mod primitives;
#[path = "../src/protocol.rs"]
mod protocol;
#[path = "../src/theme.rs"]
mod theme;
#[path = "../src/tool_changes.rs"]
mod tool_changes;
#[path = "../src/transcript.rs"]
mod transcript;
#[path = "../src/transcript_attachments.rs"]
mod transcript_attachments;
#[path = "../src/transcript_list.rs"]
mod transcript_list;
#[path = "../src/transcript_markdown.rs"]
mod transcript_markdown;

const WARMUP_FRAMES: usize = 10;
const SAMPLE_FRAMES: usize = 60;
const HISTORY_SIZES: [usize; 3] = [200, 2_000, 10_000];

#[derive(Clone, Copy, Default)]
struct FrameSample {
    reduce: Duration,
    project_and_sync: Duration,
    draw: Duration,
    total: Duration,
}

struct TranscriptBenchView {
    list: transcript_list::TranscriptListState,
    rows: Arc<persistent_vec::PersistentVec<transcript::TranscriptRow>>,
    conversation: Arc<conversation::ConversationState>,
    markdown_cache: transcript_markdown::TranscriptMarkdownCache,
}

impl TranscriptBenchView {
    fn new(message_count: usize) -> Self {
        let mut conversation = conversation::ConversationState::default();
        conversation.replace_history(&mock_history(message_count));
        conversation.reduce(&json!({
            "type": "message_start",
            "message": {"role": "assistant", "content": []}
        }));
        conversation.reduce(&json!({
            "type": "message_update",
            "assistantMessageEvent": {"type": "text_start", "contentIndex": 0}
        }));

        let conversation = Arc::new(conversation);
        let rows = Arc::new(transcript::project_rows(&conversation.items));
        let list = transcript_list::TranscriptListState::new();
        list.splice_with_size_hints(
            0..0,
            rows.iter()
                .map(|row| transcript::estimated_row_height(*row, &conversation.items)),
        );
        list.scroll_to_end();

        Self {
            list,
            rows,
            conversation,
            markdown_cache: transcript_markdown::TranscriptMarkdownCache::default(),
        }
    }

    fn apply_stream_event(&mut self, event: &Value) -> (Duration, Duration) {
        let previous = self.conversation.clone();
        let reduce_started = Instant::now();
        let changed_from = {
            let conversation = Arc::make_mut(&mut self.conversation);
            let (changed_from, _) = conversation.reduce_deferred_with_change(event);
            conversation.flush_live_projection();
            changed_from
        };
        let reduce = reduce_started.elapsed();

        let projection_started = Instant::now();
        let update = transcript::update_rows_incremental(
            &self.rows,
            &previous.items,
            &self.conversation.items,
            changed_from,
        );
        let _changed = update.apply(&self.list, &mut self.rows, &self.conversation.items);
        (reduce, projection_started.elapsed())
    }
}

impl Render for TranscriptBenchView {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        transcript::render(
            &self.list,
            transcript::TranscriptViewport {
                following: true,
                unseen: 0,
                tail_reserve: transcript::tail_reserve(window.viewport_size().height),
            },
            self.rows.clone(),
            self.conversation.clone(),
            HashMap::new(),
            self.markdown_cache.clone(),
            WeakEntity::<app::FarcasterApp>::new_invalid(),
        )
        .into_any_element()
    }
}

fn main() -> io::Result<()> {
    let stdout = io::stdout();
    let mut output = io::BufWriter::new(stdout.lock());
    writeln!(
        output,
        "history_items\tprojected_rows\tstage\tmedian_us\tp95_us\tmax_us"
    )?;
    for history_size in HISTORY_SIZES {
        run_scenario(history_size, &mut output)?;
    }
    output.flush()
}

fn run_scenario(history_size: usize, output: &mut impl io::Write) -> io::Result<()> {
    let platform = gpui_platform::current_platform(true);
    let mut app =
        TestApp::with_text_system_and_assets(platform.text_system(), Arc::new(assets::AppAssets));
    app.update(|cx| {
        gpui_component::init(cx);
        assert!(
            assets::AppAssets.load_fonts(cx).is_ok(),
            "benchmark fonts should load"
        );
        theme::install_component_theme(cx);
    });
    let mut window = app.open_window(|_, _| TranscriptBenchView::new(history_size));
    window.draw();

    let event = json!({
        "type": "message_update",
        "assistantMessageEvent": {
            "type": "text_delta",
            "contentIndex": 0,
            "delta": " streamed-token"
        }
    });
    let mut samples = Vec::with_capacity(SAMPLE_FRAMES);
    for frame in 0..WARMUP_FRAMES + SAMPLE_FRAMES {
        let total_started = Instant::now();
        let (reduce, project_and_sync) =
            window.update(|view, _, _| view.apply_stream_event(&event));
        let draw_started = Instant::now();
        window.draw();
        let sample = FrameSample {
            reduce,
            project_and_sync,
            draw: draw_started.elapsed(),
            total: total_started.elapsed(),
        };
        if frame >= WARMUP_FRAMES {
            samples.push(sample);
        }
    }
    let projected_rows = window.read(|view, _| view.rows.len());
    for (name, durations) in [
        (
            "reduce",
            samples.iter().map(|sample| sample.reduce).collect(),
        ),
        (
            "project+sync",
            samples
                .iter()
                .map(|sample| sample.project_and_sync)
                .collect(),
        ),
        ("draw", samples.iter().map(|sample| sample.draw).collect()),
        ("total", samples.iter().map(|sample| sample.total).collect()),
    ] {
        write_summary(output, history_size, projected_rows, name, durations)?;
    }
    Ok(())
}

fn mock_history(message_count: usize) -> Vec<Value> {
    (0..message_count)
        .map(|index| {
            let role = if index % 2 == 0 { "user" } else { "assistant" };
            json!({
                "role": role,
                "content": [{
                    "type": "text",
                    "text": format!(
                        "Message {index}: inspect the current implementation and report concrete evidence."
                    )
                }]
            })
        })
        .collect()
}

fn write_summary(
    output: &mut impl io::Write,
    history_items: usize,
    projected_rows: usize,
    stage: &str,
    mut samples: Vec<Duration>,
) -> io::Result<()> {
    samples.sort_unstable();
    let median = percentile(&samples, 50);
    let p95 = percentile(&samples, 95);
    let maximum = samples.last().copied().unwrap_or_default();
    writeln!(
        output,
        "{history_items}\t{projected_rows}\t{stage}\t{:.2}\t{:.2}\t{:.2}",
        micros(median),
        micros(p95),
        micros(maximum),
    )
}

fn percentile(samples: &[Duration], percentile: usize) -> Duration {
    if samples.is_empty() {
        return Duration::default();
    }
    let index = samples
        .len()
        .saturating_mul(percentile)
        .div_ceil(100)
        .saturating_sub(1)
        .min(samples.len() - 1);
    samples[index]
}

fn micros(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000_000.0
}
