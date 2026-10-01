//! Bounded Cartesian points from pinned declarations. No equipment-specific geometry or I/O.
use super::*;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Axis {
    pub count: Name,
    pub pitch: Name,
    pub direction: Vec<Real>,
}
pub fn validate_axes(axes: &[Axis]) -> Result<(), String> {
    if axes.is_empty() || axes.len() > 3 {
        return Err("point pattern requires one to three axes".into());
    }
    let mut names = BTreeSet::new();
    for (i, axis) in axes.iter().enumerate() {
        if !names.insert(&axis.count)
            || axis.direction.len() != 3
            || (axis.direction.iter().map(|v| v.get().powi(2)).sum::<f64>() - 1.0).abs() > 1e-9
            || axes[..i].iter().any(|other| {
                axis.direction
                    .iter()
                    .zip(&other.direction)
                    .map(|(a, b)| a.get() * b.get())
                    .sum::<f64>()
                    .abs()
                    > 1e-9
            })
        {
            return Err("point pattern requires distinct counts and orthonormal directions".into());
        }
    }
    Ok(())
}
pub fn validate_fields(
    e: &Effective,
    origin: &Name,
    orientation: Option<&Name>,
    frame: &Name,
    axes: &[Axis],
) -> Result<(), String> {
    validate_axes(axes)?;
    let field = |key: &Name| {
        e.fields
            .get(key)
            .map(|v| &v.specification)
            .ok_or_else(|| format!("{key}: pattern field is not declared"))
    };
    let position = field(origin)?;
    if position.value_type != ValueType::Vector
        || position.vector_length != Some(3)
        || !matches!(position.unit.as_str(), "mm" | "m")
    {
        return Err(format!(
            "{origin}: expected three-dimensional length in mm or m"
        ));
    }
    if field(frame)?.value_type != ValueType::Text || field(frame)?.unit.as_str() != "unitless" {
        return Err(format!("{frame}: expected coordinate frame text"));
    }
    if let Some(key) = orientation {
        let q = field(key)?;
        if q.value_type != ValueType::Vector
            || q.vector_length != Some(4)
            || q.unit.as_str() != "unitless"
        {
            return Err(format!("{key}: expected unitless quaternion [x,y,z,w]"));
        }
    }
    for axis in axes {
        let count = field(&axis.count)?;
        let pitch = field(&axis.pitch)?;
        if count.value_type != ValueType::Number || count.unit.as_str() != "unitless" {
            return Err(format!("{}: expected unitless axis count", axis.count));
        }
        if pitch.value_type != ValueType::Number || pitch.unit != position.unit {
            return Err(format!("{}: pitch and origin units differ", axis.pitch));
        }
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    pub subject: Reference,
    pub rule: Reference,
    pub offset: Counter,
    pub limit: u16,
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
pub struct Point {
    pub index: Counter,
    pub indices: Vec<Counter>,
    pub position: Vec<Real>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub subject: Reference,
    pub rule: Reference,
    pub inputs: Effective,
    pub unit: Option<Name>,
    pub frame: Option<String>,
    pub orientation_xyzw: Option<Vec<Real>>,
    pub total: Counter,
    pub offset: Counter,
    pub points: Vec<Point>,
    pub next: Option<Counter>,
    pub violations: Vec<Violation>,
}
impl Page {
    fn issue(&mut self, field: &Name, code: &str, message: &str) {
        self.violations.push(Violation {
            location: format!("definitions/{}/values/{field}", self.subject.id),
            code: code.into(),
            message: message.into(),
        });
    }
}
pub fn generate(
    subject: &Definition,
    rule: &Definition,
    dependencies: &BTreeMap<Reference, Definition>,
    offset: Counter,
    limit: u16,
) -> Result<Page, String> {
    if limit == 0 || limit > 100 || offset.0 > 100_000 {
        return Err("point page bounds".into());
    }
    subject.verify()?;
    rule.verify()?;
    if subject.reference.catalog != rule.reference.catalog
        || !matches!(
            subject.body.kind(),
            Kind::ResourceModel | Kind::ResourceInstance
        )
    {
        return Err(
            "point subject must be a resource model or instance in the same catalog".into(),
        );
    }
    let Body::PointPattern {
        resource_type,
        origin,
        orientation,
        frame,
        axes,
    } = &rule.body
    else {
        return Err("point pattern definition required".into());
    };
    // Only the subject's own ancestry may establish the accepted type, not unrelated lookup entries.
    let mut cursor = subject;
    let mut seen = BTreeSet::new();
    loop {
        if &cursor.reference == resource_type {
            break;
        }
        if !seen.insert(cursor.reference.id.clone()) || seen.len() > 32 {
            return Err("point subject ancestry is cyclic or too deep".into());
        }
        let parent = match &cursor.body {
            Body::ResourceModel { resource_type, .. } => Some(resource_type),
            Body::ResourceInstance { base, .. } => Some(base),
            Body::ResourceType { parent, .. } => parent.as_ref(),
            _ => None,
        }
        .ok_or("point subject does not inherit the rule's accepted type revision")?;
        cursor = dependencies
            .get(parent)
            .ok_or("point subject ancestor missing")?;
    }
    let inputs = resolve(subject, dependencies)?;
    validate_fields(&inputs, origin, orientation.as_ref(), frame, axes)?;
    let mut page = Page {
        subject: subject.reference.clone(),
        rule: rule.reference.clone(),
        inputs,
        unit: None,
        frame: None,
        orientation_xyzw: None,
        total: Counter(0),
        offset,
        points: vec![],
        next: None,
        violations: vec![],
    };
    let position = match page.inputs.values.get(origin).map(|v| &v.value) {
        Some(Value::Vector(v)) if v.len() == 3 => Some(v.clone()),
        _ => {
            page.issue(
                origin,
                "MISSING_ORIGIN",
                "Three-dimensional origin is required",
            );
            None
        }
    };
    match page.inputs.values.get(frame).map(|v| &v.value) {
        Some(Value::Text(v)) if !v.trim().is_empty() => page.frame = Some(v.clone()),
        _ => page.issue(
            frame,
            "MISSING_FRAME",
            "Explicit coordinate frame is required",
        ),
    }
    match orientation
        .as_ref()
        .and_then(|key| page.inputs.values.get(key))
        .map(|v| &v.value)
    {
        Some(Value::Vector(q))
            if q.len() == 4
                && (q.iter().map(|v| v.get().powi(2)).sum::<f64>() - 1.0).abs() <= 1e-9 =>
        {
            page.orientation_xyzw = Some(q.clone());
        }
        _ => page.issue(
            orientation.as_ref().unwrap_or(origin),
            "ORIENTATION_REQUIRED",
            "Explicit unit quaternion [x,y,z,w] is required; identity is not assumed",
        ),
    }
    let mut dimensions = vec![];
    for axis in axes {
        let count = match page.inputs.values.get(&axis.count).map(|v| &v.value) {
            Some(Value::Number(v))
                if v.get() >= 1.0 && v.get() <= 100_000.0 && v.get().fract() == 0.0 =>
            {
                Some(v.get() as u64)
            }
            _ => {
                page.issue(
                    &axis.count,
                    "AXIS_COUNT",
                    "Count must be an integer from 1 to 100000",
                );
                None
            }
        };
        let pitch = match page.inputs.values.get(&axis.pitch).map(|v| &v.value) {
            Some(Value::Number(v)) if v.get() > 0.0 => Some(v.get()),
            _ => {
                page.issue(&axis.pitch, "AXIS_PITCH", "Positive pitch is required");
                None
            }
        };
        if let (Some(count), Some(pitch)) = (count, pitch) {
            dimensions.push((count, pitch));
        }
    }
    if !page.violations.is_empty() {
        return Ok(page);
    }
    let total = dimensions
        .iter()
        .try_fold(1u64, |n, (count, _)| n.checked_mul(*count));
    let Some(total) = total.filter(|v| *v <= 100_000) else {
        page.issue(origin, "POINT_LIMIT", "Pattern exceeds 100000 points");
        return Ok(page);
    };
    if offset.0 > total {
        return Err("point offset exceeds pattern size".into());
    }
    let position = position.ok_or("origin missing")?;
    let q = page
        .orientation_xyzw
        .as_ref()
        .ok_or("orientation missing")?;
    let directions = axes
        .iter()
        .map(|axis| rotate(q, &axis.direction))
        .collect::<Vec<_>>();
    // Check every extreme once, even when the caller asks for the first page only.
    for (coordinate, start) in position.iter().enumerate() {
        let bound = directions
            .iter()
            .zip(&dimensions)
            .map(|(axis, (count, pitch))| (*count - 1) as f64 * pitch * axis[coordinate].abs())
            .sum::<f64>();
        if !bound.is_finite() || !(start.get().abs() + bound).is_finite() {
            page.issue(
                origin,
                "POINT_OVERFLOW",
                "Worst-case coordinate is not finite",
            );
            return Ok(page);
        }
    }
    page.unit = Some(page.inputs.fields[origin].specification.unit.clone());
    page.total = Counter(total);
    let end = total.min(offset.0 + u64::from(limit));
    for index in offset.0..end {
        let mut remainder = index;
        let mut indices = vec![Counter(0); axes.len()];
        for i in (0..axes.len()).rev() {
            indices[i] = Counter(remainder % dimensions[i].0);
            remainder /= dimensions[i].0;
        }
        let point = position
            .iter()
            .enumerate()
            .map(|(coordinate, start)| {
                Real::new(
                    start.get()
                        + directions
                            .iter()
                            .enumerate()
                            .map(|(i, axis)| {
                                indices[i].0 as f64 * dimensions[i].1 * axis[coordinate]
                            })
                            .sum::<f64>(),
                )
                .map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        page.points.push(Point {
            index: Counter(index),
            indices,
            position: point,
        });
    }
    page.next = (end < total).then_some(Counter(end));
    Ok(page)
}

// Rotate local pattern vectors into the declared parent frame using a unit quaternion.
fn rotate(q: &[Real], v: &[Real]) -> [f64; 3] {
    let [x, y, z, w] = [q[0].get(), q[1].get(), q[2].get(), q[3].get()];
    let [a, b, c] = [v[0].get(), v[1].get(), v[2].get()];
    let t = [
        2.0 * (y * c - z * b),
        2.0 * (z * a - x * c),
        2.0 * (x * b - y * a),
    ];
    [
        a + w * t[0] + y * t[2] - z * t[1],
        b + w * t[1] + z * t[0] - x * t[2],
        c + w * t[2] + x * t[1] - y * t[0],
    ]
}
