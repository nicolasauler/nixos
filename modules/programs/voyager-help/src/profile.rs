use serde::Deserialize;

pub const PRODUCT_PREFIX: &str = "Voyager nic-swap-v1:";

// Compiled from the same Nix profile used to label the firmware's USB product.
// The Oryx serial stays unchanged; this identifies a known build, not hardware
// attestation against a malicious device that copies the descriptor.
const MANIFEST: &str = include_str!(env!("VOYAGER_PROFILE_JSON"));

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct SwapProfile {
    pub schema: u8,
    pub layout: String,
    pub revision: String,
    pub swap_layer: usize,
    pub nav_layer: Option<usize>,
    pub alias_layer: Option<usize>,
    pub position_map: Vec<usize>,
}

impl SwapProfile {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1 || self.swap_layer >= 32 {
            return Err("Unsupported custom Voyager profile schema or mirror layer".into());
        }
        match (self.nav_layer, self.alias_layer) {
            (None, None) => {}
            (Some(nav), Some(alias))
                if nav < 32
                    && alias < 32
                    && nav != alias
                    && nav != self.swap_layer
                    && alias != self.swap_layer => {}
            _ => {
                return Err(
                    "Custom Voyager profile has inconsistent navigation alias layers".into(),
                );
            }
        }
        if self.position_map.len() != 52 {
            return Err("Custom Voyager profile must map all 52 physical positions".into());
        }
        for (from, &to) in self.position_map.iter().enumerate() {
            if to >= 52 || self.position_map[to] != from {
                return Err("Custom Voyager hand swap is not a reversible position map".into());
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct Manifest {
    fingerprint: String,
    product: String,
    profile: SwapProfile,
}

fn select_from(
    manifest: &str,
    product: &str,
    layout: &str,
    revision: &str,
) -> Result<Option<SwapProfile>, String> {
    if product == "Voyager" {
        return Ok(None);
    }
    let known: Manifest = serde_json::from_str(manifest)
        .map_err(|error| format!("Invalid compiled custom-firmware profile: {error}"))?;
    if known.fingerprint.len() != 64
        || !known
            .fingerprint
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || known.product != format!("{PRODUCT_PREFIX}{}", known.fingerprint)
    {
        return Err("Compiled custom-firmware identity is inconsistent".into());
    }
    if product != known.product {
        return Err(format!(
            "Unrecognized Voyager firmware product {product:?}; use the reference packaged with that custom build, not a stock Oryx interpretation"
        ));
    }
    known.profile.validate()?;
    if known.profile.layout != layout || known.profile.revision != revision {
        return Err(format!(
            "Custom firmware tag does not match reported Oryx revision {layout}/{revision}"
        ));
    }
    Ok(Some(known.profile))
}

pub fn select(product: &str, layout: &str, revision: &str) -> Result<Option<SwapProfile>, String> {
    select_from(MANIFEST, product, layout, revision)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manifest() -> (String, String) {
        let fingerprint = "a".repeat(64);
        let product = format!("{PRODUCT_PREFIX}{fingerprint}");
        let value = json!({
            "fingerprint": fingerprint, "product": product,
            "profile": {"schema":1,"layout":"layout","revision":"revision",
                "swap_layer":6,"nav_layer":4,"alias_layer":7,
                "position_map": (0..52).rev().collect::<Vec<_>>()}
        });
        (value.to_string(), product)
    }

    #[test]
    fn stock_and_known_custom_builds_do_not_share_semantics() {
        let (manifest, product) = manifest();
        assert_eq!(
            select_from(&manifest, "Voyager", "layout", "revision").unwrap(),
            None
        );
        let profile = select_from(&manifest, &product, "layout", "revision")
            .unwrap()
            .unwrap();
        assert_eq!(profile.swap_layer, 6);
        assert_eq!(profile.alias_layer, Some(7));
        assert!(
            select_from(
                &manifest,
                &format!("{PRODUCT_PREFIX}{}", "b".repeat(64)),
                "layout",
                "revision"
            )
            .is_err()
        );
        assert!(select_from(&manifest, &product, "layout", "other").is_err());
        assert!(select_from(&manifest, "Voyager renamed custom", "layout", "revision").is_err());
    }

    #[test]
    fn malformed_maps_and_aliases_cannot_be_applied() {
        let (manifest, product) = manifest();
        let mut value: serde_json::Value = serde_json::from_str(&manifest).unwrap();
        value["profile"]["position_map"][0] = json!(52);
        assert!(select_from(&value.to_string(), &product, "layout", "revision").is_err());
        value["profile"]["position_map"][0] = json!(51);
        value["profile"]["alias_layer"] = json!(6);
        assert!(select_from(&value.to_string(), &product, "layout", "revision").is_err());
        value["profile"]["alias_layer"] = json!(7);
        value["profile"]["schema"] = json!(2);
        assert!(select_from(&value.to_string(), &product, "layout", "revision").is_err());
    }
}
