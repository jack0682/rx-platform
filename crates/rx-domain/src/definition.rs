//! Version-pinned authoring content. Declarations never grant device or operating authority.
use crate::{canonical, types::*};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub catalog: Id,
    pub id: Id,
    pub revision: Counter,
    pub digest: Digest,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Kind {
    Property,
    ObjectType,
    ObjectModel,
    ResourceType,
    ResourceModel,
    ResourceInstance,
    PropertySet,
    Task,
    PointPattern,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ValueType {
    Number,
    Boolean,
    Text,
    Vector,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Category {
    Object,
    Resource,
    Execution,
    Constraint,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConstraintScope {
    Object,
    Resource,
    System,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Value {
    Number(Real),
    Boolean(bool),
    Text(String),
    Vector(Vec<Real>),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Property {
    pub value_type: ValueType,
    pub category: Category,
    pub constraint_scope: Option<ConstraintScope>,
    pub unit: Name,
    pub minimum: Option<Real>,
    pub maximum: Option<Real>,
    pub choices: Vec<String>,
    pub vector_length: Option<u16>,
    pub overridable: bool,
    /// Logical parameter names. Runtime implementation/signature binding is checked separately.
    pub parameter_mapping: BTreeMap<Name, Name>,
}
impl Property {
    pub fn validate(&self) -> Result<(), String> {
        if (self.category == Category::Constraint) != self.constraint_scope.is_some()
            || self.parameter_mapping.len() > 64
            || self.minimum.zip(self.maximum).is_some_and(|(a, b)| a > b)
            || self.choices.len() > 128
            || self
                .choices
                .iter()
                .any(|v| v.is_empty() || v.chars().count() > 120)
            || self.choices.iter().collect::<BTreeSet<_>>().len() != self.choices.len()
            || (self.value_type != ValueType::Text && !self.choices.is_empty())
            || (matches!(self.value_type, ValueType::Boolean | ValueType::Text)
                && (self.minimum.is_some()
                    || self.maximum.is_some()
                    || self.unit.as_str() != "unitless"))
            || (self.value_type == ValueType::Vector
                && !self.vector_length.is_some_and(|v| (1..=128).contains(&v)))
            || (self.value_type != ValueType::Vector && self.vector_length.is_some())
            || (self.category == Category::Constraint && self.overridable)
        {
            return Err("invalid property type, bounds, unit, choices or override policy".into());
        }
        Ok(())
    }
    pub fn check(&self, value: &Value) -> Result<(), String> {
        self.validate()?;
        let number = |v: &Real| {
            self.minimum.is_none_or(|min| *v >= min) && self.maximum.is_none_or(|max| *v <= max)
        };
        let valid = match (self.value_type, value) {
            (ValueType::Number, Value::Number(v)) => number(v),
            (ValueType::Boolean, Value::Boolean(_)) => true,
            (ValueType::Text, Value::Text(v)) => {
                v.chars().count() <= 2048 && (self.choices.is_empty() || self.choices.contains(v))
            }
            (ValueType::Vector, Value::Vector(v)) => {
                Some(v.len()) == self.vector_length.map(usize::from) && v.iter().all(number)
            }
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err("value does not satisfy its pinned property definition".into())
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub property: Reference,
    pub required: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assignment {
    pub property: Reference,
    pub value: Value,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SlotKind {
    Object,
    Resource,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    pub label: String,
    pub kind: SlotKind,
    pub accepted_types: Vec<Reference>,
    pub required: bool,
    pub multiple: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum Services {
    All,
    Only { names: Vec<String> },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskMetadata {
    pub display_name: String,
    pub category: String,
    pub services: Services,
    /// Human-readable intent; this is not completion evidence.
    pub completion_description: String,
    pub default_timeout_ns: Option<Counter>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum Source {
    Override,
    Context { slot: Name, field: Name },
    PropertySet,
    Default,
    Input { key: Name },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskProperty {
    pub property: Reference,
    pub required: bool,
    pub sources: Vec<Source>,
    pub default: Option<Value>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum Body {
    Property {
        specification: Property,
    },
    ObjectType {
        parent: Option<Reference>,
        fields: BTreeMap<Name, Field>,
    },
    ResourceType {
        parent: Option<Reference>,
        fields: BTreeMap<Name, Field>,
    },
    ObjectModel {
        object_type: Reference,
        values: BTreeMap<Name, Value>,
    },
    ResourceModel {
        resource_type: Reference,
        values: BTreeMap<Name, Value>,
    },
    ResourceInstance {
        base: Reference,
        values: BTreeMap<Name, Value>,
    },
    PropertySet {
        values: BTreeMap<Name, Assignment>,
    },
    PointPattern {
        resource_type: Reference,
        origin: Name,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        orientation: Option<Name>,
        frame: Name,
        axes: Vec<pattern::Axis>,
    },
    Task {
        metadata: TaskMetadata,
        slots: BTreeMap<Name, Slot>,
        properties: BTreeMap<Name, TaskProperty>,
    },
}
impl Body {
    pub fn kind(&self) -> Kind {
        match self {
            Self::Property { .. } => Kind::Property,
            Self::ObjectType { .. } => Kind::ObjectType,
            Self::ObjectModel { .. } => Kind::ObjectModel,
            Self::ResourceType { .. } => Kind::ResourceType,
            Self::ResourceModel { .. } => Kind::ResourceModel,
            Self::ResourceInstance { .. } => Kind::ResourceInstance,
            Self::PropertySet { .. } => Kind::PropertySet,
            Self::Task { .. } => Kind::Task,
            Self::PointPattern { .. } => Kind::PointPattern,
        }
    }
    pub fn references(&self) -> Vec<&Reference> {
        match self {
            Self::Property { .. } => vec![],
            Self::PointPattern { resource_type, .. } => vec![resource_type],
            Self::ObjectType { parent, fields } | Self::ResourceType { parent, fields } => parent
                .iter()
                .chain(fields.values().map(|f| &f.property))
                .collect(),
            Self::ObjectModel { object_type, .. } => vec![object_type],
            Self::ResourceModel { resource_type, .. } => vec![resource_type],
            Self::ResourceInstance { base, .. } => vec![base],
            Self::PropertySet { values } => values.values().map(|v| &v.property).collect(),
            Self::Task {
                slots, properties, ..
            } => slots
                .values()
                .flat_map(|s| s.accepted_types.iter())
                .chain(properties.values().map(|p| &p.property))
                .collect(),
        }
    }
    pub fn validate_shape(&self) -> Result<(), String> {
        if canonical::bytes(self).map_err(|e| e.to_string())?.len() > 65536
            || self.references().len() > 256
        {
            return Err("definition content exceeds its bounds".into());
        }
        let count = match self {
            Self::Property { specification } => {
                specification.validate()?;
                0
            }
            Self::PointPattern { axes, .. } => {
                pattern::validate_axes(axes)?;
                axes.len()
            }
            Self::ObjectType { fields, .. } | Self::ResourceType { fields, .. } => fields.len(),
            Self::ObjectModel { values, .. }
            | Self::ResourceModel { values, .. }
            | Self::ResourceInstance { values, .. } => values.len(),
            Self::PropertySet { values } => values.len(),
            Self::Task {
                metadata,
                slots,
                properties,
            } => {
                if metadata.display_name.chars().count() > 120
                    || metadata.category.trim().is_empty()
                    || metadata.category.chars().count() > 120
                    || metadata.completion_description.chars().count() > 2048
                    || metadata.default_timeout_ns == Some(Counter(0))
                {
                    return Err("task metadata bounds".into());
                }
                if let Services::Only { names } = &metadata.services
                    && (names.is_empty()
                        || names.len() > 64
                        || names
                            .iter()
                            .any(|n| n.trim().is_empty() || n.chars().count() > 120)
                        || names.iter().collect::<BTreeSet<_>>().len() != names.len())
                {
                    return Err("task service selection".into());
                }
                for slot in slots.values() {
                    if slot.label.trim().is_empty()
                        || slot.label.chars().count() > 120
                        || slot.accepted_types.is_empty()
                        || (slot.kind == SlotKind::Object && slot.multiple)
                    {
                        return Err("task slot label, types or multiplicity".into());
                    }
                }
                for p in properties.values() {
                    if p.sources.is_empty()
                        || p.sources.len() > 16
                        || p.default.is_some() && !p.sources.contains(&Source::Default)
                    {
                        return Err("task property source chain is incomplete or too large".into());
                    }
                    let mut seen = BTreeSet::new();
                    for source in &p.sources {
                        if !seen.insert(canonical::bytes(source).map_err(|e| e.to_string())?) {
                            return Err("duplicate property source".into());
                        }
                        if let Source::Context { slot, .. } = source
                            && !slots.contains_key(slot)
                        {
                            return Err("property source names a missing context slot".into());
                        }
                    }
                }
                slots.len() + properties.len()
            }
        };
        if count > 128 || self.references().iter().any(|r| r.revision.0 == 0) {
            return Err("definition field/reference bounds".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub reference: Reference,
    pub label: String,
    pub body: Body,
}
impl Definition {
    pub fn new(
        catalog: Id,
        id: Id,
        revision: Counter,
        label: String,
        body: Body,
    ) -> Result<Self, String> {
        if label.trim().is_empty() || label.chars().count() > 120 || revision.0 == 0 {
            return Err("definition label/revision".into());
        }
        body.validate_shape()?;
        let digest = canonical::digest(
            "RX-AUTHORING-DEFINITION-v1",
            &(&catalog, &id, revision, &label, &body),
        )
        .map_err(|e| e.to_string())?;
        Ok(Self {
            reference: Reference {
                catalog,
                id,
                revision,
                digest,
            },
            label,
            body,
        })
    }
    pub fn verify(&self) -> Result<(), String> {
        let expected = Self::new(
            self.reference.catalog.clone(),
            self.reference.id.clone(),
            self.reference.revision,
            self.label.clone(),
            self.body.clone(),
        )?;
        if expected.reference == self.reference {
            Ok(())
        } else {
            Err("definition content digest differs".into())
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedField {
    pub property: Reference,
    pub specification: Property,
    pub required: bool,
    pub declared_by: Reference,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedValue {
    pub value: Value,
    pub declared_by: Reference,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Effective {
    pub fields: BTreeMap<Name, ResolvedField>,
    pub values: BTreeMap<Name, ResolvedValue>,
    pub shadowed: BTreeMap<Name, Vec<ResolvedValue>>,
    /// Absent required values remain explicit; this projection is not execution readiness.
    pub missing: Vec<Name>,
}

mod validation;
pub use validation::resolve;

pub mod pattern;
