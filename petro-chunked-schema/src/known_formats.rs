//! Built-in YAML schema definitions for known Petroglyph chunked formats (`alo_model`,
//! `alo_particle`, and `ala_animation`).
//!
//! Available when the `known-formats` crate feature is enabled.

use crate::schema::Schema;

/// Embedded YAML definition for Alamo 3D Model (`.alo`) files.
pub const ALO_MODEL_YAML: &str = include_str!("../schemas/alo_model.yaml");

/// Embedded YAML definition for Alamo Particle System (`.alo`) files.
pub const ALO_PARTICLE_YAML: &str = include_str!("../schemas/alo_particle.yaml");

/// Embedded YAML definition for Alamo Animation (`.ala`) files.
pub const ALA_ANIMATION_YAML: &str = include_str!("../schemas/ala_animation.yaml");

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

/// Returns all built-in known [`Schema`] definitions.
pub fn all() -> Vec<Schema> {
    vec![alo_model(), alo_particle(), ala_animation()]
}
