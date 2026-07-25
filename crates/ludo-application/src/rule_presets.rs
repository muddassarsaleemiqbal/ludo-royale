//! Named custom-rule configuration.

use ludo_domain::{DomainError, Rules};
use serde::{Deserialize, Serialize};

/// Current custom-rule schema.
pub const RULE_PRESET_SCHEMA_VERSION: u16 = 1;

/// Portable named rule configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedRulePreset {
    /// Import/export schema.
    pub schema_version: u16,
    /// User-facing unique name.
    pub name: String,
    /// Validated engine switches.
    pub rules: Rules,
}

impl NamedRulePreset {
    /// Creates and validates a named preset.
    ///
    /// # Errors
    ///
    /// Returns an error for blank names or incompatible rule switches.
    pub fn new(name: impl Into<String>, rules: Rules) -> Result<Self, RulePresetError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(RulePresetError::BlankName);
        }
        let rules = rules.validate()?;
        Ok(Self {
            schema_version: RULE_PRESET_SCHEMA_VERSION,
            name: name.trim().to_owned(),
            rules,
        })
    }

    /// Validates an imported preset.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported schema or invalid contents.
    pub fn validated(self) -> Result<Self, RulePresetError> {
        if self.schema_version != RULE_PRESET_SCHEMA_VERSION {
            return Err(RulePresetError::UnsupportedSchema(self.schema_version));
        }
        Self::new(self.name, self.rules)
    }
}

/// Custom-rule validation errors.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum RulePresetError {
    /// Preset names must contain visible text.
    #[error("custom rule preset name cannot be blank")]
    BlankName,
    /// Imported schema is unsupported.
    #[error("rule preset schema {0} is not supported")]
    UnsupportedSchema(u16),
    /// Domain rule combination is incompatible.
    #[error(transparent)]
    Domain(#[from] DomainError),
}

/// Storage boundary for named custom rules.
pub trait RulePresetRepository: Send + Sync {
    /// Loads all named presets.
    ///
    /// # Errors
    ///
    /// Returns an adapter-specific persistence message.
    fn load_rule_presets(&self) -> Result<Vec<NamedRulePreset>, String>;

    /// Replaces all named presets.
    ///
    /// # Errors
    ///
    /// Returns an adapter-specific persistence message.
    fn save_rule_presets(&self, presets: &[NamedRulePreset]) -> Result<(), String>;

    /// Exports a single portable preset.
    ///
    /// # Errors
    ///
    /// Returns an adapter-specific persistence message.
    fn export_rule_preset(&self, preset: &NamedRulePreset) -> Result<(), String>;

    /// Imports a single portable preset.
    ///
    /// # Errors
    ///
    /// Returns an adapter-specific persistence or validation message.
    fn import_rule_preset(&self) -> Result<Option<NamedRulePreset>, String>;
}

#[cfg(test)]
mod tests {
    use ludo_domain::RulePreset;

    use super::*;

    #[test]
    fn incompatible_switches_are_rejected() {
        let mut rules = RulePreset::Classic.rules();
        rules.extra_turn_on_six = false;
        assert!(matches!(
            NamedRulePreset::new("Invalid", rules),
            Err(RulePresetError::Domain(DomainError::InvalidRules))
        ));
    }

    #[test]
    fn names_are_trimmed_and_blank_names_are_rejected() {
        let preset = NamedRulePreset::new("  Fast table  ", RulePreset::Quick.rules())
            .unwrap_or_else(|_| std::process::abort());
        assert_eq!(preset.name, "Fast table");
        assert!(matches!(
            NamedRulePreset::new(" \n\t ", RulePreset::Classic.rules()),
            Err(RulePresetError::BlankName)
        ));
    }

    #[test]
    fn imported_schema_must_match_exactly() {
        let preset = NamedRulePreset {
            schema_version: RULE_PRESET_SCHEMA_VERSION.saturating_add(1),
            name: "Future".to_owned(),
            rules: RulePreset::Classic.rules(),
        };
        assert!(matches!(
            preset.validated(),
            Err(RulePresetError::UnsupportedSchema(_))
        ));
    }
}
