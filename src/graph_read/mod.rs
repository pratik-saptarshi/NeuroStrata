//! Graph-read capability: deterministic, zero-embedding lookups beside the
//! vector store (`VectorStore`). See the plan Task 3 / spec §4 for the
//! `memories_for_file` contract, and Task 4 / spec §4.2 for the bounded
//! `related` traversal.
pub mod memories_for_file;
pub mod related;

/// Deterministic hybrid (graph + vector) lookups beside the vector store.
pub mod hybrid;

/// Bounded time-window reasoning over relationship validity windows.
pub mod temporal;

/// Deterministic weighted ordering of `related` neighbours.
pub mod prioritized;

/// Cross-namespace boundary policy for `related` hops.
pub mod traversal;

/// Bounded, transient domain/tag propagation from seed nodes.
pub mod propagate;

/// Single-call 360-degree Ontology Object View hub (`neurostrata_object_view`).
pub mod object_view;

/// Compound single-call Code Finder, Blast-Radius & Governance Locator (`neurostrata_locate`).
pub mod locate;

/// Dependency-ordered 4-stop Guided Diff Tour (`neurostrata_diff_tour`).
pub mod diff_tour;
