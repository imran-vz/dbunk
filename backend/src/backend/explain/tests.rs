use super::*;
use serde_json::json;

#[test]
fn baseline_nested_timing_estimates_and_insights_use_preorder_identity() {
    let raw = json!([{
        "Planning Time": 12.0, "Execution Time": 0.6,
        "Plan": { "Node Type": "Nested Loop", "Actual Total Time": 0.15, "Actual Loops": 4,
            "Plan Rows": 1, "Actual Rows": 100,
            "Plans": [
                { "Node Type": "Index Scan", "Actual Total Time": 0.01, "Actual Loops": 1,
                    "Plan Rows": 100, "Actual Rows": 150 },
                { "Node Type": "Seq Scan", "Relation Name": "漢字", "Alias": "q",
                    "Actual Total Time": 0.5, "Actual Loops": 1, "Plan Rows": 580, "Actual Rows": 1,
                    "Shared Hit Blocks": 4, "Temp Written Blocks": 2, "Temp Read Blocks": 0 }
            ] }
    }])
    .to_string();
    let plan = ExplainPlan::parse_json(&raw, 2, true).unwrap();
    assert_eq!(plan.raw, raw);
    assert_eq!(plan.nodes.len(), 3);
    assert_eq!(plan.nodes[0].children, [1, 2]);
    assert_eq!(plan.nodes[2].parent, Some(0));
    assert_eq!(plan.nodes[2].relation.as_deref(), Some("漢字"));
    assert_eq!(plan.total_exec_ms, Some(0.6));
    assert!((plan.nodes[0].self_ms.unwrap() - 0.09).abs() < 0.000001);
    assert_eq!(plan.nodes[1].estimate, Estimate::Accurate(1.5));
    assert_eq!(plan.nodes[2].estimate, Estimate::Over(580.0));
    assert_eq!(plan.nodes[0].estimate, Estimate::Under(100.0));
    assert_eq!(plan.hottest, Some(2));
    assert_eq!(
        plan.nodes[2].buffers,
        [(BufferKind::SharedHit, 4.0), (BufferKind::TempWritten, 2.0)]
    );
    assert_eq!(
        plan.insights,
        [
            Insight::PlanningSkew { ratio: 20.0 },
            Insight::EstimateMiss {
                node: 2,
                estimate: Estimate::Over(580.0)
            },
            Insight::SequentialScans {
                count: 1,
                uses_index: true
            },
            Insight::TemporaryWrites { node: 2 },
        ]
    );
}

#[test]
fn absent_timing_and_parallel_children_do_not_invent_negative_durations() {
    let unknown = json!({ "Plan": { "Node Type": "Append", "Actual Total Time": 1.0,
        "Plans": [{ "Node Type": "Seq Scan" }] } })
    .to_string();
    let plan = ExplainPlan::parse_json(&unknown, 1, true).unwrap();
    assert!(plan.nodes[0].has_unknown_child_timing);
    assert_eq!(plan.nodes[0].self_ms, Some(1.0));
    assert_eq!(plan.nodes[1].inclusive_ms, None);
    assert_eq!(plan.nodes[1].estimate, Estimate::Unknown);
    let parallel = json!({ "Plan": { "Node Type": "Gather", "Actual Total Time": 1.0,
        "Plans": [{ "Node Type": "Seq Scan", "Actual Total Time": 1.0, "Actual Loops": 3 }] } })
    .to_string();
    let plan = ExplainPlan::parse_json(&parallel, 1, true).unwrap();
    assert_eq!(plan.nodes[0].self_ms, Some(0.0));
    assert_eq!(plan.nodes[1].inclusive_ms, Some(3.0));
    let estimates =
        json!([{ "Plan": { "Node Type": "Result", "Plan Rows": 0, "Actual Rows": 0 } }])
            .to_string();
    let plan = ExplainPlan::parse_json(&estimates, 1, true).unwrap();
    assert_eq!(plan.nodes[0].estimate, Estimate::Accurate(1.0));
    assert_eq!(plan.hottest, None);
    assert_eq!(plan.total_exec_ms, None);
}

#[test]
fn malformed_truncated_and_over_budget_plans_are_explicit_refusals() {
    assert!(matches!(
        ExplainPlan::parse_json("{}", 0, false),
        Err(PlanError::Incomplete)
    ));
    for raw in ["{}", "[]", "[{\"Plan\":{}}, {\"Plan\":{}}]"] {
        assert!(matches!(
            ExplainPlan::parse_json(raw, 0, true),
            Err(PlanError::MissingPlan)
        ));
    }
    assert!(matches!(
        ExplainPlan::parse_json("[{", 0, true),
        Err(PlanError::InvalidJson)
    ));
    assert!(matches!(
        ExplainPlan::parse_json("{\"Plan\":{}}", 0, true),
        Err(PlanError::InvalidNode)
    ));
    assert!(matches!(
        ExplainPlan::parse_json(&" ".repeat(MAX_PLAN_BYTES + 1), 0, true),
        Err(PlanError::TooLarge)
    ));
    let mut nested = json!({ "Node Type": "Result" });
    for _ in 0..=MAX_PLAN_DEPTH {
        nested = json!({ "Node Type": "Append", "Plans": [nested] });
    }
    assert!(matches!(
        ExplainPlan::parse_json(&json!({"Plan":nested}).to_string(), 0, true),
        Err(PlanError::TooDeep)
    ));
    let wide = json!({ "Plan": { "Node Type": "Append", "Plans": vec![json!({"Node Type":"Result"}); MAX_PLAN_NODES] } });
    assert!(matches!(
        ExplainPlan::parse_json(&wide.to_string(), 0, true),
        Err(PlanError::TooManyNodes)
    ));
    let overflow = json!({ "Plan": { "Node Type": "Result", "Actual Total Time": 1e308, "Actual Loops": 1e308 } });
    assert!(matches!(
        ExplainPlan::parse_json(&overflow.to_string(), 0, true),
        Err(PlanError::InvalidMetric)
    ));
    let ratio_overflow = json!({ "Planning Time": 1e308, "Execution Time": 1e-308,
        "Plan": { "Node Type": "Result" } });
    assert!(matches!(
        ExplainPlan::parse_json(&ratio_overflow.to_string(), 0, true),
        Err(PlanError::InvalidMetric)
    ));
}

#[test]
fn present_malformed_metrics_are_not_presented_as_unknown() {
    for metric in [
        json!("not-a-number"),
        json!(null),
        json!({"value": 1}),
        json!(true),
    ] {
        let raw = json!({"Plan": {"Node Type":"Result", "Total Cost":metric}}).to_string();
        assert!(matches!(
            ExplainPlan::parse_json(&raw, 0, true),
            Err(PlanError::InvalidMetric)
        ));
    }
}
