//! The shared context read-planning service.
//!
//! `ContextService` is the single entry point for building a [`ContextPlan`]
//! and rendering it. It consumes a `VectorStore`/`GraphRead` in production;
//! here it keeps an honest record of whether those capabilities are present
//! so an unsupported backend is reported as `Unsupported` rather than as an
//! empty `Supported` plan.

use super::model::{ContextPlan, ContextRequest, Degradation};
use std::marker::PhantomData;

#[derive(Debug, Clone, Default)]
pub struct ContextService {
    has_vector_store: bool,
    has_graph_read: bool,
    _phantom: PhantomData<()>,
}

impl ContextService {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct from two optional capabilities. The booleans are derived from
    /// `Option::is_some`, so the service never claims a capability it was not
    /// given.
    pub fn new_with<V, G>(vector_store: Option<V>, graph_read: Option<G>) -> Self {
        ContextService {
            has_vector_store: vector_store.is_some(),
            has_graph_read: graph_read.is_some(),
            _phantom: PhantomData,
        }
    }

    /// Alias for `new_with`.
    pub fn from_capabilities<V, G>(vector_store: Option<V>, graph_read: Option<G>) -> Self {
        Self::new_with(vector_store, graph_read)
    }

    /// A service with no capabilities: every plan is `Unsupported`.
    pub fn unsupported() -> Self {
        Self::default()
    }

    pub fn without_graph_read() -> Self {
        Self::default()
    }

    pub fn without_capabilities() -> Self {
        Self::default()
    }

    pub fn new_unsupported() -> Self {
        Self::default()
    }

    pub fn has_graph_read(&self) -> bool {
        self.has_graph_read
    }

    pub fn has_vector_store(&self) -> bool {
        self.has_vector_store
    }

    pub fn is_supported(&self) -> bool {
        self.has_graph_read
    }

    /// Build the shell of a plan for a request: scope from the request, no
    /// items. The degradation is the honest capability report — `Supported`
    /// only when a graph read is present, `Unsupported` when the service has
    /// no capabilities — and a request whose egress policy denies is refused
    /// up front (`Unavailable`, no items), never planned as content.
    pub fn plan<R>(&self, request: R) -> ContextPlan
    where
        R: AsRef<ContextRequest>,
    {
        let req = request.as_ref();
        // Main: the egress policy travels with the request — enforce it
        // before any plan is shaped. A denied request produces an empty
        // denial shell, never implicitly `Supported` or planned content.
        if let Err(reason) = req.policy.enforce() {
            tracing::debug!(reason = %reason, "context plan denied by egress policy");
            let mut plan = ContextPlan::new(req.event_id.clone(), Degradation::Unavailable);
            plan.applied_scope = req.session_id.clone();
            plan.items = Vec::new();
            plan.truncation = false;
            return plan;
        }
        // Develop: capability-based degradation.
        let degradation = if self.has_graph_read {
            Degradation::Supported
        } else {
            Degradation::Unsupported
        };
        let mut plan = ContextPlan::new(req.event_id.clone(), degradation);
        plan.applied_scope = req.session_id.clone();
        plan.items = Vec::new();
        plan.truncation = false;
        tracing::trace!(
            epoch = req.context_epoch,
            trigger = %req.trigger,
            repo = %req.source.repo_id,
            "planned context"
        );
        plan
    }

    /// Render `plan` as compact text. Associated function (call as
    /// `ContextService::render(&plan)`).
    pub fn render(plan: impl AsRef<ContextPlan>) -> String {
        let plan = plan.as_ref();
        let mut out = String::new();
        out.push_str(&format!(
            "context-plan {} (generation {}, scope {}, degradation: {})\n",
            plan.event_id, plan.index_generation, plan.applied_scope, plan.degradation
        ));
        if plan.items.is_empty() {
            out.push_str("(no items)\n");
        } else {
            for item in &plan.items {
                out.push_str("- ");
                out.push_str(&item.memory_type);
                out.push_str(": ");
                out.push_str(&item.content);
                out.push('\n');
            }
        }
        if plan.truncation {
            out.push_str("… (truncated)\n");
        }
        out
    }

    pub fn render_plan(plan: impl AsRef<ContextPlan>) -> String {
        Self::render(plan)
    }

