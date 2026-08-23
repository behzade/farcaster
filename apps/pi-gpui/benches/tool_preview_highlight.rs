//! Preserves the isolated cost of the synchronous syntax-highlighting path that
//! compact embedded edit/write summaries intentionally avoid.

#![allow(dead_code, unused_imports)]

use std::{
    hint::black_box,
    io::{self, Write as _},
    time::Instant,
};

#[path = "../src/performance.rs"]
mod performance;
#[path = "../src/syntax_highlight.rs"]
mod syntax_highlight;

const SAMPLE_COUNT: usize = 20;

struct Corpus {
    language: &'static str,
    source: &'static str,
}

fn main() -> io::Result<()> {
    // The removed previews highlighted at most 12 lines. Keep each corpus at that
    // boundary so fixes can still measure initialization/parsing independently.
    let corpora = [
        Corpus {
            language: "rs",
            source: "use std::sync::Arc;\n\npub struct Preview {\n    rows: Arc<Vec<String>>,\n}\n\nimpl Preview {\n    pub fn len(&self) -> usize {\n        self.rows.len()\n    }\n}\n",
        },
        Corpus {
            language: "ts",
            source: "type Preview = {\n  rows: string[];\n};\n\nexport function render(value: Preview): string {\n  return value.rows\n    .filter(Boolean)\n    .map((row, index) => `${index}: ${row}`)\n    .join('\\n');\n}\n",
        },
        Corpus {
            language: "json",
            source: "{\n  \"name\": \"preview\",\n  \"enabled\": true,\n  \"rows\": [\n    \"one\",\n    \"two\",\n    \"three\"\n  ],\n  \"limit\": 12\n}\n",
        },
    ];

    let stdout = io::stdout();
    let mut output = io::BufWriter::new(stdout.lock());
    writeln!(
        output,
        "language\tbytes\tlines\tcold_us\twarm_median_us\twarm_p95_us\twarm_max_us"
    )?;
    for corpus in corpora {
        run_case(&mut output, corpus)?;
    }
    output.flush()
}

fn run_case(output: &mut impl io::Write, corpus: Corpus) -> io::Result<()> {
    let lines = corpus.source.lines().collect::<Vec<_>>();
    let cold_started = Instant::now();
    black_box(syntax_highlight::highlight_lines(
        black_box(&lines),
        black_box(corpus.language),
    ));
    let cold_us = cold_started.elapsed().as_secs_f64() * 1_000_000.0;

    let mut warm_samples = Vec::with_capacity(SAMPLE_COUNT);
    for _ in 0..SAMPLE_COUNT {
        let started = Instant::now();
        black_box(syntax_highlight::highlight_lines(
            black_box(&lines),
            black_box(corpus.language),
        ));
        warm_samples.push(started.elapsed().as_secs_f64() * 1_000_000.0);
    }
    warm_samples.sort_by(f64::total_cmp);

    writeln!(
        output,
        "{}\t{}\t{}\t{:.2}\t{:.2}\t{:.2}\t{:.2}",
        corpus.language,
        corpus.source.len(),
        lines.len(),
        cold_us,
        percentile(&warm_samples, 50),
        percentile(&warm_samples, 95),
        warm_samples.last().copied().unwrap_or_default(),
    )
}

fn percentile(samples: &[f64], percentile: usize) -> f64 {
    let index = samples
        .len()
        .saturating_mul(percentile)
        .div_ceil(100)
        .saturating_sub(1)
        .min(samples.len().saturating_sub(1));
    samples.get(index).copied().unwrap_or_default()
}
