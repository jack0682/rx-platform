use super::model::*;
use crate::types::*;

fn scalar(q: &Quantity) -> Result<&Span, String> {
    match &q.data {
        Data::Number { range } => Ok(range),
        _ => Err("numeric scalar required".into()),
    }
}
fn same_unit(a: &Quantity, b: &Quantity) -> Result<(), String> {
    if a.unit != b.unit {
        Err(format!("unit mismatch: {} versus {}", a.unit, b.unit))
    } else {
        Ok(())
    }
}
fn bounds(min: f64, max: f64, uncertain: bool) -> Result<Span, String> {
    // Point inputs use the concrete IEEE value sent to a provider. Uncertain ranges are rounded outward.
    Span::checked(
        if uncertain { min.next_down() } else { min },
        if uncertain { max.next_up() } else { max },
    )
}
pub fn binary(op: &str, a: &Quantity, b: &Quantity) -> Result<Quantity, String> {
    same_unit(a, b)?;
    let (a_range, b_range) = (scalar(a)?, scalar(b)?);
    let uncertain = a_range.min != a_range.max || b_range.min != b_range.max;
    let (lo, hi) = match op {
        "ADD" => (
            a_range.min.get() + b_range.min.get(),
            a_range.max.get() + b_range.max.get(),
        ),
        "SUBTRACT" => (
            a_range.min.get() - b_range.max.get(),
            a_range.max.get() - b_range.min.get(),
        ),
        "MIN" => (
            a_range.min.get().min(b_range.min.get()),
            a_range.max.get().min(b_range.max.get()),
        ),
        "MAX" => (
            a_range.min.get().max(b_range.min.get()),
            a_range.max.get().max(b_range.max.get()),
        ),
        _ => return Err("unsupported numeric operation".into()),
    };
    Ok(Quantity {
        unit: a.unit.clone(),
        data: Data::Number {
            range: bounds(lo, hi, uncertain)?,
        },
    })
}
fn product(a: &Span, b: &Span) -> Result<Span, String> {
    let all = [
        a.min.get() * b.min.get(),
        a.min.get() * b.max.get(),
        a.max.get() * b.min.get(),
        a.max.get() * b.max.get(),
    ];
    if all.iter().any(|v| !v.is_finite()) {
        return Err("nonfinite interval product".into());
    }
    bounds(
        all.iter().copied().fold(f64::INFINITY, f64::min),
        all.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        a.min != a.max || b.min != b.max,
    )
}
pub fn scale(value: &Quantity, factor: &Quantity) -> Result<Quantity, String> {
    if factor.unit.as_str() != "unitless" {
        return Err("scale factor must be unitless".into());
    }
    let range = product(scalar(value)?, scalar(factor)?)?;
    Ok(Quantity {
        unit: value.unit.clone(),
        data: Data::Number { range },
    })
}
pub fn component(value: &Quantity, index: u16) -> Result<Quantity, String> {
    let Data::Vector { ranges } = &value.data else {
        return Err("vector required".into());
    };
    let range = ranges
        .get(usize::from(index))
        .ok_or("vector component out of bounds")?
        .clone();
    Ok(Quantity {
        unit: value.unit.clone(),
        data: Data::Number { range },
    })
}
pub fn offset(
    position: &Quantity,
    orientation: &Quantity,
    distance: &Quantity,
    direction: &[Real; 3],
) -> Result<Quantity, String> {
    same_unit(position, distance)?;
    if !matches!(position.unit.as_str(), "mm" | "m") || orientation.unit.as_str() != "unitless" {
        return Err("pose offset needs explicit length and unitless quaternion".into());
    }
    let Data::Vector { ranges: p } = &position.data else {
        return Err("position vector required".into());
    };
    let Data::Vector { ranges: q } = &orientation.data else {
        return Err("quaternion vector required".into());
    };
    if p.len() != 3 || q.len() != 4 {
        return Err("pose vector dimensions".into());
    }
    let q = q
        .iter()
        .map(|v| v.exact().ok_or("quaternion cannot be an uncertain range"))
        .collect::<Result<Vec<_>, _>>()?;
    if (q.iter().map(|v| v * v).sum::<f64>() - 1.0).abs() > 1e-9
        || (direction.iter().map(|v| v.get().powi(2)).sum::<f64>() - 1.0).abs() > 1e-9
    {
        return Err("unit quaternion and unit direction required".into());
    }
    let [x, y, z, w] = [q[0], q[1], q[2], q[3]];
    let [a, b, c] = [direction[0].get(), direction[1].get(), direction[2].get()];
    let t = [
        2.0 * (y * c - z * b),
        2.0 * (z * a - x * c),
        2.0 * (x * b - y * a),
    ];
    let axes = [
        a + w * t[0] + y * t[2] - z * t[1],
        b + w * t[1] + z * t[0] - x * t[2],
        c + w * t[2] + x * t[1] - y * t[0],
    ];
    let distance = scalar(distance)?;
    let ranges = p
        .iter()
        .zip(axes)
        .map(|(p, axis)| {
            let d = product(
                distance,
                &Span::point(Real::new(axis).map_err(|e| e.to_string())?),
            )?;
            bounds(
                p.min.get() + d.min.get(),
                p.max.get() + d.max.get(),
                p.min != p.max || d.min != d.max,
            )
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Quantity {
        unit: position.unit.clone(),
        data: Data::Vector { ranges },
    })
}
pub fn compare(relation: Relation, a: &Quantity, b: &Quantity) -> Result<bool, String> {
    same_unit(a, b)?;
    Ok(match (&a.data, &b.data) {
        (Data::Number { range: a }, Data::Number { range: b }) => match relation {
            Relation::Le => a.max <= b.min,
            Relation::Ge => a.min >= b.max,
            Relation::Eq => a.min == a.max && b.min == b.max && a.min == b.min,
        },
        (Data::Boolean { value: a }, Data::Boolean { value: b })
            if matches!(relation, Relation::Eq) =>
        {
            a == b
        }
        (Data::Text { value: a }, Data::Text { value: b }) if matches!(relation, Relation::Eq) => {
            a == b
        }
        (Data::Vector { ranges: a }, Data::Vector { ranges: b })
            if matches!(relation, Relation::Eq) =>
        {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|(a, b)| a.min == a.max && b.min == b.max && a.min == b.min)
        }
        _ => return Err("constraint operand types or relation differ".into()),
    })
}
