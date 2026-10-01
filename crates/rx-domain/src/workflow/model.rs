use crate::{
    canonical,
    definition::{Property, Reference, Slot, Value},
    types::*,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA: &str = "rx.workflow-model.v1";
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Span {
    pub min: Real,
    pub max: Real,
}
impl Span {
    pub fn point(v: Real) -> Self {
        Self { min: v, max: v }
    }
    pub fn exact(&self) -> Option<f64> {
        (self.min == self.max).then(|| self.min.get())
    }
    pub fn checked(min: f64, max: f64) -> Result<Self, String> {
        if min > max {
            return Err("interval lower bound exceeds upper bound".into());
        }
        Ok(Self {
            min: Real::new(min).map_err(|e| e.to_string())?,
            max: Real::new(max).map_err(|e| e.to_string())?,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum Data {
    Number { range: Span },
    Vector { ranges: Vec<Span> },
    Boolean { value: bool },
    Text { value: String },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quantity {
    pub unit: Name,
    pub data: Data,
}
impl Quantity {
    pub fn concrete(&self) -> bool {
        match &self.data {
            Data::Number { range } => range.min == range.max,
            Data::Vector { ranges } => ranges.iter().all(|r| r.min == r.max),
            _ => true,
        }
    }

    pub fn from_value(unit: Name, value: &Value) -> Self {
        Self {
            unit,
            data: match value {
                Value::Number(v) => Data::Number {
                    range: Span::point(*v),
                },
                Value::Vector(v) => Data::Vector {
                    ranges: v.iter().map(|n| Span::point(*n)).collect(),
                },
                Value::Boolean(v) => Data::Boolean { value: *v },
                Value::Text(v) => Data::Text { value: v.clone() },
            },
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        let valid = match &self.data {
            Data::Number { range } => range.min <= range.max,
            Data::Vector { ranges } => {
                !ranges.is_empty() && ranges.len() <= 128 && ranges.iter().all(|r| r.min <= r.max)
            }
            Data::Boolean { .. } => self.unit.as_str() == "unitless",
            Data::Text { value } => {
                self.unit.as_str() == "unitless" && value.chars().count() <= 2048
            }
        };
        if valid {
            Ok(())
        } else {
            Err("quantity interval, shape or unit is invalid".into())
        }
    }
    pub fn check(&self, property: &Property) -> Result<(), String> {
        self.validate()?;
        if self.unit != property.unit {
            return Err(format!(
                "unit {} differs from declared {}",
                self.unit, property.unit
            ));
        }
        let pair = match &self.data {
            Data::Number { range } => {
                if range.min > range.max {
                    return Err("reversed numeric interval".into());
                }
                (Value::Number(range.min), Value::Number(range.max))
            }
            Data::Vector { ranges } => {
                if ranges.iter().any(|r| r.min > r.max) {
                    return Err("reversed vector interval".into());
                }
                (
                    Value::Vector(ranges.iter().map(|r| r.min).collect()),
                    Value::Vector(ranges.iter().map(|r| r.max).collect()),
                )
            }
            Data::Boolean { value } => (Value::Boolean(*value), Value::Boolean(*value)),
            Data::Text { value } => (Value::Text(value.clone()), Value::Text(value.clone())),
        };
        property.check(&pair.0)?;
        property.check(&pair.1)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum Source {
    Override,
    Context {
        slot: Name,
        field: Name,
        index: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        frame_field: Option<Name>,
    },
    PropertySet,
    Rule {
        rule: Name,
    },
    Pattern {
        slot: Name,
        rule: Reference,
        component: PatternComponent,
    },
    Default,
    Input {
        key: Name,
    },
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PatternComponent {
    Position,
    Orientation,
    Frame,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub property: Reference,
    pub sources: Vec<Source>,
    pub default: Option<Quantity>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum Operand {
    Property {
        name: Name,
    },
    Context {
        slot: Name,
        field: Name,
        index: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        frame_field: Option<Name>,
    },
    Rule {
        name: Name,
    },
    Literal {
        value: Quantity,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum Operation {
    Add {
        left: Operand,
        right: Operand,
    },
    Subtract {
        left: Operand,
        right: Operand,
    },
    Min {
        left: Operand,
        right: Operand,
    },
    Max {
        left: Operand,
        right: Operand,
    },
    Scale {
        value: Operand,
        factor: Operand,
    },
    Component {
        vector: Operand,
        index: u16,
    },
    Offset {
        position: Operand,
        orientation: Operand,
        distance: Operand,
        direction: [Real; 3],
    },
}
impl Operation {
    pub fn operands(&self) -> Vec<&Operand> {
        match self {
            Self::Add { left, right }
            | Self::Subtract { left, right }
            | Self::Min { left, right }
            | Self::Max { left, right } => vec![left, right],
            Self::Scale { value, factor } => vec![value, factor],
            Self::Component { vector, .. } => vec![vector],
            Self::Offset {
                position,
                orientation,
                distance,
                ..
            } => vec![position, orientation, distance],
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub output: Reference,
    pub operation: Operation,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Relation {
    Le,
    Ge,
    Eq,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Constraint {
    pub left: Operand,
    pub right: Operand,
    pub relation: Relation,
    pub property: Name,
    pub message: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    pub name: Name,
    pub slot: Name,
    pub field: Name,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skill {
    pub capability: Name,
    pub slot: Name,
    pub implementation_field: Name,
    pub version_field: Name,
    pub primitive: Name,
    pub parameters: BTreeMap<Name, Name>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Done {
    pub observation: Name,
    pub property: Reference,
    pub equals: Quantity,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum KnownFailure {
    Stop,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UnknownOutcome {
    HoldAndReconcile,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub label: String,
    pub contexts: Vec<Name>,
    pub properties: BTreeMap<Name, Usage>,
    pub constraints: Vec<Name>,
    pub capabilities: Vec<Capability>,
    pub skills: Vec<Skill>,
    pub timeout_property: Name,
    pub done: Done,
    pub on_failure: KnownFailure,
    pub on_unknown: UnknownOutcome,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub id: Name,
    pub task: Name,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetSelector {
    pub slot: Name,
    pub accepted_type: Reference,
    pub property_set: Reference,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub schema: Name,
    pub contexts: BTreeMap<Name, Slot>,
    pub defaults: BTreeMap<Name, Vec<Reference>>,
    pub property_sets: Vec<SetSelector>,
    pub tasks: BTreeMap<Name, Task>,
    pub rules: BTreeMap<Name, Rule>,
    pub constraints: BTreeMap<Name, Constraint>,
    pub steps: Vec<Step>,
}
impl Spec {
    pub fn references(&self) -> Vec<Reference> {
        let mut refs = BTreeSet::new();
        refs.extend(
            self.contexts
                .values()
                .flat_map(|s| s.accepted_types.iter())
                .cloned(),
        );
        refs.extend(self.defaults.values().flatten().cloned());
        for s in &self.property_sets {
            refs.insert(s.accepted_type.clone());
            refs.insert(s.property_set.clone());
        }
        for task in self.tasks.values() {
            refs.insert(task.done.property.clone());
            for usage in task.properties.values() {
                refs.insert(usage.property.clone());
                for source in &usage.sources {
                    if let Source::Pattern { rule, .. } = source {
                        refs.insert(rule.clone());
                    }
                }
            }
        }
        refs.extend(self.rules.values().map(|r| r.output.clone()));
        refs.into_iter().collect()
    }
    pub fn validate_shape(&self) -> Result<(), String> {
        if self.schema.as_str() != SCHEMA
            || self.steps.is_empty()
            || self.steps.len() > 128
            || self.tasks.len() > 64
            || self.rules.len() > 256
            || self.constraints.len() > 256
            || self.contexts.len() > 32
            || self.property_sets.len() > 64
            || self.references().len() > 256
            || canonical::bytes(self).map_err(|e| e.to_string())?.len() > 131072
        {
            return Err("workflow schema or complexity bounds".into());
        }
        let mut ids = BTreeSet::new();
        for step in &self.steps {
            if !ids.insert(&step.id) || !self.tasks.contains_key(&step.task) {
                return Err("duplicate node or missing task".into());
            }
        }
        for slot in self.contexts.values() {
            if slot.accepted_types.is_empty()
                || slot.accepted_types.len() > 32
                || slot.label.trim().is_empty()
                || slot.label.chars().count() > 120
                || slot.accepted_types.iter().collect::<BTreeSet<_>>().len()
                    != slot.accepted_types.len()
            {
                return Err("context type declaration missing".into());
            }
        }
        for (slot, values) in &self.defaults {
            if !self.contexts.contains_key(slot) || values.len() > 32 {
                return Err("context defaults differ from declaration".into());
            }
        }
        if self.references().iter().any(|r| r.revision.0 == 0)
            || self
                .property_sets
                .iter()
                .any(|s| !self.contexts.contains_key(&s.slot))
        {
            return Err("invalid reference revision or property-set context".into());
        }
        for task in self.tasks.values() {
            if task.contexts.iter().collect::<BTreeSet<_>>().len() != task.contexts.len()
                || task
                    .capabilities
                    .iter()
                    .map(|c| &c.name)
                    .collect::<BTreeSet<_>>()
                    .len()
                    != task.capabilities.len()
                || task
                    .capabilities
                    .iter()
                    .any(|c| !task.contexts.contains(&c.slot))
            {
                return Err("duplicate or undeclared task context/capability".into());
            }
            if task.label.trim().is_empty()
                || task.label.chars().count() > 120
                || task.properties.len() > 64
                || task.properties.is_empty()
                || task.contexts.len() > 32
                || task.capabilities.len() > 32
                || task.skills.is_empty()
                || task.skills.len() > 16
                || task.constraints.len() > 64
                || !task.properties.contains_key(&task.timeout_property)
                || task.contexts.iter().any(|s| !self.contexts.contains_key(s))
                || task
                    .constraints
                    .iter()
                    .any(|s| !self.constraints.contains_key(s))
            {
                return Err("task contract shape or bounds".into());
            }
            for usage in task.properties.values() {
                if usage.sources.is_empty()
                    || usage.sources.len() > 16
                    || usage
                        .sources
                        .iter()
                        .position(|s| matches!(s, Source::Override))
                        .is_some_and(|i| i != 0)
                    || usage.default.is_some()
                        && !usage.sources.iter().any(|s| matches!(s, Source::Default))
                    || usage
                        .sources
                        .iter()
                        .map(|s| canonical::bytes(s).unwrap_or_default())
                        .collect::<BTreeSet<_>>()
                        .len()
                        != usage.sources.len()
                {
                    return Err("property source chain bounds".into());
                }
            }
            for skill in &task.skills {
                if skill.parameters.is_empty()
                    || skill.parameters.len() > 64
                    || skill
                        .parameters
                        .values()
                        .any(|p| !task.properties.contains_key(p))
                    || !task.contexts.contains(&skill.slot)
                    || !task.capabilities.iter().any(|c| c.name == skill.capability)
                {
                    return Err("skill parameter/capability mapping is incomplete".into());
                }
            }
        }
        if self
            .constraints
            .values()
            .any(|c| c.message.trim().is_empty() || c.message.chars().count() > 1024)
        {
            return Err("constraint message bounds".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub workflow: Reference,
    pub contexts: BTreeMap<Name, Vec<Reference>>,
    pub property_sets: Vec<Reference>,
    pub overrides: BTreeMap<Name, BTreeMap<Name, Quantity>>,
    pub inputs: BTreeMap<Name, Quantity>,
    pub slot_index: Counter,
}
impl Request {
    pub fn validate_shape(&self) -> Result<(), String> {
        if self.contexts.len() > 32
            || self.property_sets.len() > 32
            || self.inputs.len() > 128
            || self.overrides.len() > 128
            || self.contexts.values().any(|v| v.len() > 32)
            || self.overrides.values().any(|v| v.len() > 64)
            || canonical::bytes(self).map_err(|e| e.to_string())?.len() > 131072
        {
            return Err("resolution request bounds".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    pub kind: String,
    pub reference: Option<Reference>,
    pub path: String,
    pub value: Quantity,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resolved {
    pub value: Quantity,
    pub frame: Option<String>,
    pub selected_source: String,
    pub origins: Vec<Origin>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Violation {
    pub location: String,
    pub code: String,
    pub message: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedSkill {
    pub implementation: Resolved,
    pub version: Resolved,
    pub primitive: Name,
    pub parameters: BTreeMap<Name, Name>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedStep {
    pub node: Name,
    pub task: Name,
    pub label: String,
    pub timeout_property: Name,
    pub properties: BTreeMap<Name, Resolved>,
    pub skills: Vec<ResolvedSkill>,
    pub declared_capabilities: BTreeMap<Name, Resolved>,
    pub done: Done,
    pub on_failure: KnownFailure,
    pub on_unknown: UnknownOutcome,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefinitionName {
    pub reference: Reference,
    pub label: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema: Name,
    pub resolver_digest: Digest,
    pub request: Request,
    pub valid: bool,
    pub concrete: bool,
    pub status: String,
    pub definitions: Vec<DefinitionName>,
    pub steps: Vec<ResolvedStep>,
    pub violations: Vec<Violation>,
}
