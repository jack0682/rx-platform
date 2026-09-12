use rx_domain::{condition::Condition, intent::Intent, types::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessSource {
    pub schema: Name,
    pub process: Name,
    pub entry: Name,
    pub flows: Vec<Flow>,
    pub conditions: BTreeMap<Name, Condition>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Flow {
    pub id: Name,
    pub root: Name,
    pub nodes: Vec<Node>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: Name,
    pub body: NodeBody,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum NodeBody {
    Sequence {
        children: Vec<Name>,
    },
    ParallelAll {
        children: Vec<Name>,
    },
    Branch {
        condition: Name,
        when_true: Name,
        when_false: Name,
    },
    Repeat {
        count: Counter,
        child: Name,
    },
    Call {
        flow: Name,
    },
    Operation {
        binding: Name,
    },
    Wait {
        condition: Name,
        timeout_ns: Counter,
    },
    Intervention {
        procedure: ArtifactRef,
    },
}
impl NodeBody {
    pub fn children(&self) -> Vec<&Name> {
        match self {
            Self::Sequence { children } | Self::ParallelAll { children } => {
                children.iter().collect()
            }
            Self::Branch {
                when_true,
                when_false,
                ..
            } => vec![when_true, when_false],
            Self::Repeat { child, .. } => vec![child],
            _ => vec![],
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionBinding {
    pub host: Name,
    pub intent: Intent,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceLocation {
    pub flow: Name,
    pub node: Name,
    pub instantiation: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledNode {
    pub id: Name,
    pub source: SourceLocation,
    pub body: CompiledBody,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum CompiledBody {
    Sequence {
        children: Vec<CompiledNode>,
    },
    ParallelAll {
        children: Vec<CompiledNode>,
    },
    Branch {
        condition: Name,
        when_true: Box<CompiledNode>,
        when_false: Box<CompiledNode>,
    },
    Operation {
        binding: Name,
    },
    Wait {
        condition: Name,
        timeout_ns: Counter,
    },
    Intervention {
        procedure: ArtifactRef,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedProcess {
    pub schema: Name,
    pub package_digest: Option<Digest>,
    pub source_digest: Digest,
    pub process: Name,
    pub root: CompiledNode,
    pub bindings: BTreeMap<Name, ActionBinding>,
    pub conditions: BTreeMap<Name, Condition>,
}