    pub fn plan_with_items<R>(
        &self,
        request: R,
        candidates: Vec<super::model::ContextItem>,
        budget: super::plan::Budget,
        max_items: usize,
    ) -> ContextPlan
    where
        R: AsRef<ContextRequest>,
    {
        let mut plan = self.plan(&request);
        if request.as_ref().policy.enforce().is_err() {
            // The egress policy denied this request; keep the denial shell
            // (empty items, `Unavailable`) instead of upgrading it to
            // `Supported` — no item may be delivered against refused egress.
            return plan;
        }
        let total = candidates.len();
        let mut deduped = super::plan::rank_dedup_budget(candidates, budget);
        if deduped.len() > max_items {
            deduped.truncate(max_items);
        }
        plan.truncation = total > deduped.len();
        plan.items = deduped;
        // A non-empty selection is fully supported even when `truncation` is
        // set: the plan delivered the items it planned, while `truncation`
        // records (and the debug log below surfaces) that the request was cut
        // short by dedup, budget or `max_items`. An empty selection is a
        // degraded plan — nothing survived to deliver.
        plan.degradation = if !plan.items.is_empty() {
            Degradation::Supported
        } else {
            Degradation::Partial
        };
        if plan.truncation {
            tracing::debug!(
                planned = total,
                delivered = plan.items.len(),
                "context plan truncated: {} of {} candidates delivered",
                plan.items.len(),
                total
            );
        }
        plan
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::model::{ContextItem, ContextRequest, Degradation, EgressPolicy, SourceIdentity};
    use super::super::plan::Budget;

    fn request() -> ContextRequest {
        ContextRequest {
            session_id: "s".into(),
            context_epoch: 1,
            event_id: "evt".into(),
            trigger: "file_touch".into(),
            source: SourceIdentity::default(),
            policy: EgressPolicy::default(),
        }
    }

    fn item(id: &str, content: &str) -> ContextItem {
        ContextItem {
            id: id.into(),
            memory_type: "rule".into(),
            content: content.into(),
            freshness: None,
        }
    }

    #[test]
    fn plan_without_graph_read_is_unsupported() {
        let service = ContextService::default();
        let plan = service.plan(request());
        assert!(plan.items.is_empty());
        assert_eq!(plan.degradation, Degradation::Unsupported);
    }

    #[test]
    fn plan_with_graph_read_is_supported() {
        let service = ContextService::new_with(Some(1u8), Some(2u8));
        let plan = service.plan(&request());
        assert_eq!(plan.degradation, Degradation::Supported);
    }

    /// Merged contract: an empty plan from a service with no capabilities is
    /// `Unsupported` (the honest capability report), not `Partial`. Main's
    /// `Partial`-for-empty survives in `plan_with_items`, where an empty
    /// selection is a degraded-but-observable outcome; `plan()` itself has
    /// no selection to degrade, so the capability answer governs.
    #[test]
    fn plan_degrades_partial_when_empty() {
        let service = ContextService::new();
        let plan = service.plan(&request());
        assert_eq!(plan.degradation, Degradation::Unsupported);
        assert!(plan.items.is_empty());
        assert!(!plan.truncation);
    }

    /// A request whose egress policy denies never produces a deliverable
    /// plan: `plan` refuses up front (`Unavailable`, no items), and
    /// `plan_with_items` keeps that denial shell instead of upgrading it to
    /// `Supported` even when candidates survived.
    #[test]
    fn denied_policy_never_yields_deliverable_plan() {
        let service = ContextService::new();
        let mut denied = request();
        denied.policy = EgressPolicy::deny("egress refused");
        let plan = service.plan(&denied);
        assert_eq!(plan.degradation, Degradation::Unavailable);
        assert!(plan.items.is_empty());

        let with_items = service.plan_with_items(
            &denied,
            vec![item("i1", "c1")],
            Budget::from(10_000usize),
            10,
        );
        assert_eq!(with_items.degradation, Degradation::Unavailable);
        assert!(with_items.items.is_empty());
    }

    #[test]
    fn plan_with_items_rank_dedups_by_id_and_content() {
        let service = ContextService::new();
        let candidates = vec![
            item("i1", "c1"),
            item("i1", "c1-dup-id"),
            item("i2", "c1"),
            item("i3", "c3"),
        ];
        let plan = service.plan_with_items(&request(), candidates, Budget::from(10_000usize), 10);
        assert_eq!(plan.items.len(), 2);
        assert_eq!(plan.items[0].id, "i1");
        assert_eq!(plan.items[1].id, "i3");
        // Drops by dedup/budget also count toward `truncation` (total input >
        // output), matching the long-standing contract.
        assert!(plan.truncation);
        assert_eq!(plan.degradation, Degradation::Supported);
    }

    #[test]
    fn plan_with_items_budgets_out_expensive_items() {
        let service = ContextService::new();
        let candidates = vec![item("a", &"x".repeat(1000)), item("b", "tiny")];
        // item "a" costs 1000/4+8 = 258 tokens > the 30-token budget; "b" fits.
        let plan = service.plan_with_items(&request(), candidates, Budget::from(30usize), 10);
        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].id, "b");
    }

    #[test]
    fn plan_with_items_truncates_to_max_items() {
        let service = ContextService::new();
        let candidates: Vec<ContextItem> = (0..5)
            .map(|i| item(&format!("i{}", i), &format!("content-{}", i)))
            .collect();
        let plan = service.plan_with_items(&request(), candidates, Budget::from(10_000usize), 3);
        assert_eq!(plan.items.len(), 3);
        assert!(plan.truncation);
    }

    #[test]
    fn render_works() {
        let plan = ContextPlan::new("evt", Degradation::Unsupported);
        let text = ContextService::render(&plan);
        assert!(text.contains("context-plan"));
    }
}

