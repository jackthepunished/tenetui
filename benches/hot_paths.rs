//! Criterion benches for the hot paths: line diffing (ghost bookkeeping),
//! snapshot materialization (cache miss vs. hit), and syntax highlighting.
//!
//! The syntax bench is the spike that decides whether highlighting can stay on
//! the interactive path or must move to the prefetch thread (docs/m4-plan.md,
//! item 1). Run with `cargo bench`.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use git2::{Repository, Signature};
use std::hint::black_box;
use tenetui::diff;
use tenetui::repo::{self, SnapshotCache};
use tenetui::syntax::Highlighter;
use tenetui::theme::{ColorDepth, Theme};

/// Generate `lines` of Rust-ish source, so the syntax bench exercises real
/// tokenizing rather than one repeated token.
fn rust_source(lines: usize) -> String {
    let mut s = String::new();
    for i in 0..lines {
        match i % 5 {
            0 => s.push_str(&format!("// comment describing item {i}\n")),
            1 => s.push_str(&format!("fn function_{i}(arg: usize) -> usize {{\n")),
            2 => s.push_str(&format!("    let value = \"string literal {i}\";\n")),
            3 => s.push_str(&format!("    let n = {i} + 42 * 7;\n")),
            _ => s.push_str("}\n"),
        }
    }
    s
}

fn bench_diff(c: &mut Criterion) {
    let mut group = c.benchmark_group("diff/compute_ghosts");
    for lines in [500usize, 5_000, 20_000] {
        let old = rust_source(lines);
        // Change ~1% of lines so the diff has real work but isn't a full rewrite.
        let new: String = old
            .lines()
            .enumerate()
            .map(|(i, l)| {
                if i % 100 == 0 {
                    format!("{l} // edited\n")
                } else {
                    format!("{l}\n")
                }
            })
            .collect();
        let existing = std::collections::HashMap::new();
        group.bench_with_input(BenchmarkId::from_parameter(lines), &lines, |b, _| {
            b.iter(|| diff::compute_ghosts(black_box(&old), black_box(&new), black_box(&existing)));
        });
    }
    group.finish();
}

fn bench_highlight(c: &mut Criterion) {
    let highlighter = Highlighter::new();
    let theme = Theme {
        depth: ColorDepth::TrueColor,
    };
    let mut group = c.benchmark_group("syntax/highlight");
    for lines in [500usize, 5_000, 20_000] {
        let source = rust_source(lines);
        group.bench_with_input(BenchmarkId::from_parameter(lines), &lines, |b, _| {
            b.iter(|| highlighter.highlight(black_box(&source), black_box("bench.rs"), &theme));
        });
    }
    group.finish();
}

/// Build a temp repo whose `f.txt` changes across `commits` commits; return the
/// repo dir (kept alive by the returned `TempDir`) and the ordered oids.
fn build_repo(commits: usize) -> (tempfile::TempDir, Vec<git2::Oid>) {
    let tmp = tempfile::tempdir().unwrap();
    let repo = Repository::init(tmp.path()).unwrap();
    let sig = Signature::now("Bench", "bench@example.com").unwrap();
    let mut body = String::new();
    for i in 0..commits {
        body.push_str(&format!("line {i}\n"));
        std::fs::write(tmp.path().join("f.txt"), &body).unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<_> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, &format!("c{i}"), &tree, &parents)
            .unwrap();
    }
    let oids = repo::timeline(&repo, "f.txt")
        .unwrap()
        .iter()
        .map(|c| c.oid)
        .collect();
    (tmp, oids)
}

fn bench_snapshot(c: &mut Criterion) {
    let (tmp, oids) = build_repo(200);
    let repo = Repository::discover(tmp.path()).unwrap();
    let mid = oids[oids.len() / 2];

    let mut group = c.benchmark_group("snapshot");
    // Cold: a fresh cache every iteration → always a git2 tree lookup.
    group.bench_function("miss", |b| {
        b.iter(|| {
            let mut cache = SnapshotCache::new(256);
            cache.get_or_fetch(&repo, black_box(mid), "f.txt").unwrap()
        });
    });
    // Warm: prewarmed cache → the scrub hot path (must be well under 16ms).
    let mut warm = SnapshotCache::new(256);
    warm.get_or_fetch(&repo, mid, "f.txt").unwrap();
    group.bench_function("hit", |b| {
        b.iter(|| warm.get_or_fetch(&repo, black_box(mid), "f.txt").unwrap());
    });
    group.finish();
}

criterion_group!(benches, bench_diff, bench_highlight, bench_snapshot);
criterion_main!(benches);
