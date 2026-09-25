# Changelog

All notable changes to `nexus-rs` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-09-25

### Added

- Initial release of `nexus-rs`.
- Multi-engine fanout with adaptive quorum early-exit (DuckDuckGo, Bing, Yahoo, Mojeek, GoogleWML).
- SSRF-hardened egress with DNS pre-flight, socket pinning, and manual redirect validation loop.
- Sliding-window passage chunking with configurable size and overlap.
- Tri-mode ranking: Sparse (BM25), Dense (cosine similarity), Hybrid (RRF fusion with k=60).
- 2-stage re-ranking with consensus corroboration multiplier.
- `TextEmbedder` trait for decoupled dense/hybrid ranking.
- Strongly typed `NexusError` enum covering the full failure surface.
- Granular pipeline telemetry (`NexusSearchMetrics`).
- Fluent `NexusSearchBuilder` with validation.
- Examples: `basic_search` and `hybrid_search`.
