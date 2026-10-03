//! Bounded EXPLAIN JSON model for the native tools view. This module never runs
//! SQL; EXPLAIN ANALYZE must use the policy-enforcing Query Session facade.
//! Timing and estimate calculations preserve the baseline plan-analysis model.
use serde::Serialize;
use serde_json::{Map, Value};

pub const MAX_PLAN_BYTES: usize = 1024 * 1024;
pub const MAX_PLAN_NODES: usize = 4096;
pub const MAX_PLAN_DEPTH: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanError {
    Incomplete,
    TooLarge,
    InvalidJson,
    MissingPlan,
    InvalidNode,
    TooManyNodes,
    TooDeep,
    InvalidMetric,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum Estimate {
    Unknown,
    Accurate(f64),
    Over(f64),
    Under(f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum BufferKind {
    SharedHit,
    SharedRead,
    SharedDirtied,
    SharedWritten,
    TempRead,
    TempWritten,
}

#[derive(Serialize)]
pub struct PlanNode {
    pub node_type: String,
    pub relation: Option<String>,
    pub alias: Option<String>,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub depth: usize,
    pub startup_cost: Option<f64>,
    pub total_cost: Option<f64>,
    pub plan_rows: Option<f64>,
    pub actual_startup_ms: Option<f64>,
    pub actual_total_ms: Option<f64>,
    pub actual_rows: Option<f64>,
    pub actual_loops: Option<f64>,
    pub buffers: Vec<(BufferKind, f64)>,
    pub inclusive_ms: Option<f64>,
    /// Derived attribution, not a separately measured duration. Parallel child
    /// timings may overlap. Unknown child timing uses the inclusive fallback.
    pub self_ms: Option<f64>,
    pub has_unknown_child_timing: bool,
    pub self_fraction: Option<f64>,
    pub estimate: Estimate,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Insight {
    PlanningSkew { ratio: f64 },
    EstimateMiss { node: usize, estimate: Estimate },
    SequentialScans { count: usize, uses_index: bool },
    TemporaryWrites { node: usize },
}

#[derive(Serialize)]
pub struct ExplainPlan {
    /// Exact JSON remains available for inspection. It is never formatted into
    /// a diagnostic or log; it can contain literal values from the query.
    pub raw: String,
    /// Preorder indices are stable within this result and support virtual rows.
    pub nodes: Vec<PlanNode>,
    pub planning_ms: Option<f64>,
    pub execution_ms: Option<f64>,
    pub runtime_ms: u64,
    pub total_exec_ms: Option<f64>,
    pub max_self_ms: f64,
    pub hottest: Option<usize>,
    pub insights: Vec<Insight>,
}

impl ExplainPlan {
    /// `complete` must reflect the execution and cell-retention outcome. A
    /// syntactically valid prefix is still refused after truncation or failure.
    pub fn parse_json(raw: &str, runtime_ms: u64, complete: bool) -> Result<Self, PlanError> {
        if !complete {
            return Err(PlanError::Incomplete);
        }
        if raw.len() > MAX_PLAN_BYTES {
            return Err(PlanError::TooLarge);
        }
        let json: Value = serde_json::from_str(raw).map_err(|_| PlanError::InvalidJson)?;
        let document = match &json {
            Value::Array(plans) if plans.len() == 1 => plans[0].as_object(),
            Value::Object(document) => Some(document),
            _ => None,
        }
        .ok_or(PlanError::MissingPlan)?;
        let mut nodes = Vec::new();
        parse_node(
            document.get("Plan").ok_or(PlanError::MissingPlan)?,
            None,
            0,
            &mut nodes,
        )?;
        let planning_ms = metric(document, "Planning Time")?;
        let execution_ms = metric(document, "Execution Time")?;
        let total_exec_ms = nodes[0].inclusive_ms;
        let mut hottest = None;
        let mut max_self_ms = 0.0;
        for node in &mut nodes {
            node.self_fraction = match (node.self_ms, total_exec_ms) {
                (Some(own), Some(total)) if total > 0.0 => {
                    let fraction = own / total;
                    if !fraction.is_finite() {
                        return Err(PlanError::InvalidMetric);
                    }
                    Some(fraction)
                }
                _ => None,
            };
        }
        // First preorder node wins a tie, matching the baseline waterfall.
        for (index, node) in nodes.iter().enumerate() {
            if let Some(own) = node.self_ms {
                if hottest.is_none() || own > max_self_ms {
                    hottest = Some(index);
                    max_self_ms = own;
                }
            }
        }
        let insights = insights(&nodes, planning_ms, execution_ms.or(total_exec_ms))?;
        Ok(Self {
            raw: raw.into(),
            nodes,
            planning_ms,
            execution_ms,
            runtime_ms,
            total_exec_ms,
            max_self_ms,
            hottest,
            insights,
        })
    }
}

fn metric(object: &Map<String, Value>, key: &str) -> Result<Option<f64>, PlanError> {
    let value = object
        .get(key)
        .map(|value| value.as_f64().ok_or(PlanError::InvalidMetric))
        .transpose()?;
    if value.is_some_and(|n| !n.is_finite() || n < 0.0) {
        return Err(PlanError::InvalidMetric);
    }
    Ok(value)
}

fn parse_node(
    value: &Value,
    parent: Option<usize>,
    depth: usize,
    nodes: &mut Vec<PlanNode>,
) -> Result<usize, PlanError> {
    if depth > MAX_PLAN_DEPTH {
        return Err(PlanError::TooDeep);
    }
    if nodes.len() >= MAX_PLAN_NODES {
        return Err(PlanError::TooManyNodes);
    }
    let value = value.as_object().ok_or(PlanError::InvalidNode)?;
    let text = |key| value.get(key).and_then(Value::as_str).map(str::to_owned);
    let node_type = text("Node Type")
        .filter(|s| !s.trim().is_empty())
        .ok_or(PlanError::InvalidNode)?;
    let actual_total_ms = metric(value, "Actual Total Time")?;
    let actual_loops = metric(value, "Actual Loops")?;
    let inclusive_ms = actual_total_ms.map(|time| time * actual_loops.unwrap_or(1.0));
    if inclusive_ms.is_some_and(|time| !time.is_finite()) {
        return Err(PlanError::InvalidMetric);
    }
    let plan_rows = metric(value, "Plan Rows")?;
    let actual_rows = metric(value, "Actual Rows")?;
    let mut buffers = Vec::new();
    for (kind, key) in [
        (BufferKind::SharedHit, "Shared Hit Blocks"),
        (BufferKind::SharedRead, "Shared Read Blocks"),
        (BufferKind::SharedDirtied, "Shared Dirtied Blocks"),
        (BufferKind::SharedWritten, "Shared Written Blocks"),
        (BufferKind::TempRead, "Temp Read Blocks"),
        (BufferKind::TempWritten, "Temp Written Blocks"),
    ] {
        if let Some(count) = metric(value, key)?.filter(|count| *count > 0.0) {
            buffers.push((kind, count));
        }
    }
    let index = nodes.len();
    nodes.push(PlanNode {
        node_type,
        relation: text("Relation Name"),
        alias: text("Alias"),
        parent,
        children: Vec::new(),
        depth,
        startup_cost: metric(value, "Startup Cost")?,
        total_cost: metric(value, "Total Cost")?,
        plan_rows,
        actual_startup_ms: metric(value, "Actual Startup Time")?,
        actual_total_ms,
        actual_rows,
        actual_loops,
        buffers,
        inclusive_ms,
        self_ms: None,
        has_unknown_child_timing: false,
        self_fraction: None,
        estimate: estimate(plan_rows, actual_rows),
    });
    if let Some(children) = value.get("Plans") {
        for child in children.as_array().ok_or(PlanError::InvalidNode)? {
            let child = parse_node(child, Some(index), depth + 1, nodes)?;
            nodes[index].children.push(child);
        }
    }
    let unknown = nodes[index]
        .children
        .iter()
        .any(|child| nodes[*child].inclusive_ms.is_none());
    let children_ms = nodes[index]
        .children
        .iter()
        .filter_map(|child| nodes[*child].inclusive_ms)
        .sum::<f64>();
    if !children_ms.is_finite() {
        return Err(PlanError::InvalidMetric);
    }
    nodes[index].has_unknown_child_timing = unknown;
    nodes[index].self_ms = inclusive_ms.map(|time| {
        if unknown {
            time
        } else {
            (time - children_ms).max(0.0)
        }
    });
    Ok(index)
}

fn estimate(plan: Option<f64>, actual: Option<f64>) -> Estimate {
    let (Some(plan), Some(actual)) = (plan, actual) else {
        return Estimate::Unknown;
    };
    if plan == actual {
        return Estimate::Accurate(1.0);
    }
    let ratio = plan.max(actual) / plan.min(actual).max(1.0);
    if ratio < 2.0 {
        Estimate::Accurate(ratio)
    } else if plan > actual {
        Estimate::Over(ratio)
    } else {
        Estimate::Under(ratio)
    }
}

fn insights(
    nodes: &[PlanNode],
    planning: Option<f64>,
    execution: Option<f64>,
) -> Result<Vec<Insight>, PlanError> {
    let mut output = Vec::new();
    if let (Some(planning), Some(execution)) = (planning, execution) {
        if execution > 0.0 {
            let ratio = planning / execution;
            if !ratio.is_finite() {
                return Err(PlanError::InvalidMetric);
            }
            if ratio >= 10.0 {
                output.push(Insight::PlanningSkew { ratio });
            }
        }
    }
    let mut worst = None;
    for (index, node) in nodes.iter().enumerate() {
        if let Estimate::Over(ratio) | Estimate::Under(ratio) = node.estimate {
            if worst.is_none_or(|(_, previous)| ratio > previous) {
                worst = Some((index, ratio));
            }
        }
    }
    if let Some((node, _)) = worst {
        output.push(Insight::EstimateMiss {
            node,
            estimate: nodes[node].estimate,
        });
    }
    let count = nodes.iter().filter(|n| n.node_type == "Seq Scan").count();
    if count > 0 {
        output.push(Insight::SequentialScans {
            count,
            uses_index: nodes.iter().any(|n| n.node_type.contains("Index")),
        });
    }
    if let Some(node) = nodes.iter().position(|n| {
        n.buffers
            .iter()
            .any(|(kind, _)| *kind == BufferKind::TempWritten)
    }) {
        output.push(Insight::TemporaryWrites { node });
    }
    Ok(output)
}

#[cfg(test)]
mod tests;
