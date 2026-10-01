use super::*;

/// Resolve pinned type/model inheritance only. Task source-chain execution belongs to the resolver.
pub fn resolve(
    root: &Definition,
    dependencies: &BTreeMap<Reference, Definition>,
) -> Result<Effective, String> {
    if dependencies.len() > 256 {
        return Err("definition dependency limit".into());
    }
    root.verify()?;
    let mut resolver = Resolver {
        catalog: &root.reference.catalog,
        dependencies,
        active: BTreeSet::new(),
    };
    resolver.definition(root)
}
struct Resolver<'a> {
    catalog: &'a Id,
    dependencies: &'a BTreeMap<Reference, Definition>,
    active: BTreeSet<Id>,
}
impl<'a> Resolver<'a> {
    fn lookup(&self, reference: &Reference, kind: Kind) -> Result<&'a Definition, String> {
        if reference.catalog != *self.catalog {
            return Err("cross-catalog reference requires an explicit import".into());
        }
        let value = self
            .dependencies
            .get(reference)
            .ok_or("referenced revision is missing")?;
        if value.reference != *reference || value.body.kind() != kind {
            return Err("reference identity or kind differs".into());
        }
        value.verify()?;
        Ok(value)
    }
    fn property(&self, reference: &Reference) -> Result<&'a Property, String> {
        match &self.lookup(reference, Kind::Property)?.body {
            Body::Property { specification } => Ok(specification),
            _ => Err("expected property definition".into()),
        }
    }
    fn definition(&mut self, d: &Definition) -> Result<Effective, String> {
        d.verify()?;
        if self.active.len() >= 32 || !self.active.insert(d.reference.id.clone()) {
            return Err("cyclic identity or definition depth limit".into());
        }
        let mut result = match &d.body {
            Body::Property { .. } => Effective::default(),
            Body::PointPattern {
                resource_type,
                origin,
                orientation,
                frame,
                axes,
            } => {
                let shape = self.definition(self.lookup(resource_type, Kind::ResourceType)?)?;
                pattern::validate_fields(&shape, origin, orientation.as_ref(), frame, axes)?;
                Effective::default()
            }
            Body::ObjectType { parent, fields } | Body::ResourceType { parent, fields } => {
                let mut inherited = if let Some(parent) = parent {
                    self.definition(self.lookup(parent, d.body.kind())?)?
                } else {
                    Effective::default()
                };
                for (name, field) in fields {
                    if inherited.fields.contains_key(name) {
                        return Err("inherited field cannot be redefined".into());
                    }
                    let property = self.property(&field.property)?;
                    let category = if d.body.kind() == Kind::ObjectType {
                        Category::Object
                    } else {
                        Category::Resource
                    };
                    if property.category != category && property.category != Category::Constraint {
                        return Err("property category does not match type".into());
                    }
                    if property.category == Category::Constraint
                        && property.constraint_scope
                            != Some(if category == Category::Object {
                                ConstraintScope::Object
                            } else {
                                ConstraintScope::Resource
                            })
                    {
                        return Err("constraint scope does not match type".into());
                    }
                    inherited.fields.insert(
                        name.clone(),
                        ResolvedField {
                            property: field.property.clone(),
                            specification: property.clone(),
                            required: field.required,
                            declared_by: d.reference.clone(),
                        },
                    );
                }
                inherited
            }
            Body::ObjectModel {
                object_type,
                values,
            } => self.model(d, object_type, Kind::ObjectType, values)?,
            Body::ResourceModel {
                resource_type,
                values,
            } => self.model(d, resource_type, Kind::ResourceType, values)?,
            Body::ResourceInstance { base, values } => {
                let kind = self
                    .dependencies
                    .get(base)
                    .ok_or("resource base is missing")?
                    .body
                    .kind();
                if !matches!(kind, Kind::ResourceType | Kind::ResourceModel) {
                    return Err("resource instance requires a type or model".into());
                }
                self.model(d, base, kind, values)?
            }
            Body::PropertySet { values } => {
                let mut effective = Effective::default();
                for (name, assignment) in values {
                    let property = self.property(&assignment.property)?;
                    if !property.overridable || property.category == Category::Constraint {
                        return Err("property set cannot override this property".into());
                    }
                    property.check(&assignment.value)?;
                    effective.fields.insert(
                        name.clone(),
                        ResolvedField {
                            property: assignment.property.clone(),
                            specification: property.clone(),
                            required: false,
                            declared_by: d.reference.clone(),
                        },
                    );
                    effective.values.insert(
                        name.clone(),
                        ResolvedValue {
                            value: assignment.value.clone(),
                            declared_by: d.reference.clone(),
                        },
                    );
                }
                effective
            }
            Body::Task {
                slots, properties, ..
            } => {
                for slot in slots.values() {
                    let kind = match slot.kind {
                        SlotKind::Object => Kind::ObjectType,
                        SlotKind::Resource => Kind::ResourceType,
                    };
                    if slot.accepted_types.len() > 32
                        || slot.accepted_types.iter().collect::<BTreeSet<_>>().len()
                            != slot.accepted_types.len()
                    {
                        return Err("task slot type bounds or duplicates".into());
                    }
                    for reference in &slot.accepted_types {
                        self.definition(self.lookup(reference, kind)?)?;
                    }
                }
                let mut effective = Effective::default();
                for (name, usage) in properties {
                    let property = self.property(&usage.property)?;
                    if property.category == Category::Constraint
                        || (!property.overridable
                            && usage
                                .sources
                                .iter()
                                .any(|s| matches!(s, Source::Override | Source::PropertySet)))
                    {
                        return Err("task source chain violates property policy".into());
                    }
                    if let Some(value) = &usage.default {
                        property.check(value)?;
                    }
                    effective.fields.insert(
                        name.clone(),
                        ResolvedField {
                            property: usage.property.clone(),
                            specification: property.clone(),
                            required: usage.required,
                            declared_by: d.reference.clone(),
                        },
                    );
                    // Defaults are merely declared; do not resolve the source chain here.
                }
                effective
            }
        };
        if result.fields.len() > 128 || result.values.len() > 128 {
            return Err("inherited field limit".into());
        }
        result.missing = result
            .fields
            .iter()
            .filter(|(name, field)| field.required && !result.values.contains_key(*name))
            .map(|(name, _)| name.clone())
            .collect();
        self.active.remove(&d.reference.id);
        Ok(result)
    }
    fn model(
        &mut self,
        d: &Definition,
        parent: &Reference,
        kind: Kind,
        values: &BTreeMap<Name, Value>,
    ) -> Result<Effective, String> {
        let mut effective = self.definition(self.lookup(parent, kind)?)?;
        for (name, value) in values {
            let field = effective
                .fields
                .get(name)
                .ok_or("value names a field not declared by its pinned type")?;
            field.specification.check(value)?;
            // Instance configuration may override model defaults. This is not Task Override authority.
            if let Some(previous) = effective.values.insert(
                name.clone(),
                ResolvedValue {
                    value: value.clone(),
                    declared_by: d.reference.clone(),
                },
            ) {
                effective
                    .shadowed
                    .entry(name.clone())
                    .or_default()
                    .push(previous);
            }
        }
        Ok(effective)
    }
}
