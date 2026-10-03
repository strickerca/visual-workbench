use super::{ConnectionAssistError as Error, ConnectionRequest, Result};
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const MAX_ROUTES: usize = 4096;
const MAX_METRIC: u32 = 9999;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, uniffi::Enum,
)]
pub enum RouteFamily {
    Ipv4,
    Ipv6,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum MetricActionKind {
    Fix,
    Revert,
}
#[derive(Clone, uniffi::Record)]
pub struct MetricActionDetails {
    pub kind: MetricActionKind,
    pub family: RouteFamily,
    pub interface_index: u32,
    pub old_automatic: bool,
    pub old_metric: u32,
    pub new_automatic: bool,
    pub new_metric: u32,
}

/// No names or addresses cross the UI boundary. The next-hop bytes are hashed
/// in the platform adapter solely to reject a changed/reused route.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct RouteRow {
    pub family: RouteFamily,
    pub index: u32,
    pub luid: u64,
    pub route_metric: u32,
    pub metric: u32,
    pub automatic: bool,
    pub next_hop_hash: [u8; 32],
}
impl RouteRow {
    fn effective(&self) -> u64 {
        u64::from(self.route_metric) + u64::from(self.metric)
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Plan {
    pub kind: MetricActionKind,
    pub family: RouteFamily,
    pub index: u32,
    pub luid: u64,
    pub old_automatic: bool,
    pub old_metric: u32,
    pub new_automatic: bool,
    pub new_metric: u32,
    pub snapshot_hash: [u8; 32],
}
impl Plan {
    pub(super) fn validate(&self) -> Result<()> {
        if self.index == 0
            || self.luid == 0
            || self.old_metric > MAX_METRIC
            || self.new_metric > MAX_METRIC
            || (self.old_automatic == self.new_automatic && self.old_metric == self.new_metric)
            || (self.kind == MetricActionKind::Fix
                && (self.new_automatic || self.new_metric < self.old_metric))
        {
            return Err(Error::InvalidSelection);
        }
        Ok(())
    }
    fn details(&self) -> MetricActionDetails {
        MetricActionDetails {
            kind: self.kind,
            family: self.family,
            interface_index: self.index,
            old_automatic: self.old_automatic,
            old_metric: self.old_metric,
            new_automatic: self.new_automatic,
            new_metric: self.new_metric,
        }
    }
    pub(super) fn matches(&self, row: &RouteRow) -> bool {
        row.family == self.family && row.index == self.index && row.luid == self.luid
    }
}

#[derive(uniffi::Object)]
pub struct RouteMetricAction {
    plan: Plan,
    used: AtomicBool,
}
impl RouteMetricAction {
    pub(super) fn new(plan: Plan) -> Arc<Self> {
        Arc::new(Self {
            plan,
            used: AtomicBool::new(false),
        })
    }
    fn release_known_refusal(&self, error: &Error) {
        // These outcomes are returned only before the setter can run. In
        // particular, process cancellation after launch is OutcomeUnknown,
        // not Cancelled. Retain a saved inverse through a declined UAC prompt.
        if matches!(
            error,
            Error::Busy | Error::Cancelled | Error::ElevationDeclined
        ) {
            self.used.store(false, Ordering::Release);
        }
    }
}
#[derive(Clone, uniffi::Record)]
pub struct RouteMetricReceipt {
    pub applied: MetricActionDetails,
    /// This opaque inverse is bound to the complete observed post-change route
    /// table. No automatic revert is run on close, cancellation or app exit.
    pub inverse: Arc<RouteMetricAction>,
}
#[uniffi::export]
impl RouteMetricAction {
    pub fn details(&self) -> MetricActionDetails {
        self.plan.details()
    }
    /// User presses Apply/Revert, then Windows asks for administrator approval.
    /// This method is never called by preparation, route refresh or shutdown.
    pub async fn apply(&self, request: Arc<ConnectionRequest>) -> Result<RouteMetricReceipt> {
        self.plan.validate()?;
        if self.used.swap(true, Ordering::AcqRel) {
            return Err(Error::AlreadyUsed);
        }
        let plan = self.plan.clone();
        let result = super::offload(request, move |request| {
            let before = super::windows::read_routes()?;
            preflight(&plan, &before)?;
            request.check()?;
            super::windows::elevate_metric(&plan, request)?;
            // A successful helper exit is not sufficient. Bind the inverse to
            // the actual post-state and refuse rollback after external edits.
            let after = super::windows::read_routes().map_err(|_| Error::MetricOutcomeUnknown)?;
            let inverse = verified_inverse(&plan, &before, &after)?;
            Ok(RouteMetricReceipt {
                applied: plan.details(),
                inverse: RouteMetricAction::new(inverse),
            })
        })
        .await;
        if let Err(error) = &result {
            self.release_known_refusal(error);
        }
        result
    }
}

pub(super) fn snapshot_hash(rows: &[RouteRow]) -> Result<[u8; 32]> {
    if rows.len() > MAX_ROUTES
        || rows
            .iter()
            .any(|r| r.index == 0 || r.luid == 0 || r.metric > MAX_METRIC)
    {
        return Err(Error::RouteUnavailable);
    }
    let mut ordered: Vec<_> = rows.iter().collect();
    ordered.sort_unstable();
    let mut hash = blake3::Hasher::new();
    hash.update(b"vw-default-routes-v1\0");
    hash.update(&(ordered.len() as u32).to_le_bytes());
    for row in ordered {
        hash.update(&[if row.family == RouteFamily::Ipv4 {
            4
        } else {
            6
        }]);
        hash.update(&row.index.to_le_bytes());
        hash.update(&row.luid.to_le_bytes());
        hash.update(&row.route_metric.to_le_bytes());
        hash.update(&row.metric.to_le_bytes());
        hash.update(&[u8::from(row.automatic)]);
        hash.update(&row.next_hop_hash);
    }
    Ok(*hash.finalize().as_bytes())
}

pub(super) fn prepare(rows: &[RouteRow], index: u32, family: RouteFamily) -> Result<Plan> {
    let hash = snapshot_hash(rows)?;
    let chosen = rows
        .iter()
        .filter(|row| row.index == index && row.family == family)
        .min_by_key(|r| r.effective())
        .ok_or(Error::NoMetricFix)?;
    // Multiple default routes on one IP interface share the metric fields.
    if rows.iter().any(|row| {
        row.index == index
            && row.family == family
            && (row.luid != chosen.luid
                || row.metric != chosen.metric
                || row.automatic != chosen.automatic)
    }) {
        return Err(Error::RouteUnavailable);
    }
    let alternate = rows
        .iter()
        .filter(|row| row.index != index && row.family == family)
        .map(RouteRow::effective)
        .min()
        .ok_or(Error::NoMetricFix)?;
    if chosen.effective() > alternate {
        return Err(Error::NoMetricFix);
    }
    let desired = alternate
        .checked_add(50)
        .ok_or(Error::NoMetricFix)?
        .saturating_sub(u64::from(chosen.route_metric))
        .max(500);
    let new_metric = u32::try_from(desired)
        .ok()
        .filter(|n| *n <= MAX_METRIC)
        .ok_or(Error::NoMetricFix)?;
    let result = Plan {
        kind: MetricActionKind::Fix,
        family,
        index,
        luid: chosen.luid,
        old_automatic: chosen.automatic,
        old_metric: chosen.metric,
        new_automatic: false,
        new_metric,
        snapshot_hash: hash,
    };
    result.validate()?;
    Ok(result)
}

pub(super) fn preflight(plan: &Plan, rows: &[RouteRow]) -> Result<()> {
    plan.validate()?;
    if snapshot_hash(rows)? != plan.snapshot_hash {
        return Err(Error::StaleRoute);
    }
    let selected: Vec<_> = rows.iter().filter(|r| plan.matches(r)).collect();
    if selected.is_empty()
        || selected
            .iter()
            .any(|r| r.metric != plan.old_metric || r.automatic != plan.old_automatic)
    {
        return Err(Error::StaleRoute);
    }
    Ok(())
}

pub(super) fn verified_inverse(
    plan: &Plan,
    before: &[RouteRow],
    after: &[RouteRow],
) -> Result<Plan> {
    preflight(plan, before)?;
    let after_hash = snapshot_hash(after).map_err(|_| Error::MetricOutcomeUnknown)?;
    let selected = after
        .iter()
        .find(|r| plan.matches(r))
        .ok_or(Error::MetricOutcomeUnknown)?;
    if selected.automatic != plan.new_automatic
        || (!plan.new_automatic && selected.metric != plan.new_metric)
    {
        return Err(Error::MetricOutcomeUnknown);
    }
    if after
        .iter()
        .filter(|r| plan.matches(r))
        .any(|r| r.automatic != selected.automatic || r.metric != selected.metric)
    {
        return Err(Error::MetricOutcomeUnknown);
    }
    let normalized: Vec<_> = after
        .iter()
        .cloned()
        .map(|mut row| {
            if plan.matches(&row) {
                row.metric = plan.old_metric;
                row.automatic = plan.old_automatic;
            }
            row
        })
        .collect();
    if snapshot_hash(&normalized)? != snapshot_hash(before)? {
        return Err(Error::MetricOutcomeUnknown);
    }
    // Automatic mode may recompute its metric; retain the actual new metric in
    // the next action's expected state rather than inventing the prior value.
    Ok(Plan {
        kind: if plan.kind == MetricActionKind::Fix {
            MetricActionKind::Revert
        } else {
            MetricActionKind::Fix
        },
        family: plan.family,
        index: plan.index,
        luid: plan.luid,
        old_automatic: selected.automatic,
        old_metric: selected.metric,
        new_automatic: plan.old_automatic,
        new_metric: plan.old_metric,
        snapshot_hash: after_hash,
    })
}

#[cfg(test)]
mod tests;
