//! Bounded source-structure checks shared by authoring and the solutions compiler.
//! Valid structure is not a binding, package, device or execution approval.
use crate::model::*;
use rx_domain::{canonical, condition::Condition, types::*};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Issue {
    pub location: String,
    pub code: String,
    pub message: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub validator: String,
    pub validator_digest: Digest,
    pub structurally_valid: bool,
    pub issues: Vec<Issue>,
    pub required_bindings: Vec<Name>,
    pub procedure_references: Vec<ArtifactRef>,
    pub expanded_nodes: Counter,
}
impl Report {
    fn empty() -> Self {
        Self {
            validator: "rx.process-source-validator.v1".into(),
            validator_digest: canonical::digest(
                "RX-PROCESS-SOURCE-VALIDATOR-v1",
                &(
                    include_str!("source_validation.rs"),
                    include_str!("model.rs"),
                ),
            )
            .expect("embedded validator sources"),
            structurally_valid: false,
            issues: vec![],
            required_bindings: vec![],
            procedure_references: vec![],
            expanded_nodes: Counter(0),
        }
    }
    fn issue(&mut self, location: impl Into<String>, code: &str, message: &str) {
        if self.issues.len() < 128 {
            self.issues.push(Issue {
                location: location.into(),
                code: code.into(),
                message: message.into(),
            });
        }
    }
}
pub fn document(value: &serde_json::Value) -> Report {
    let bytes = match canonical::bytes(value) {
        Ok(v) => v,
        Err(_) => {
            let mut r = Report::empty();
            r.issue(
                "source",
                "DOCUMENT_SHAPE",
                "source cannot be represented canonically",
            );
            return r;
        }
    };
    match canonical::decode_json::<ProcessSource>(&bytes) {
        Ok(source) => validate(&source),
        Err(error) => {
            let mut r = Report::empty();
            r.issue("source", "DOCUMENT_SHAPE", &error.to_string());
            r
        }
    }
}
pub fn validate(source: &ProcessSource) -> Report {
    let mut r = Report::empty();
    if source.schema.as_str() != "rx.process-source.v1"
        || source.flows.is_empty()
        || source.flows.len() > 128
    {
        r.issue("source", "SOURCE_SHAPE", "schema or flow count");
        return r;
    }
    let flows: BTreeMap<_, _> = source.flows.iter().map(|f| (&f.id, f)).collect();
    if flows.len() != source.flows.len() {
        r.issue("source", "DUPLICATE_FLOW", "duplicate flow ID");
        return r;
    }
    if !flows.contains_key(&source.entry) {
        r.issue(
            source.entry.to_string(),
            "ENTRY_MISSING",
            "entry flow missing",
        );
    }
    let mut bindings = BTreeSet::new();
    for flow in &source.flows {
        let location = flow.id.to_string();
        if flow.nodes.is_empty() || flow.nodes.len() > 1024 {
            r.issue(&location, "NODE_LIMIT", "node count");
            continue;
        }
        let nodes: BTreeMap<_, _> = flow.nodes.iter().map(|n| (&n.id, n)).collect();
        if nodes.len() != flow.nodes.len() {
            r.issue(&location, "DUPLICATE_NODE", "duplicate node ID");
            continue;
        }
        if !nodes.contains_key(&flow.root) {
            r.issue(&location, "ROOT_MISSING", "flow root missing");
            continue;
        }
        let mut parents = BTreeMap::<&Name, usize>::new();
        for node in &flow.nodes {
            let at = format!("{}/{}", flow.id, node.id);
            for child in node.body.children() {
                if !nodes.contains_key(child) {
                    r.issue(&at, "CHILD_MISSING", "child missing");
                }
                *parents.entry(child).or_default() += 1;
            }
            match &node.body {
                NodeBody::Sequence { children } | NodeBody::ParallelAll { children }
                    if children.is_empty() =>
                {
                    r.issue(&at, "EMPTY_CONTROL", "empty control node")
                }
                NodeBody::Repeat { count, .. } if count.0 == 0 || count.0 > 1024 => r.issue(
                    &at,
                    "REPEAT_LIMIT",
                    "repeat must have a finite count in 1..1024",
                ),
                NodeBody::Call { flow } if !flows.contains_key(flow) => {
                    r.issue(&at, "CALL_MISSING", "called flow missing")
                }
                NodeBody::Operation { binding } => {
                    bindings.insert(binding.clone());
                }
                NodeBody::Branch { condition, .. } | NodeBody::Wait { condition, .. }
                    if !source.conditions.contains_key(condition) =>
                {
                    r.issue(&at, "CONDITION_MISSING", "condition missing")
                }
                NodeBody::Wait { timeout_ns, .. } if timeout_ns.0 == 0 => {
                    r.issue(&at, "WAIT_LIMIT", "wait deadline required")
                }
                NodeBody::Intervention { procedure } => {
                    r.procedure_references.push(procedure.clone());
                }
                _ => {}
            }
        }
        if parents.get(&flow.root).copied().unwrap_or(0) != 0 || parents.values().any(|n| *n != 1) {
            r.issue(
                &location,
                "SHARED_OR_CYCLIC_NODE",
                "shared/cyclic node: use a distinct Call occurrence for reuse",
            );
        }
        if r.issues.is_empty() {
            let mut seen = BTreeSet::new();
            fn visit<'a>(
                id: &'a Name,
                nodes: &BTreeMap<&Name, &'a Node>,
                seen: &mut BTreeSet<&'a Name>,
                depth: usize,
            ) -> bool {
                if depth > 64 || !seen.insert(id) {
                    return false;
                }
                nodes[id]
                    .body
                    .children()
                    .into_iter()
                    .all(|child| visit(child, nodes, seen, depth + 1))
            }
            if !visit(&flow.root, &nodes, &mut seen, 0) {
                r.issue(
                    &location,
                    "TREE_DEPTH_OR_CYCLE",
                    "cycle or structural depth",
                );
            } else if seen.len() != nodes.len() {
                r.issue(&location, "UNREACHABLE_NODE", "unreachable nodes");
            }
        }
    }
    for (id, condition) in &source.conditions {
        let mut count = 0;
        if let Err(reason) = condition_shape(condition, 0, &mut count) {
            r.issue(id.to_string(), "CONDITION_SHAPE", reason);
        }
    }
    r.required_bindings = bindings.into_iter().collect();
    if r.issues.is_empty() {
        let mut visited = BTreeSet::new();
        let mut calls = vec![source.entry.clone()];
        let mut remaining = 4096u64;
        fn expand(
            flows: &BTreeMap<&Name, &Flow>,
            flow: &Flow,
            node: &Name,
            calls: &mut Vec<Name>,
            visited: &mut BTreeSet<Name>,
            remaining: &mut u64,
            depth: usize,
        ) -> Result<(), (&'static str, &'static str)> {
            if depth > 64 || *remaining == 0 {
                return Err(("EXPANSION_LIMIT", "expanded plan limit"));
            }
            *remaining -= 1;
            visited.insert(flow.id.clone());
            let n = flow
                .nodes
                .iter()
                .find(|n| &n.id == node)
                .expect("validated child");
            match &n.body {
                NodeBody::Call { flow: target } => {
                    if calls.contains(target) {
                        return Err(("CALL_CYCLE", "recursive flow call"));
                    }
                    calls.push(target.clone());
                    let next = flows[target];
                    expand(
                        flows,
                        next,
                        &next.root,
                        calls,
                        visited,
                        remaining,
                        depth + 1,
                    )?;
                    calls.pop();
                }
                NodeBody::Repeat { count, child } => {
                    for _ in 0..count.0 {
                        expand(flows, flow, child, calls, visited, remaining, depth + 1)?;
                    }
                }
                _ => {
                    for child in n.body.children() {
                        expand(flows, flow, child, calls, visited, remaining, depth + 1)?;
                    }
                }
            }
            Ok(())
        }
        let entry = flows[&source.entry];
        if let Err((code, message)) = expand(
            &flows,
            entry,
            &entry.root,
            &mut calls,
            &mut visited,
            &mut remaining,
            0,
        ) {
            r.issue(source.entry.to_string(), code, message);
        } else if visited.len() != flows.len() {
            r.issue("source", "UNREACHABLE_FLOW", "unreachable flow definition");
        }
        r.expanded_nodes = Counter(4096 - remaining);
    }
    r.structurally_valid = r.issues.is_empty();
    r
}
fn condition_shape(c: &Condition, depth: usize, count: &mut usize) -> Result<(), &'static str> {
    *count += 1;
    if depth >= 32 || *count > 1024 {
        return Err("condition complexity");
    }
    match c {
        Condition::All { children } | Condition::Any { children } => {
            if children.is_empty() {
                return Err("empty condition");
            }
            for c in children {
                condition_shape(c, depth + 1, count)?;
            }
        }
        Condition::Range { min, max, .. } if min > max => return Err("inverted range"),
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> ProcessSource {
        ProcessSource {
            schema: Name::new("rx.process-source.v1").unwrap(),
            process: Name::new("example").unwrap(),
            entry: Name::new("main").unwrap(),
            conditions: BTreeMap::new(),
            flows: vec![Flow {
                id: Name::new("main").unwrap(),
                root: Name::new("root").unwrap(),
                nodes: vec![Node {
                    id: Name::new("root").unwrap(),
                    body: NodeBody::Operation {
                        binding: Name::new("load").unwrap(),
                    },
                }],
            }],
        }
    }
    #[test]
    fn source_validation_does_not_claim_binding_or_package_approval() {
        let report = validate(&source());
        assert!(report.structurally_valid);
        assert_eq!(report.required_bindings, vec![Name::new("load").unwrap()]);
        assert_eq!(report.expanded_nodes, Counter(1));
        let bad = document(&serde_json::json!({"schema":"rx.process-source.v1","flows":[]}));
        assert!(!bad.structurally_valid);
        assert_eq!(bad.issues[0].code, "DOCUMENT_SHAPE");
    }
    #[test]
    fn recursive_calls_unreachable_nodes_and_expansion_overflow_are_invalid() {
        let mut value = source();
        value.flows[0].nodes[0].body = NodeBody::Call {
            flow: value.entry.clone(),
        };
        assert!(
            validate(&value)
                .issues
                .iter()
                .any(|i| i.code == "CALL_CYCLE")
        );
        let mut value = source();
        value.flows[0].nodes.push(Node {
            id: Name::new("unused").unwrap(),
            body: NodeBody::Operation {
                binding: Name::new("x").unwrap(),
            },
        });
        assert!(
            validate(&value)
                .issues
                .iter()
                .any(|i| i.code == "UNREACHABLE_NODE")
        );
        let mut value = source();
        value.flows[0].nodes[0].body = NodeBody::Repeat {
            count: Counter(1024),
            child: Name::new("inner").unwrap(),
        };
        value.flows[0].nodes.push(Node {
            id: Name::new("inner").unwrap(),
            body: NodeBody::Repeat {
                count: Counter(1024),
                child: Name::new("leaf").unwrap(),
            },
        });
        value.flows[0].nodes.push(Node {
            id: Name::new("leaf").unwrap(),
            body: NodeBody::Operation {
                binding: Name::new("load").unwrap(),
            },
        });
        assert!(
            validate(&value)
                .issues
                .iter()
                .any(|i| i.code == "EXPANSION_LIMIT")
        );
    }
    #[test]
    fn a_full_error_budget_does_not_attempt_to_follow_missing_nodes() {
        let mut value = source();
        value.flows[0].nodes.clear();
        for i in 0..300 {
            value.flows[0].nodes.push(Node {
                id: Name::new(format!("n{i}")).unwrap(),
                body: NodeBody::Sequence {
                    children: vec![Name::new(format!("missing-{i}")).unwrap()],
                },
            });
        }
        value.flows[0].root = Name::new("n0").unwrap();
        let report = validate(&value);
        assert!(!report.structurally_valid);
        assert_eq!(report.issues.len(), 128);
    }
}
