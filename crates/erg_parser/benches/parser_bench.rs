//! Stage-isolated benchmarks for the Erg parser front end.
//!
//! Measures, across small/medium/large inputs:
//!   * `lex`        — tokenization only
//!   * `parse`      — parsing only (lexing happens in the untimed setup)
//!   * `desugar`    — desugaring only (parsing happens in the untimed setup)
//!   * `end_to_end` — `SimpleParser::parse`, i.e. lex + parse + desugar combined
//!
//! Run with `cargo bench -p erg_parser`.
//!
//! `harness = false` (see Cargo.toml): we supply `main` ourselves and run the
//! whole criterion session on a 16 MB-stack thread, because the recursive
//! descent parser overflows the default main-thread stack on large inputs (the
//! `large_thread` feature only enlarges `exec_new_thread`-spawned threads).

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput};

use erg_parser::desugar::Desugarer;
use erg_parser::lex::Lexer;
use erg_parser::parse::{Parser, SimpleParser};

/// (label, source) pairs embedded at compile time for reproducibility.
/// Paths are relative to this file (`crates/erg_parser/benches/`).
const INPUTS: &[(&str, &str)] = &[
    ("small", include_str!("../tests/containers.er")), // 50 lines, container syntax
    (
        "medium",
        include_str!("../../erg_compiler/lib/pystd/builtins.d.er"),
    ), // 242 lines, declarations
    ("large", include_str!("../../../tests/should_ok/long.er")), // 699 lines, many definitions
];

fn bench_lex(c: &mut Criterion) {
    let mut group = c.benchmark_group("lex");
    for &(name, src) in INPUTS {
        group.throughput(Throughput::Bytes(src.len() as u64));
        group.bench_function(BenchmarkId::from_parameter(name), |b| {
            b.iter_batched(
                || src.to_string(),
                |code| Lexer::from_str(code).lex(),
                BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

fn bench_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse");
    for &(name, src) in INPUTS {
        group.throughput(Throughput::Bytes(src.len() as u64));
        group.bench_function(BenchmarkId::from_parameter(name), |b| {
            b.iter_batched(
                || {
                    Lexer::from_str(src.to_string())
                        .lex()
                        .unwrap_or_else(|_| panic!("lex failed for {name}"))
                },
                |ts| Parser::new(ts).parse(),
                BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

fn bench_desugar(c: &mut Criterion) {
    let mut group = c.benchmark_group("desugar");
    for &(name, src) in INPUTS {
        let ts = Lexer::from_str(src.to_string())
            .lex()
            .unwrap_or_else(|_| panic!("lex failed for {name}"));
        // the undesugared module — `SimpleParser::parse` would desugar already,
        // so we use the lower-level `Parser` to isolate the desugar cost.
        let module = match Parser::new(ts).parse() {
            Ok(art) => art.ast,
            Err(_) => panic!("parse failed for {name}"),
        };
        group.throughput(Throughput::Bytes(src.len() as u64));
        group.bench_function(BenchmarkId::from_parameter(name), |b| {
            b.iter_batched(
                || module.clone(),
                |m| Desugarer::new().desugar(m),
                BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

fn bench_end_to_end(c: &mut Criterion) {
    let mut group = c.benchmark_group("end_to_end");
    for &(name, src) in INPUTS {
        group.throughput(Throughput::Bytes(src.len() as u64));
        group.bench_function(BenchmarkId::from_parameter(name), |b| {
            b.iter_batched(
                || src.to_string(),
                SimpleParser::parse,
                BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

fn run() {
    let mut c = Criterion::default().configure_from_args();
    bench_lex(&mut c);
    bench_parse(&mut c);
    bench_desugar(&mut c);
    bench_end_to_end(&mut c);
    c.final_summary();
}

fn main() {
    // drive the whole criterion run on a deep stack (see module docs).
    std::thread::Builder::new()
        .name("parser_bench".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(run)
        .expect("failed to spawn bench thread")
        .join()
        .expect("bench thread panicked");
}
