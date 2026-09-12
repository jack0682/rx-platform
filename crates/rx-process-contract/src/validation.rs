use crate::model::*;
use rx_domain::{canonical, condition::Condition, types::*};
use std::collections::{BTreeMap, BTreeSet};

pub fn validate(process: &ResolvedProcess) -> Result<(), String> {
    if process.schema.as_str() != "rx.resolved-process.v1" {
        return Err("resolved process schema".into());
    }
    let mut nodes = BTreeMap::new();
    let mut used = BTreeSet::new();
    visit(&process.root, process, &mut nodes, &mut used, 0)?;
    if process.bindings.keys().cloned().collect::<BTreeSet<_>>() != used {
        return Err("unused or missing action bindings".into());
    }
    for condition in process.conditions.values() {
        let mut count = 0;
        condition_shape(condition, 0, &mut count)?;
    }
    Ok(())
}
pub fn nodes(process: &ResolvedProcess) -> Vec<&CompiledNode> {
    fn collect<'a>(node: &'a CompiledNode, out: &mut Vec<&'a CompiledNode>) {
        out.push(node);
        match &node.body {
            CompiledBody::Sequence { children } | CompiledBody::ParallelAll { children } => {
                for child in children {
                    collect(child, out)
                }
            }
            CompiledBody::Branch {
                when_true,
                when_false,
                ..
            } => {
                collect(when_true, out);
                collect(when_false, out)
            }
            _ => {}
        }
    }
    let mut out = vec![];
    collect(&process.root, &mut out);
    out
}
fn visit<'a>(
    node: &'a CompiledNode,
    process: &ResolvedProcess,
    nodes: &mut BTreeMap<Name, &'a CompiledNode>,
    used: &mut BTreeSet<Name>,
    depth: usize,
) -> Result<BTreeSet<Name>, String> {
    if depth > 64 || nodes.len() >= 4096 || nodes.insert(node.id.clone(), node).is_some() {
        return Err("compiled node count/depth/identity".into());
    }
    let digest = canonical::digest("RX-PROCESS-NODE-v1", &(&process.process, &node.source))
        .map_err(|e| e.to_string())?;
    if node.id.as_str() != format!("node/{digest}") {
        return Err("compiled node/source identity mismatch".into());
    }
    let mut resources = BTreeSet::new();
    match &node.body {
        CompiledBody::Operation { binding } => {
            let action = process
                .bindings
                .get(binding)
                .ok_or("compiled binding missing")?;
            action.intent.normalized().map_err(|e| e.to_string())?;
            used.insert(binding.clone());
            resources.extend(action.intent.resource_set.iter().cloned());
        }
        CompiledBody::Sequence { children } | CompiledBody::ParallelAll { children } => {
            if children.is_empty() {
                return Err("empty compiled control".into());
            }
            for child in children {
                let subset = visit(child, process, nodes, used, depth + 1)?;
                if matches!(node.body, CompiledBody::ParallelAll { .. })
                    && !resources.is_disjoint(&subset)
                {
                    return Err("parallel resource overlap".into());
                }
                resources.extend(subset);
            }
        }
        CompiledBody::Branch {
            condition,
            when_true,
            when_false,
        } => {
            if !process.conditions.contains_key(condition) {
                return Err("branch condition missing".into());
            }
            resources.extend(visit(when_true, process, nodes, used, depth + 1)?);
            resources.extend(visit(when_false, process, nodes, used, depth + 1)?);
        }
        CompiledBody::Wait {
            condition,
            timeout_ns,
        } => {
            if timeout_ns.0 == 0 || !process.conditions.contains_key(condition) {
                return Err("wait definition".into());
            }
        }
        CompiledBody::Intervention { .. } => {}
    }
    Ok(resources)
}
fn condition_shape(value: &Condition, depth: usize, count: &mut usize) -> Result<(), String> {
    *count += 1;
    if depth >= 32 || *count > 1024 {
        return Err("condition limit".into());
    }
    match value {
        Condition::All { children } | Condition::Any { children } => {
            if children.is_empty() {
                return Err("empty condition".into());
            }
            for child in children {
                condition_shape(child, depth + 1, count)?;
            }
        }
        Condition::Range { min, max, .. } if min.get() > max.get() => {
            return Err("condition range".into());
        }
        _ => {}
    }
    Ok(())
}
