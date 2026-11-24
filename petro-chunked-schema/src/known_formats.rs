//! Built-in YAML schema definitions for known Petroglyph chunked formats (`Alo`, `AloModel`,
//! `AloParticle`, and `AlaAnimation`).
//!
//! Available when the `known-formats` crate feature is enabled (enabled by default).

use crate::schema::Schema;

/// Canonical names of all built-in schema definitions in display order.
pub const BUILTIN_NAMES: &[&str] = &["Alo", "AloModel", "AloParticle", "AlaAnimation"];

/// Embedded YAML definition for the unified Alamo `.alo` format (dispatching between 3D models
/// and particle systems).
pub const ALO_YAML: &str = include_str!("../schemas/alo.yaml");

/// Embedded YAML definition for Alamo 3D Model (`.alo`) files.
pub const ALO_MODEL_YAML: &str = include_str!("../schemas/alo_model.yaml");

/// Embedded YAML definition for Alamo Particle System (`.alo`) files.
pub const ALO_PARTICLE_YAML: &str = include_str!("../schemas/alo_particle.yaml");

/// Embedded YAML definition for Alamo Animation (`.ala`) files.
pub const ALA_ANIMATION_YAML: &str = include_str!("../schemas/ala_animation.yaml");

/// Returns the parsed [`Schema`] for the combined Alamo `.alo` format.
pub fn alo() -> Schema {
    Schema::from_yaml_str(ALO_YAML).expect("built-in alo.yaml must be valid")
}

/// Returns the parsed [`Schema`] for Alamo 3D Model (`.alo`) files.
pub fn alo_model() -> Schema {
    Schema::from_yaml_str(ALO_MODEL_YAML).expect("built-in alo_model.yaml must be valid")
}

/// Returns the parsed [`Schema`] for Alamo Particle System (`.alo`) files.
pub fn alo_particle() -> Schema {
    Schema::from_yaml_str(ALO_PARTICLE_YAML).expect("built-in alo_particle.yaml must be valid")
}

/// Returns the parsed [`Schema`] for Alamo Animation (`.ala`) files.
pub fn ala_animation() -> Schema {
    Schema::from_yaml_str(ALA_ANIMATION_YAML).expect("built-in ala_animation.yaml must be valid")
}

/// Returns the canonical built-in format name if `name` matches one of [`BUILTIN_NAMES`]
/// (accepting both PascalCase and snake_case aliases, case-insensitively).
pub fn canonical_name(name: &str) -> Option<&'static str> {
    if name.eq_ignore_ascii_case("Alo") {
        Some("Alo")
    } else if name.eq_ignore_ascii_case("AloModel") || name.eq_ignore_ascii_case("alo_model") {
        Some("AloModel")
    } else if name.eq_ignore_ascii_case("AloParticle") || name.eq_ignore_ascii_case("alo_particle")
    {
        Some("AloParticle")
    } else if name.eq_ignore_ascii_case("AlaAnimation")
        || name.eq_ignore_ascii_case("ala_animation")
    {
        Some("AlaAnimation")
    } else {
        None
    }
}

/// Loads a built-in [`Schema`] by name or snake_case alias (case-insensitively).
pub fn by_name(name: &str) -> Option<Schema> {
    match canonical_name(name)? {
        "Alo" => Some(alo()),
        "AloModel" => Some(alo_model()),
        "AloParticle" => Some(alo_particle()),
        "AlaAnimation" => Some(ala_animation()),
        _ => None,
    }
}

/// Returns all built-in known [`Schema`] definitions.
pub fn all() -> Vec<Schema> {
    vec![alo(), alo_model(), alo_particle(), ala_animation()]
}
