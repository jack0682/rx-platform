//! Check a supplied expanded tree against source without producing a replacement compiled plan.
use crate::{model::*, source_validation, validation};
use rx_domain::canonical;
use std::collections::{BTreeMap, BTreeSet};
pub fn verify(source: &ProcessSource, process: &ResolvedProcess) -> Result<(), String> {
    if !source_validation::validate(source).structurally_valid {
        return Err("invalid source structure".into());
    }
    validation::validate(process)?;
    if source.process != process.process
        || canonical::bytes(&source.conditions).map_err(|e| e.to_string())?
            != canonical::bytes(&process.conditions).map_err(|e| e.to_string())?
    {
        return Err("source/compiled header differs".into());
    }
    let flows: BTreeMap<_, _> = source.flows.iter().map(|f| (&f.id, f)).collect();
    let entry = *flows.get(&source.entry).ok_or("entry missing")?;
    let mut pending = vec![(entry, &entry.root, Vec::<String>::new(), &process.root)];
    let mut count = 0;
    let mut used = BTreeSet::new();
    while let Some((flow, id, instance, compiled)) = pending.pop() {
        count += 1;
        if count > 4096 {
            return Err("expanded source link limit".into());
        }
        used.insert(&flow.id);
        let node = flow
            .nodes
            .iter()
            .find(|n| &n.id == id)
            .ok_or("source node missing")?;
        if compiled.source.flow != flow.id
            || compiled.source.node != node.id
            || compiled.source.instantiation != instance
        {
            return Err("source location differs from expansion position".into());
        }
        match (&node.body, &compiled.body) {
            (
                NodeBody::Sequence { children: expected },
                CompiledBody::Sequence { children: actual },
            )
            | (
                NodeBody::ParallelAll { children: expected },
                CompiledBody::ParallelAll { children: actual },
            ) => {
                if expected.len() != actual.len() {
                    return Err("control child count differs".into());
                }
                for (id, child) in expected.iter().zip(actual) {
                    pending.push((flow, id, instance.clone(), child));
                }
            }
            (
                NodeBody::Branch {
                    condition: c,
                    when_true: t,
                    when_false: f,
                },
                CompiledBody::Branch {
                    condition,
                    when_true,
                    when_false,
                },
            ) if c == condition => {
                pending.push((flow, t, instance.clone(), when_true));
                pending.push((flow, f, instance, when_false));
            }
            (NodeBody::Repeat { count, child }, CompiledBody::Sequence { children }) => {
                if count.0 != children.len() as u64 {
                    return Err("repeat expansion count differs".into());
                }
                for (iteration, compiled) in children.iter().enumerate() {
                    let mut next = instance.clone();
                    next.push(format!("repeat:{}/{}", node.id, iteration));
                    pending.push((flow, child, next, compiled));
                }
            }
            (NodeBody::Call { flow: callee }, CompiledBody::Sequence { children })
                if children.len() == 1 =>
            {
                let next = *flows.get(callee).ok_or("called flow missing")?;
                let mut path = instance;
                path.push(format!("call:{}/{}", flow.id, node.id));
                pending.push((next, &next.root, path, &children[0]));
            }
            (NodeBody::Operation { binding: expected }, CompiledBody::Operation { binding })
                if expected == binding => {}
            (
                NodeBody::Wait {
                    condition: expected,
                    timeout_ns: limit,
                },
                CompiledBody::Wait {
                    condition,
                    timeout_ns,
                },
            ) if expected == condition && limit == timeout_ns => {}
            (
                NodeBody::Intervention {
                    procedure: expected,
                },
                CompiledBody::Intervention { procedure },
            ) if expected == procedure => {}
            _ => return Err("compiled behavior differs from source node".into()),
        }
    }
    if used.len() != flows.len() {
        return Err("unreachable source flow".into());
    }
    Ok(())
}
