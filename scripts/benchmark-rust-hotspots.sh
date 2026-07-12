#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

echo "Running informational Rust hotspot benchmarks in release mode"
cargo test --release -p stratum-dsp benchmark_generated_dsp_pipeline -- --ignored --nocapture --test-threads=1
cargo test --release -p reklawdbox benchmark_batch_audio_cache_reads -- --ignored --nocapture --test-threads=1
