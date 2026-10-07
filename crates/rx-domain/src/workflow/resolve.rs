use super::{math, model::*};
use crate::{
    canonical,
    definition::{self, Body, Definition, Effective, Kind, Reference, SlotKind, Value},
    types::*,
};
use std::collections::{BTreeMap, BTreeSet};

fn n(s: &str) -> Name {
    Name::new(s).expect("static name")
}
fn escaped(s: &Name) -> String {
    s.as_str().replace('~', "~0").replace('/', "~1")
}
#[derive(Clone, Debug)]
struct Error {
    code: &'static str,
    message: String,
}
type Result<T> = std::result::Result<T, Error>;
fn err(code: &'static str, message: impl Into<String>) -> Error {
    Error {
        code,
        message: message.into(),
    }
}
#[derive(Clone)]
struct Context {
    definition: Definition,
    effective: Effective,
}

fn definition<'a>(
    all: &'a BTreeMap<Reference, Definition>,
    r: &Reference,
) -> Result<&'a Definition> {
    let d = all
        .get(r)
        .ok_or_else(|| err("REFERENCE_MISSING", "pinned definition not found"))?;
    if &d.reference != r {
        return Err(err("REFERENCE_MISMATCH", "definition reference differs"));
    }
    d.verify().map_err(|e| err("REFERENCE_MISMATCH", e))?;
    Ok(d)
}
fn accepts(d: &Definition, typ: &Reference, all: &BTreeMap<Reference, Definition>) -> bool {
    let mut cursor = d;
    let mut seen = BTreeSet::new();
    loop {
        if &cursor.reference == typ {
            return true;
        }
        if !seen.insert(cursor.reference.clone()) || seen.len() > 32 {
            return false;
        }
        let parent = match &cursor.body {
            Body::ObjectModel { object_type, .. } => Some(object_type),
            Body::ResourceModel { resource_type, .. } => Some(resource_type),
            Body::ResourceInstance { base, .. } | Body::ObjectInstance { base, .. } => Some(base),
            Body::ObjectType { parent, .. } | Body::ResourceType { parent, .. } => parent.as_ref(),
            _ => None,
        };
        let Some(parent) = parent else {
            return false;
        };
        let Ok(next) = definition(all, parent) else {
            return false;
        };
        cursor = next;
    }
}
fn property<'a>(
    all: &'a BTreeMap<Reference, Definition>,
    r: &Reference,
) -> Result<&'a definition::Property> {
    match &definition(all, r)?.body {
        Body::Property { specification } => Ok(specification),
        _ => Err(err("PROPERTY_REFERENCE", "expected a Property definition")),
    }
}
fn definition_graph(
    d: &Definition,
    all: &BTreeMap<Reference, Definition>,
) -> Result<BTreeMap<Reference, Definition>> {
    // The legacy resolver deliberately bounds each transitive dependency graph, not the entire workflow library.
    let mut deps = BTreeMap::new();
    let mut pending = d.body.references().into_iter().cloned().collect::<Vec<_>>();
    while let Some(r) = pending.pop() {
        if deps.contains_key(&r) {
            continue;
        }
        if deps.len() >= 256 {
            return Err(err("DEPENDENCY_LIMIT", "definition dependency limit"));
        }
        let value = definition(all, &r)?;
        pending.extend(value.body.references().into_iter().cloned());
        deps.insert(r, value.clone());
    }
    Ok(deps)
}
fn resolve_definition(d: &Definition, all: &BTreeMap<Reference, Definition>) -> Result<Effective> {
    definition::resolve(d, &definition_graph(d, all)?).map_err(|e| err("DEFINITION_INVALID", e))
}
fn origin(kind: &str, reference: Option<Reference>, path: String, value: Quantity) -> Resolved {
    Resolved {
        value: value.clone(),
        frame: None,
        selected_source: kind.into(),
        origins: vec![Origin {
            kind: kind.into(),
            reference,
            path,
            value,
        }],
    }
}
fn join_origins(values: &[Resolved]) -> Vec<Origin> {
    let mut seen = BTreeSet::new();
    let mut result = vec![];
    for v in values {
        for o in &v.origins {
            if let Ok(bytes) = canonical::bytes(o)
                && seen.insert(bytes)
            {
                result.push(o.clone());
            }
        }
    }
    result
}
pub fn resolve(
    spec: &Spec,
    request: Request,
    all: &BTreeMap<Reference, Definition>,
) -> std::result::Result<Report, String> {
    spec.validate_shape()?;
    request.validate_shape()?;
    let resolver_digest = canonical::digest(
        "RX-WORKFLOW-RESOLVER-v1",
        &(
            include_str!("model.rs"),
            include_str!("math.rs"),
            include_str!("resolve.rs"),
            include_str!("../definition/pattern.rs"),
        ),
    )
    .map_err(|e| e.to_string())?;
    let mut report = Report {
        schema: n("rx.workflow-resolution.v1"),
        resolver_digest,
        request: request.clone(),
        valid: false,
        concrete: false,
        status: "BLOCKED".into(),
        definitions: all
            .values()
            .map(|d| DefinitionName {
                reference: d.reference.clone(),
                label: d.label.clone(),
            })
            .collect(),
        steps: vec![],
        violations: vec![],
    };
    let mut bindings = spec.defaults.clone();
    bindings.extend(request.contexts.clone());
    let mut contexts = BTreeMap::new();
    for key in bindings.keys() {
        if !spec.contexts.contains_key(key) {
            issue(
                &mut report,
                &format!("contexts/{}", escaped(key)),
                err("UNKNOWN_CONTEXT", "context is not declared"),
            );
        }
    }
    for (key, slot) in &spec.contexts {
        let refs = bindings.get(key).cloned().unwrap_or_default();
        if (slot.required && refs.is_empty()) || (!slot.multiple && refs.len() > 1) {
            issue(
                &mut report,
                &format!("contexts/{}", escaped(key)),
                err(
                    "CONTEXT_CARDINALITY",
                    "context required/multiplicity contract differs",
                ),
            );
        }
        if refs.iter().collect::<BTreeSet<_>>().len() != refs.len() {
            issue(
                &mut report,
                &format!("contexts/{}", escaped(key)),
                err("CONTEXT_DUPLICATE", "same target is bound more than once"),
            );
        }
        for accepted in &slot.accepted_types {
            let expected = match slot.kind {
                SlotKind::Object => Kind::ObjectType,
                SlotKind::Resource => Kind::ResourceType,
            };
            if !definition(all, accepted).is_ok_and(|d| d.body.kind() == expected) {
                issue(
                    &mut report,
                    &format!("contexts/{}/accepted_types", escaped(key)),
                    err(
                        "CONTEXT_TYPE_REFERENCE",
                        "accepted type must refer to the declared type kind",
                    ),
                );
            }
        }
        let mut values = vec![];
        for r in refs {
            let result = (|| {
                let d = definition(all, &r)?;
                let kind = match slot.kind {
                    SlotKind::Object => {
                        matches!(d.body.kind(), Kind::ObjectModel | Kind::ObjectInstance)
                    }
                    SlotKind::Resource => {
                        matches!(d.body.kind(), Kind::ResourceModel | Kind::ResourceInstance)
                    }
                };
                if !kind || !slot.accepted_types.iter().any(|t| accepts(d, t, all)) {
                    return Err(err(
                        "CONTEXT_TYPE",
                        "selected definition does not satisfy an accepted type revision",
                    ));
                }
                let effective = resolve_definition(d, all)?;
                if !effective.missing.is_empty() {
                    return Err(err(
                        "CONTEXT_VALUES_MISSING",
                        format!("required fields: {:?}", effective.missing),
                    ));
                }
                Ok(Context {
                    definition: d.clone(),
                    effective,
                })
            })();
            match result {
                Ok(v) => values.push(v),
                Err(e) => issue(&mut report, &format!("contexts/{}", escaped(key)), e),
            }
        }
        contexts.insert(key.clone(), values);
    }
    let mut sets = BTreeSet::new();
    sets.extend(request.property_sets.iter().cloned());
    for selector in &spec.property_sets {
        let kind = match spec.contexts[&selector.slot].kind {
            SlotKind::Object => Kind::ObjectType,
            SlotKind::Resource => Kind::ResourceType,
        };
        if !definition(all, &selector.accepted_type).is_ok_and(|d| d.body.kind() == kind) {
            issue(
                &mut report,
                "property_sets",
                err(
                    "SELECTOR_TYPE",
                    "property-set selector must reference the declared context type kind",
                ),
            );
            continue;
        }
        if contexts.get(&selector.slot).is_some_and(|v| {
            v.iter()
                .any(|c| accepts(&c.definition, &selector.accepted_type, all))
        }) {
            sets.insert(selector.property_set.clone());
        }
    }
    let mut set_values = vec![];
    for reference in sets {
        let result = (|| {
            let d = definition(all, &reference)?;
            if !matches!(d.body, Body::PropertySet { .. }) {
                return Err(err("PROPERTY_SET_TYPE", "PropertySet definition required"));
            }
            Ok(Context {
                definition: d.clone(),
                effective: resolve_definition(d, all)?,
            })
        })();
        match result {
            Ok(v) => set_values.push(v),
            Err(e) => issue(&mut report, "property_sets", e),
        }
    }
    for (node, overrides) in &request.overrides {
        let task = spec
            .steps
            .iter()
            .find(|s| s.id == *node)
            .and_then(|s| spec.tasks.get(&s.task));
        for prop in overrides.keys() {
            if task.is_none_or(|t| !t.properties.contains_key(prop)) {
                issue(
                    &mut report,
                    &format!("nodes/{}/properties/{}", escaped(node), escaped(prop)),
                    err(
                        "UNKNOWN_OVERRIDE",
                        "override property is not declared at this node",
                    ),
                );
            }
        }
    }
    let declared_inputs = spec
        .steps
        .iter()
        .map(|s| &spec.tasks[&s.task])
        .flat_map(|t| t.properties.values())
        .flat_map(|u| u.sources.iter())
        .filter_map(|s| match s {
            Source::Input { key } => Some(key),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    for key in request.inputs.keys() {
        if !declared_inputs.contains(key) {
            issue(
                &mut report,
                &format!("inputs/{}", escaped(key)),
                err("UNKNOWN_INPUT", "runtime input is not declared"),
            );
        }
    }
    for step in &spec.steps {
        let task = &spec.tasks[&step.task];
        let mut evaluator = Evaluator {
            spec,
            request: &request,
            all,
            contexts: &contexts,
            sets: &set_values,
            step,
            task,
            cache: BTreeMap::new(),
            active: BTreeSet::new(),
            remaining: 4096,
        };
        let mut properties = BTreeMap::new();
        for (name, usage) in &task.properties {
            if usage
                .sources
                .iter()
                .any(|s| matches!(s, Source::PropertySet))
            {
                let mut candidate: Option<Quantity> = None;
                for set in &set_values {
                    if let Some(value) = set.effective.values.get(name) {
                        let field = &set.effective.fields[name];
                        let quantity =
                            Quantity::from_value(field.specification.unit.clone(), &value.value);
                        if field.property != usage.property
                            || candidate.as_ref().is_some_and(|q| q != &quantity)
                        {
                            issue(
                                &mut report,
                                &evaluator.location(name),
                                err(
                                    "PROPERTY_SET_CONFLICT",
                                    "selected property sets disagree or refer to a different property revision",
                                ),
                            );
                        }
                        candidate = Some(quantity);
                    }
                }
            }
            // Validate supplied runtime input even when another tier would hide it.
            for source in &usage.sources {
                if let Source::Input { key } = source
                    && let Some(value) = request.inputs.get(key)
                    && let Err(e) = property(all, &usage.property)
                        .and_then(|p| value.check(p).map_err(|e| err("INPUT_INVALID", e)))
                {
                    issue(&mut report, &evaluator.location(name), e);
                }
            }
            match evaluator.value(name) {
                Ok(v) => {
                    properties.insert(name.clone(), v);
                }
                Err(e) => issue(&mut report, &evaluator.location(name), e),
            }
        }
        if let Some(timeout) = properties.get(&task.timeout_property) {
            let valid = timeout.value.unit.as_str() == "s"
                && matches!(&timeout.value.data,Data::Number{range} if range.min.get()>0.0);
            if !valid {
                issue(
                    &mut report,
                    &evaluator.location(&task.timeout_property),
                    err("TIMEOUT_INVALID", "positive timeout in seconds is required"),
                );
            }
        }
        for constraint_id in &task.constraints {
            let constraint = &spec.constraints[constraint_id];
            let result = (|| {
                let a = evaluator.operand(
                    &constraint.left,
                    &format!("constraints/{constraint_id}/left"),
                )?;
                let b = evaluator.operand(
                    &constraint.right,
                    &format!("constraints/{constraint_id}/right"),
                )?;
                if a.frame != b.frame {
                    return Err(err(
                        "CONSTRAINT_FRAME",
                        "constraint operands use different coordinate frames",
                    ));
                }
                if !math::compare(constraint.relation, &a.value, &b.value)
                    .map_err(|e| err("CONSTRAINT_TYPE", e))?
                {
                    return Err(err(
                        "CONSTRAINT_VIOLATION",
                        format!(
                            "{}; worst-case left={} right={}",
                            constraint.message,
                            serde_json::to_string(&a.value).unwrap_or_default(),
                            serde_json::to_string(&b.value).unwrap_or_default()
                        ),
                    ));
                }
                Ok(())
            })();
            if let Err(e) = result {
                issue(&mut report, &evaluator.location(&constraint.property), e);
            }
        }
        let mut declared_capabilities = BTreeMap::new();
        for capability in &task.capabilities {
            let result = evaluator.context(&capability.slot, &capability.field, 0);
            if let Ok(v) = &result {
                declared_capabilities.insert(capability.name.clone(), v.clone());
            }
            match result {
                Ok(v)
                    if matches!(v.value.data, Data::Boolean { value: true })
                        && v.value.unit.as_str() == "unitless" => {}
                Ok(_) => issue(
                    &mut report,
                    &format!(
                        "nodes/{}/capabilities/{}",
                        escaped(&step.id),
                        escaped(&capability.name)
                    ),
                    err(
                        "CAPABILITY_MISSING",
                        "required capability is not declared true",
                    ),
                ),
                Err(e) => issue(
                    &mut report,
                    &format!(
                        "nodes/{}/capabilities/{}",
                        escaped(&step.id),
                        escaped(&capability.name)
                    ),
                    e,
                ),
            }
        }
        let mut skills = vec![];
        for skill in &task.skills {
            let result = (|| {
                let implementation =
                    evaluator.context(&skill.slot, &skill.implementation_field, 0)?;
                let version = evaluator.context(&skill.slot, &skill.version_field, 0)?;
                for v in [&implementation, &version] {
                    if !matches!(&v.value.data,Data::Text{value} if !value.trim().is_empty())
                        || v.value.unit.as_str() != "unitless"
                    {
                        return Err(err(
                            "SKILL_MAPPING_INVALID",
                            "implementation and version must be declared text",
                        ));
                    }
                }
                Ok(ResolvedSkill {
                    implementation,
                    version,
                    primitive: skill.primitive.clone(),
                    parameters: skill.parameters.clone(),
                })
            })();
            match result {
                Ok(v) => skills.push(v),
                Err(e) => issue(
                    &mut report,
                    &format!(
                        "nodes/{}/skills/{}",
                        escaped(&step.id),
                        escaped(&skill.primitive)
                    ),
                    e,
                ),
            }
        }
        if let Err(e) = property(all, &task.done.property).and_then(|p| {
            task.done
                .equals
                .check(p)
                .map_err(|e| err("DONE_CONTRACT_INVALID", e))
        }) {
            issue(&mut report, &format!("nodes/{}/done", escaped(&step.id)), e);
        }
        report.steps.push(ResolvedStep {
            node: step.id.clone(),
            task: step.task.clone(),
            label: task.label.clone(),
            timeout_property: task.timeout_property.clone(),
            properties,
            skills,
            declared_capabilities,
            done: task.done.clone(),
            on_failure: task.on_failure,
            on_unknown: task.on_unknown,
        });
    }
    report.valid = report.violations.is_empty();
    report.concrete = report.valid
        && report.steps.iter().all(|s| {
            s.properties
                .get(&s.timeout_property)
                .is_some_and(|v| v.value.concrete())
                && s.skills.iter().all(|skill| {
                    skill
                        .parameters
                        .values()
                        .all(|p| s.properties.get(p).is_some_and(|v| v.value.concrete()))
                })
        });
    if report.valid {
        report.status = if report.concrete {
            "RESOLVED_NOT_QUALIFIED"
        } else {
            "BOUNDED_INPUT_NOT_EXECUTABLE"
        }
        .into();
    }
    if canonical::bytes(&report).map_err(|e| e.to_string())?.len() > 900_000 {
        return Err("resolved report exceeds supported size".into());
    }
    Ok(report)
}
fn issue(report: &mut Report, location: &str, error: Error) {
    if report
        .violations
        .iter()
        .any(|v| v.location == location && v.code == error.code && v.message == error.message)
    {
        return;
    }
    if report.violations.len() < 512 {
        report.violations.push(Violation {
            location: location.into(),
            code: error.code.into(),
            message: error.message,
        });
    } else if report.violations.len() == 512 {
        report.violations.push(Violation {
            location: "workflow".into(),
            code: "DIAGNOSTIC_LIMIT".into(),
            message: "Further diagnostics omitted; resolution remains blocked".into(),
        });
    }
}
struct Evaluator<'a> {
    spec: &'a Spec,
    request: &'a Request,
    all: &'a BTreeMap<Reference, Definition>,
    contexts: &'a BTreeMap<Name, Vec<Context>>,
    sets: &'a [Context],
    step: &'a Step,
    task: &'a Task,
    cache: BTreeMap<String, Result<Resolved>>,
    active: BTreeSet<String>,
    remaining: usize,
}
impl Evaluator<'_> {
    fn location(&self, prop: &Name) -> String {
        format!(
            "nodes/{}/properties/{}",
            escaped(&self.step.id),
            escaped(prop)
        )
    }
    fn context(&self, slot: &Name, field: &Name, index: u16) -> Result<Resolved> {
        if !self.task.contexts.contains(slot) {
            return Err(err(
                "UNDECLARED_CONTEXT",
                "task does not declare this context",
            ));
        }
        let context = self
            .contexts
            .get(slot)
            .and_then(|v| v.get(usize::from(index)))
            .ok_or_else(|| err("CONTEXT_MISSING", "context target missing"))?;
        let f = context.effective.fields.get(field).ok_or_else(|| {
            err(
                "FIELD_UNDECLARED",
                format!("{slot}.{field} is not declared"),
            )
        })?;
        let value = context
            .effective
            .values
            .get(field)
            .ok_or_else(|| err("VALUE_MISSING", format!("{slot}.{field} has no value")))?;
        let q = Quantity::from_value(f.specification.unit.clone(), &value.value);
        let mut result = origin(
            "CONTEXT",
            Some(value.declared_by.clone()),
            format!("{slot}[{index}]/{field}"),
            q,
        );
        for old in context.effective.shadowed.get(field).into_iter().flatten() {
            result.origins.push(Origin {
                kind: "SHADOWED".into(),
                reference: Some(old.declared_by.clone()),
                path: format!("{slot}[{index}]/{field}"),
                value: Quantity::from_value(f.specification.unit.clone(), &old.value),
            });
        }
        Ok(result)
    }
    fn framed_context(
        &self,
        slot: &Name,
        field: &Name,
        index: u16,
        frame_field: Option<&Name>,
    ) -> Result<Resolved> {
        let mut value = self.context(slot, field, index)?;
        if let Some(frame_field) = frame_field {
            let frame = self.context(slot, frame_field, index)?;
            let Data::Text { value: text } = &frame.value.data else {
                return Err(err("FRAME_INVALID", "frame field must be text"));
            };
            if text.trim().is_empty() || frame.value.unit.as_str() != "unitless" {
                return Err(err(
                    "FRAME_INVALID",
                    "explicit unitless frame name required",
                ));
            }
            value.frame = Some(text.clone());
            value.origins.extend(frame.origins);
        }
        Ok(value)
    }
    fn enter(&mut self, key: &str) -> Result<()> {
        if self.remaining == 0 || self.active.len() >= 32 {
            return Err(err("RESOLUTION_LIMIT", "resolution work/depth limit"));
        }
        self.remaining -= 1;
        if !self.active.insert(key.into()) {
            return Err(err("RESOLUTION_CYCLE", format!("cycle at {key}")));
        }
        Ok(())
    }
    fn value(&mut self, name: &Name) -> Result<Resolved> {
        let key = format!("property/{name}");
        if let Some(v) = self.cache.get(&key) {
            return v.clone();
        }
        self.enter(&key)?;
        let result = self.value_inner(name);
        self.active.remove(&key);
        self.cache.insert(key, result.clone());
        result
    }
    fn value_inner(&mut self, name: &Name) -> Result<Resolved> {
        let usage = self
            .task
            .properties
            .get(name)
            .ok_or_else(|| err("PROPERTY_UNDECLARED", "property not declared by task"))?
            .clone();
        let p = property(self.all, &usage.property)?.clone();
        if p.category == definition::Category::Constraint {
            return Err(err(
                "CONSTRAINT_OVERRIDE",
                "constraint definitions are not execution value overrides",
            ));
        }
        let provided = self
            .request
            .overrides
            .get(&self.step.id)
            .and_then(|m| m.get(name));
        if provided.is_some()
            && (!p.overridable || !usage.sources.iter().any(|s| matches!(s, Source::Override)))
        {
            return Err(err(
                "OVERRIDE_FORBIDDEN",
                "property does not allow this override",
            ));
        }
        if let Some(default) = &usage.default {
            default.check(&p).map_err(|e| err("DEFAULT_INVALID", e))?;
        }
        for source in &usage.sources {
            let candidate = match source {
                Source::Override => {
                    provided.map(|v| Ok(origin("OVERRIDE", None, self.location(name), v.clone())))
                }
                Source::Input { key } => self
                    .request
                    .inputs
                    .get(key)
                    .map(|v| Ok(origin("INPUT", None, format!("inputs/{key}"), v.clone()))),
                Source::Default => usage.default.as_ref().map(|v| {
                    Ok(origin(
                        "DEFAULT",
                        Some(self.request.workflow.clone()),
                        format!("tasks/{}/properties/{name}/default", self.step.task),
                        v.clone(),
                    ))
                }),
                Source::Context {
                    slot,
                    field,
                    index,
                    frame_field,
                } => {
                    if self
                        .contexts
                        .get(slot)
                        .and_then(|v| v.get(usize::from(*index)))
                        .and_then(|c| c.effective.fields.get(field))
                        .is_some_and(|f| f.property != usage.property)
                    {
                        return Err(err(
                            "CONTEXT_PROPERTY_REFERENCE",
                            "source field and task property refer to different Property revisions",
                        ));
                    }
                    match self.framed_context(slot, field, *index, frame_field.as_ref()) {
                        Err(e) if matches!(e.code, "VALUE_MISSING" | "CONTEXT_MISSING") => None,
                        other => Some(other),
                    }
                }
                Source::PropertySet => {
                    let mut selected: Option<Resolved> = None;
                    for set in self.sets {
                        if let Some(v) = set.effective.values.get(name) {
                            let field = &set.effective.fields[name];
                            if field.property != usage.property {
                                return Err(err(
                                    "PROPERTY_SET_REFERENCE",
                                    "property set refers to a different property revision",
                                ));
                            }
                            let q =
                                Quantity::from_value(field.specification.unit.clone(), &v.value);
                            if selected.as_ref().is_some_and(|old| old.value != q) {
                                return Err(err(
                                    "PROPERTY_SET_CONFLICT",
                                    "selected property sets disagree",
                                ));
                            }
                            let candidate = origin(
                                "PROPERTY_SET",
                                Some(v.declared_by.clone()),
                                name.to_string(),
                                q,
                            );
                            if let Some(old) = &mut selected {
                                old.origins.extend(candidate.origins);
                            } else {
                                selected = Some(candidate);
                            }
                        }
                    }
                    selected.map(Ok)
                }
                Source::Rule { rule } => Some((|| {
                    let r = self
                        .spec
                        .rules
                        .get(rule)
                        .ok_or_else(|| err("RULE_MISSING", "named rule not found"))?;
                    if r.output != usage.property {
                        return Err(err(
                            "RULE_OUTPUT_REFERENCE",
                            "rule output does not match the property's exact revision",
                        ));
                    }
                    self.rule(rule)
                })()),
                Source::Pattern {
                    slot,
                    rule,
                    component,
                } => Some(self.pattern(slot, rule, *component)),
            };
            if let Some(candidate) = candidate {
                let result = candidate?;
                result
                    .value
                    .check(&p)
                    .map_err(|e| err("VALUE_INVALID", e))?;
                return Ok(result);
            }
        }
        Err(err(
            "VALUE_MISSING",
            "no source in the declared chain supplies a value",
        ))
    }
    fn pattern(
        &self,
        slot: &Name,
        rule: &Reference,
        component: PatternComponent,
    ) -> Result<Resolved> {
        if !self.task.contexts.contains(slot) {
            return Err(err(
                "UNDECLARED_CONTEXT",
                "task does not declare the pattern context",
            ));
        }
        let target = self
            .contexts
            .get(slot)
            .filter(|v| v.len() == 1)
            .and_then(|v| v.first())
            .ok_or_else(|| {
                err(
                    "CONTEXT_CARDINALITY",
                    "point pattern needs exactly one resource",
                )
            })?;
        let pattern = definition(self.all, rule)?;
        let page = definition::pattern::generate(
            &target.definition,
            pattern,
            &definition_graph(&target.definition, self.all)?,
            self.request.slot_index,
            1,
        )
        .map_err(|e| err("PATTERN_INVALID", e))?;
        if let Some(v) = page.violations.first() {
            return Err(err(
                "PATTERN_INVALID",
                format!("{}: {}", v.location, v.message),
            ));
        }
        let point = page
            .points
            .first()
            .ok_or_else(|| err("SLOT_MISSING", "slot index is outside the pattern"))?;
        let value =
            match component {
                PatternComponent::Position => Quantity::from_value(
                    page.unit
                        .clone()
                        .ok_or_else(|| err("UNIT_MISSING", "pattern unit missing"))?,
                    &Value::Vector(point.position.clone()),
                ),
                PatternComponent::Orientation => Quantity::from_value(
                    n("unitless"),
                    &Value::Vector(page.orientation_xyzw.clone().ok_or_else(|| {
                        err("ORIENTATION_MISSING", "pattern orientation missing")
                    })?),
                ),
                PatternComponent::Frame => Quantity::from_value(
                    n("unitless"),
                    &Value::Text(
                        page.frame
                            .clone()
                            .ok_or_else(|| err("FRAME_MISSING", "pattern frame missing"))?,
                    ),
                ),
            };
        let mut result = origin(
            "PATTERN",
            Some(rule.clone()),
            format!("slots/{}", self.request.slot_index.0),
            value,
        );
        if !matches!(component, PatternComponent::Frame) {
            result.frame = page.frame.clone();
        }
        if let Body::PointPattern {
            origin,
            orientation,
            frame,
            axes,
            ..
        } = &pattern.body
        {
            let fields = std::iter::once(origin)
                .chain(orientation.iter())
                .chain(std::iter::once(frame))
                .chain(axes.iter().flat_map(|a| [&a.count, &a.pitch]))
                .collect::<BTreeSet<_>>();
            for field in fields {
                result.origins.extend(self.context(slot, field, 0)?.origins);
            }
        }
        Ok(result)
    }
    fn operand(&mut self, operand: &Operand, path: &str) -> Result<Resolved> {
        match operand {
            Operand::Property { name } => self.value(name),
            Operand::Rule { name } => self.rule(name),
            Operand::Context {
                slot,
                field,
                index,
                frame_field,
            } => self.framed_context(slot, field, *index, frame_field.as_ref()),
            Operand::Literal { value } => {
                value
                    .validate()
                    .map_err(|e| err("RULE_LITERAL_INVALID", e))?;
                Ok(origin(
                    "RULE_LITERAL",
                    Some(self.request.workflow.clone()),
                    path.into(),
                    value.clone(),
                ))
            }
        }
    }
    fn rule(&mut self, name: &Name) -> Result<Resolved> {
        let key = format!("rule/{name}");
        if let Some(v) = self.cache.get(&key) {
            return v.clone();
        }
        self.enter(&key)?;
        let result = self.rule_inner(name);
        self.active.remove(&key);
        self.cache.insert(key, result.clone());
        result
    }
    fn rule_inner(&mut self, name: &Name) -> Result<Resolved> {
        let rule = self
            .spec
            .rules
            .get(name)
            .ok_or_else(|| err("RULE_MISSING", "rule not found"))?
            .clone();
        let operands = rule
            .operation
            .operands()
            .into_iter()
            .enumerate()
            .map(|(i, o)| self.operand(o, &format!("rules/{name}/inputs/{i}")))
            .collect::<Result<Vec<_>>>()?;
        let a = |i: usize| &operands[i].value;
        let frame = match &rule.operation {
            Operation::Offset { .. } => {
                if operands[0].frame.is_none()
                    || operands[0].frame != operands[1].frame
                    || operands[2].frame.is_some()
                {
                    return Err(err(
                        "POSE_FRAME_MISMATCH",
                        "pose components need the same explicit frame; offset distance must be a length, not an absolute coordinate",
                    ));
                }
                operands[0].frame.clone()
            }
            Operation::Component { .. } | Operation::Scale { .. } => operands[0].frame.clone(),
            _ => {
                if operands[0].frame.is_some()
                    && operands[1].frame.is_some()
                    && operands[0].frame != operands[1].frame
                {
                    return Err(err(
                        "FRAME_MISMATCH",
                        "cannot combine coordinates from different frames",
                    ));
                }
                operands[0]
                    .frame
                    .clone()
                    .or_else(|| operands[1].frame.clone())
            }
        };
        let value = match &rule.operation {
            Operation::Add { .. } => math::binary("ADD", a(0), a(1)),
            Operation::Subtract { .. } => math::binary("SUBTRACT", a(0), a(1)),
            Operation::Min { .. } => math::binary("MIN", a(0), a(1)),
            Operation::Max { .. } => math::binary("MAX", a(0), a(1)),
            Operation::Scale { .. } => math::scale(a(0), a(1)),
            Operation::Component { index, .. } => math::component(a(0), *index),
            Operation::Offset { direction, .. } => math::offset(a(0), a(1), a(2), direction),
        }
        .map_err(|e| err("RULE_VALUE_INVALID", e))?;
        value
            .check(property(self.all, &rule.output)?)
            .map_err(|e| err("RULE_OUTPUT_INVALID", e))?;
        let mut result = origin(
            "RULE",
            Some(self.request.workflow.clone()),
            format!("rules/{name}"),
            value,
        );
        result.frame = frame;
        result.origins.extend(join_origins(&operands));
        Ok(result)
    }
}
