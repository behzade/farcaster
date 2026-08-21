use std::{
    hint::black_box,
    io::{self, Write as _},
    time::Instant,
};

use pi_gpui::diff_plan::{DiffLayout, DiffPlanOptions, plan_patch};

const SMALL_PATCH: &str = include_str!("../tests/fixtures/diff_plan/small-pi-ai.patch");
const MEDIUM_PATCH: &str = include_str!("../tests/fixtures/diff_plan/medium-pi-web.patch");
const STRESS_PATCH: &str = include_str!("../tests/fixtures/diff_plan/stress-pierre.patch");
const SAMPLE_COUNT: usize = 20;
const TARGET_SAMPLE_NANOS: f64 = 5_000_000.0;

struct Corpus {
    name: &'static str,
    patch: String,
}

fn main() -> io::Result<()> {
    let corpora = [
        Corpus {
            name: "small-real-4.8KB",
            patch: SMALL_PATCH.to_owned(),
        },
        Corpus {
            name: "medium-real-15KB",
            patch: MEDIUM_PATCH.to_owned(),
        },
        Corpus {
            name: "stress-real-403KB",
            patch: STRESS_PATCH.to_owned(),
        },
    ];
    let stdout = io::stdout();
    let mut output = io::BufWriter::new(stdout.lock());
    writeln!(
        output,
        "corpus\tlayout\tlimit\tintraline\tbytes\tbatch\tmedian_ns\tp95_ns\tmin_ns\tmax_ns\tMiB/s\trows\tcells\tspans\ttext_bytes"
    )?;
    for corpus in &corpora {
        for layout in [DiffLayout::Unified, DiffLayout::Split] {
            for maximum in [None, Some(12)] {
                run_case(&mut output, corpus, layout, maximum, true)?;
            }
        }
    }
    run_case(&mut output, &corpora[2], DiffLayout::Split, None, false)?;
    output.flush()
}

fn run_case(
    output: &mut impl io::Write,
    corpus: &Corpus,
    layout: DiffLayout,
    maximum: Option<usize>,
    intraline: bool,
) -> io::Result<()> {
    let mut options = DiffPlanOptions::new(layout);
    options.max_rows_per_file = maximum;
    options.intraline_changes = intraline;
    let initial = planned_or_panic(&corpus.patch, options);
    let metrics = plan_metrics(&initial);
    black_box(initial);
    for _ in 0..3 {
        black_box(planned_or_panic(black_box(&corpus.patch), options));
    }

    let calibration_started = Instant::now();
    black_box(planned_or_panic(black_box(&corpus.patch), options));
    let calibration_nanos = calibration_started.elapsed().as_nanos().max(1) as f64;
    let batch = (TARGET_SAMPLE_NANOS / calibration_nanos)
        .ceil()
        .clamp(1.0, 10_000.0) as usize;
    let mut samples = Vec::with_capacity(SAMPLE_COUNT);
    for _ in 0..SAMPLE_COUNT {
        let started = Instant::now();
        for _ in 0..batch {
            let plan = planned_or_panic(black_box(&corpus.patch), black_box(options));
            black_box(plan);
        }
        samples.push(started.elapsed().as_nanos() as f64 / batch as f64);
    }
    samples.sort_by(f64::total_cmp);
    let minimum = samples[0];
    let median = percentile(&samples, 50);
    let p95 = percentile(&samples, 95);
    let maximum_nanos = samples[samples.len() - 1];
    let bytes_per_second = corpus.patch.len() as f64 / (median / 1_000_000_000.0);
    writeln!(
        output,
        "{}\t{:?}\t{}\t{}\t{}\t{}\t{:.0}\t{:.0}\t{:.0}\t{:.0}\t{:.2}\t{}\t{}\t{}\t{}",
        corpus.name,
        layout,
        maximum.map_or("all".to_owned(), |maximum| maximum.to_string()),
        intraline,
        corpus.patch.len(),
        batch,
        median,
        p95,
        minimum,
        maximum_nanos,
        bytes_per_second / (1024.0 * 1024.0),
        metrics.rows,
        metrics.cells,
        metrics.spans,
        metrics.text_bytes,
    )
}

#[derive(Default)]
struct PlanMetrics {
    rows: usize,
    cells: usize,
    spans: usize,
    text_bytes: usize,
}

fn plan_metrics(plan: &pi_gpui::diff_plan::DiffRenderPlan) -> PlanMetrics {
    let mut metrics = PlanMetrics::default();
    for file in &plan.files {
        metrics.rows += file.rows.len();
        for row in &file.rows {
            let pi_gpui::diff_plan::DiffPlanRow::Line(line) = row else {
                continue;
            };
            for cell in [line.old.as_ref(), line.new.as_ref()].into_iter().flatten() {
                metrics.cells += 1;
                metrics.spans += cell.changed.len();
                metrics.text_bytes += cell.text.len();
            }
        }
    }
    metrics
}

fn percentile(samples: &[f64], percentile: usize) -> f64 {
    let index = samples
        .len()
        .saturating_mul(percentile)
        .div_ceil(100)
        .saturating_sub(1)
        .min(samples.len().saturating_sub(1));
    samples[index]
}

fn planned_or_panic(patch: &str, options: DiffPlanOptions) -> pi_gpui::diff_plan::DiffRenderPlan {
    match plan_patch(patch, options) {
        Ok(plan) => plan,
        Err(error) => panic!("benchmark fixture must parse: {error}"),
    }
}
