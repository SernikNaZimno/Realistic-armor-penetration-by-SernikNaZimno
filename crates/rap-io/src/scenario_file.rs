//! The scenario file: one impact, how to simulate it, and any materials it
//! defines itself, stored as JSON.

use std::fs;
use std::path::Path;

use rap_domain::{Scenario, SimulationSettings};
use rap_materials::Material;
use serde::{Deserialize, Serialize};

use crate::IoError;

/// Version of the scenario file format written by this program.
pub const FORMAT_VERSION: u32 = 1;

/// Contents of a scenario file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioFile {
    /// Version of the file format; see [`FORMAT_VERSION`].
    pub format_version: u32,
    /// The impact to simulate.
    pub scenario: Scenario,
    /// Numerical settings; defaults apply to whatever is left out.
    #[serde(default)]
    pub settings: SimulationSettings,
    /// Materials defined by the file. They are added to the built-in
    /// library and replace built-in materials with the same identifier.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<Material>,
}

/// The part of a scenario file that is read before anything else.
#[derive(Deserialize)]
struct Header {
    format_version: u32,
}

impl ScenarioFile {
    /// Wraps `scenario` with default settings and no custom materials.
    #[must_use]
    pub fn new(scenario: Scenario) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            scenario,
            settings: SimulationSettings::default(),
            materials: Vec::new(),
        }
    }

    /// Parses the JSON text of a scenario file.
    ///
    /// # Errors
    /// Returns [`IoError::UnsupportedVersion`] if the file was written in
    /// another format version and [`IoError::Parse`] if it is malformed.
    pub fn from_json(text: &str) -> Result<Self, IoError> {
        let header: Header = serde_json::from_str(text)?;
        if header.format_version != FORMAT_VERSION {
            return Err(IoError::UnsupportedVersion {
                found: header.format_version,
            });
        }
        Ok(serde_json::from_str(text)?)
    }

    /// The scenario file as indented JSON text.
    ///
    /// # Errors
    /// Returns [`IoError::Parse`] if a value cannot be represented in JSON,
    /// which happens only for non-finite numbers.
    pub fn to_json(&self) -> Result<String, IoError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Reads a scenario file.
    ///
    /// # Errors
    /// Returns [`IoError`] if the file cannot be read or is not a valid
    /// scenario of a supported version.
    pub fn load(path: &Path) -> Result<Self, IoError> {
        let text = fs::read_to_string(path).map_err(|source| IoError::Read {
            path: path.to_owned(),
            source,
        })?;
        Self::from_json(&text)
    }

    /// Writes the scenario file, replacing any existing file.
    ///
    /// # Errors
    /// Returns [`IoError`] if the file cannot be written.
    pub fn save(&self, path: &Path) -> Result<(), IoError> {
        fs::write(path, self.to_json()?).map_err(|source| IoError::Write {
            path: path.to_owned(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use rap_domain::material_id::STEEL_4340;
    use rap_domain::{
        ApfsdsProjectile, Armor, ArmorLayer, MaterialId, Projectile, Scenario, SimulationSettings,
    };
    use rap_materials::library;

    use super::{FORMAT_VERSION, ScenarioFile};
    use crate::IoError;

    fn scenario() -> Scenario {
        Scenario {
            projectile: Projectile::Apfsds(ApfsdsProjectile::generic_120mm()),
            armor: Armor {
                layers: vec![ArmorLayer {
                    material: MaterialId::new(STEEL_4340),
                    thickness: 0.25,
                    gap_before: 0.0,
                }],
                obliquity: 0.0,
            },
            impact_speed: 1650.0,
        }
    }

    #[test]
    fn file_survives_saving_and_loading() {
        let mut file = ScenarioFile::new(scenario());
        file.settings.cells_per_radius = 6.0;
        file.settings.end_time = Some(4.0e-4);
        let mut custom = library::steel_4340();
        custom.id = MaterialId::new("hard-steel");
        file.materials.push(custom);

        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("scenario.json");
        file.save(&path).expect("the file is written");
        let loaded = ScenarioFile::load(&path).expect("the file is read");
        // The cone half-angle passes through degrees.
        assert_eq!(loaded.settings, file.settings);
        assert_eq!(loaded.materials, file.materials);
        assert_eq!(loaded.scenario.armor, file.scenario.armor);
        assert_eq!(loaded.format_version, FORMAT_VERSION);
    }

    #[test]
    fn settings_and_materials_are_optional() {
        let text = ScenarioFile::new(scenario())
            .to_json()
            .expect("serializable");
        assert!(!text.contains("\"materials\""));
        let mut value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
        value.as_object_mut().expect("an object").remove("settings");
        let file = ScenarioFile::from_json(&value.to_string()).expect("a valid file");
        assert_eq!(file.settings, SimulationSettings::default());
        assert_eq!(file.materials, Vec::new());
    }

    #[test]
    fn other_format_versions_are_refused() {
        let text = ScenarioFile::new(scenario())
            .to_json()
            .expect("serializable");
        let newer = text.replace("\"format_version\": 1", "\"format_version\": 2");
        assert!(matches!(
            ScenarioFile::from_json(&newer),
            Err(IoError::UnsupportedVersion { found: 2 })
        ));
    }

    #[test]
    fn malformed_and_missing_files_are_reported() {
        assert!(matches!(
            ScenarioFile::from_json("{ not json"),
            Err(IoError::Parse(_))
        ));
        let directory = tempfile::tempdir().expect("a temporary directory");
        let missing = directory.path().join("missing.json");
        let error = ScenarioFile::load(&missing).unwrap_err();
        assert!(matches!(error, IoError::Read { .. }));
        assert!(error.to_string().contains("missing.json"));
    }
}
