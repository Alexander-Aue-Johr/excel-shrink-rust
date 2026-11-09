//! Observe explicit VML point geometry without qualifying the shape's rendering.
use super::super::capture_layout::Geometry;
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum Observation {
    Absent,
    ParsedPoints { geometry: Geometry },
    Unsupported { reason: String },
}

pub(super) fn observe(style: Option<&str>) -> Observation {
    let Some(style) = style else {
        return Observation::Absent;
    };
    match parse(style) {
        Ok(geometry) => Observation::ParsedPoints { geometry },
        Err(error) => Observation::Unsupported {
            reason: error.to_string(),
        },
    }
}

fn points(value: &str) -> Result<f64> {
    let number = if let Some(number) = value.strip_suffix("pt") {
        number.parse::<f64>()?
    } else {
        ensure!(
            value.parse::<f64>().ok() == Some(0.0),
            "Unsupported VML length unit"
        );
        0.0
    };
    ensure!(
        number.is_finite() && number >= 0.0,
        "Invalid VML point length"
    );
    Ok(number)
}

fn parse(style: &str) -> Result<Geometry> {
    let mut properties = BTreeMap::new();
    for declaration in style
        .split(';')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let (key, value) = declaration
            .split_once(':')
            .context("Malformed VML style declaration")?;
        ensure!(
            properties.insert(key.trim(), value.trim()).is_none(),
            "Duplicate VML style property"
        );
    }
    for (key, value) in &properties {
        match *key {
            "position" => ensure!(*value == "absolute", "Unsupported VML position"),
            "margin-left" | "margin-top" | "width" | "height" => {}
            "z-index" => {
                value
                    .parse::<u32>()
                    .context("Unsupported VML stacking order")?;
            }
            "mso-wrap-style" => ensure!(*value == "tight", "Unsupported VML wrap style"),
            _ => anyhow::bail!("Unsupported VML style property {key}"),
        }
    }
    ensure!(
        properties.get("position") == Some(&"absolute"),
        "Missing absolute VML position"
    );
    let length = |name| points(properties.get(name).context("Missing VML frame length")?);
    let geometry = Geometry {
        left: length("margin-left")?,
        top: length("margin-top")?,
        width: length("width")?,
        height: length("height")?,
    }
    .validate()?;
    ensure!(
        (geometry.left + geometry.width).is_finite()
            && (geometry.top + geometry.height).is_finite(),
        "Overflowing VML frame"
    );
    Ok(geometry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_point_frames_preserve_observed_native_coordinates() -> Result<()> {
        let pdf = parse("position:absolute;margin-left:65.25pt;margin-top:114.75pt;width:114pt;height:85.5pt;z-index:1;mso-wrap-style:tight")?;
        assert_eq!(
            pdf,
            Geometry {
                left: 65.25,
                top: 114.75,
                width: 114.0,
                height: 85.5,
            }
        );
        let word = parse(
            "position:absolute;margin-left:0;margin-top:0;width:443.25pt;height:642pt;z-index:1",
        )?;
        assert_eq!(
            word,
            Geometry {
                left: 0.0,
                top: 0.0,
                width: 443.25,
                height: 642.0,
            }
        );
        Ok(())
    }

    #[test]
    fn ambiguity_effects_units_and_invalid_lengths_remain_unsupported() {
        let base = "position:absolute;margin-left:0;margin-top:0;width:114pt;height:85.5pt";
        for style in [
            format!("{base};width:114pt"),
            format!("{base};rotation:1"),
            format!("{base};unknown:1"),
            base.replace("114pt", "114px"),
            base.replace("114pt", "50%"),
            base.replace("114pt", "NaNpt"),
            base.replace("114pt", "-1pt"),
            base.replace("position:absolute;", ""),
        ] {
            assert!(
                matches!(observe(Some(&style)), Observation::Unsupported { .. }),
                "{style}"
            );
        }
        assert!(matches!(observe(None), Observation::Absent));
    }
}
