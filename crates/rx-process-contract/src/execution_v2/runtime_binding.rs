//! Pure runtime binding checks over approved inputs; these values confer no authority.
use super::*;
use rx_domain::{definition as d, workflow};

#[derive(Clone, Debug, Serialize)]
pub struct ObjectProjection {
    pub instance: Reference,
    pub model: Reference,
    pub context: Name,
    pub candidate: u8,
    pub values_digest: Digest,
    pub effective: d::Effective,
}
fn values_digest(effective: &d::Effective) -> Result<Digest, String> {
    if !effective.missing.is_empty() {
        return Err("actual object has missing required values".into());
    }
    let fields: BTreeMap<_, _> = effective
        .fields
        .iter()
        .map(|(key, field)| (key, (&field.property, &field.specification, field.required)))
        .collect();
    let values: BTreeMap<_, _> = effective
        .values
        .iter()
        .map(|(key, value)| (key, &value.value))
        .collect();
    canonical::digest("RX-EXECUTION-OBJECT-VALUES-v2", &(fields, values)).map_err(|e| e.to_string())
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotResource {
    pub resource: Reference,
    pub rule: Reference,
    pub contexts: Vec<Name>,
    pub count: Counter,
    pub layout_digest: Digest,
}
impl InputClosure {
    /// Caller must load the current instance and check currentness atomically with admission.
    pub fn object_projection(
        &self,
        policy: &Policy,
        instance: &d::Definition,
    ) -> Result<ObjectProjection, String> {
        policy.validate()?;
        let all = self.verify(policy)?;
        instance.verify()?;
        let d::Body::ObjectInstance { base, .. } = &instance.body else {
            return Err("runtime binding requires an actual ObjectInstance".into());
        };
        if instance.reference.catalog != policy.workflow.catalog {
            return Err("actual object catalog differs".into());
        }
        let mut candidates = policy
            .candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| &c.object_model == base);
        let (candidate, _) = candidates
            .next()
            .ok_or("actual object model is not approved")?;
        if candidates.next().is_some() {
            return Err("actual object model selects ambiguous candidates".into());
        }
        let mut contexts = self.spec.defaults.clone();
        contexts.extend(self.requests[candidate].contexts.clone());
        let matching: Vec<_> = contexts
            .iter()
            .filter(|(key, refs)| {
                refs.as_slice() == std::slice::from_ref(base)
                    && self
                        .spec
                        .contexts
                        .get(*key)
                        .is_some_and(|s| s.kind == d::SlotKind::Object)
            })
            .map(|(key, _)| key.clone())
            .collect();
        if matching.len() != 1 {
            return Err("actual object context is missing or ambiguous".into());
        }
        let model = all.get(base).ok_or("approved object model missing")?;
        let effective = d::resolve(instance, &all)?;
        let model_values = d::resolve(model, &all)?;
        let digest = values_digest(&effective)?;
        if digest != values_digest(&model_values)? {
            return Err("actual object values differ from the approved model".into());
        }
        Ok(ObjectProjection {
            instance: instance.reference.clone(),
            model: base.clone(),
            context: matching[0].clone(),
            candidate: candidate as u8,
            values_digest: digest,
            effective,
        })
    }
    /// Discover every declared pattern resource; no domain-specific context names or skipped sources.
    pub fn slot_resources(&self, policy: &Policy) -> Result<Vec<SlotResource>, String> {
        policy.validate()?;
        let all = self.verify(policy)?;
        let mut declarations = BTreeSet::new();
        for step in &self.spec.steps {
            let task = self
                .spec
                .tasks
                .get(&step.task)
                .ok_or("active task definition missing")?;
            for usage in task.properties.values() {
                for source in &usage.sources {
                    if let workflow::Source::Pattern { slot, rule, .. } = source {
                        declarations.insert((slot.clone(), rule.clone()));
                    }
                }
            }
        }
        if declarations.is_empty() {
            return Err("execution profile requires pattern-driven resource slots".into());
        }
        let mut resources: BTreeMap<(Id, Id), SlotResource> = BTreeMap::new();
        for (context, rule_ref) in declarations {
            if !self
                .spec
                .contexts
                .get(&context)
                .is_some_and(|s| s.kind == d::SlotKind::Resource && !s.multiple)
            {
                return Err("slot pool requires one resource context".into());
            }
            let mut reference = None;
            for request in &self.requests {
                let refs = request
                    .contexts
                    .get(&context)
                    .or_else(|| self.spec.defaults.get(&context))
                    .ok_or("slot resource context missing")?;
                if refs.len() != 1 || reference.as_ref().is_some_and(|r| r != &refs[0]) {
                    return Err("slot resource differs or is ambiguous across candidates".into());
                }
                reference = Some(refs[0].clone());
            }
            let resource = reference.ok_or("slot resource missing")?;
            let subject = all
                .get(&resource)
                .ok_or("slot resource definition missing")?;
            if !matches!(subject.body, d::Body::ResourceInstance { .. }) {
                return Err("slot custody requires a ResourceInstance, not a model".into());
            }
            let rule = all.get(&rule_ref).ok_or("slot pattern missing")?;
            let page = d::pattern::generate(subject, rule, &all, Counter(0), 1)?;
            if !page.violations.is_empty() || page.points.len() != 1 {
                return Err("slot layout has unresolved or invalid geometry".into());
            }
            if page.total.0 == 0
                || page.total.0 > MAX_SLOTS as u64
                || policy.slot_order.len() as u64 > page.total.0
            {
                return Err("slot pool capacity is outside the execution profile".into());
            }
            let layout_digest = canonical::digest(
                "RX-EXECUTION-SLOT-LAYOUT-v2",
                &(&resource, &rule_ref, page.total, policy.resolver_digest),
            )
            .map_err(|e| e.to_string())?;
            let key = (resource.catalog.clone(), resource.id.clone());
            if let Some(previous) = resources.get_mut(&key) {
                if previous.resource != resource
                    || previous.rule != rule_ref
                    || previous.layout_digest != layout_digest
                {
                    return Err("one resource instance has competing slot layouts".into());
                }
                previous.contexts.push(context);
            } else {
                resources.insert(
                    key,
                    SlotResource {
                        resource,
                        rule: rule_ref,
                        contexts: vec![context],
                        count: page.total,
                        layout_digest,
                    },
                );
            }
        }
        Ok(resources.into_values().collect())
    }
}
