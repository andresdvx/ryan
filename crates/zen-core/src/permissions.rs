use std::collections::HashSet;

use crate::tools::RiskLevel;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Confirm { detailed: bool },
    Deny(String),
}

/// Preferencias del usuario. Pueden endurecer las reglas (desactivar herramientas, confirmar todo),
/// nunca relajar las de las acciones peligrosas.
#[derive(Debug, Default)]
pub struct PermissionPolicy {
    disabled_tools: HashSet<String>,
    confirm_safe_actions: bool,
}

impl PermissionPolicy {
    pub fn disable_tool(&mut self, name: &str) {
        self.disabled_tools.insert(name.to_string());
    }

    pub fn require_confirmation_for_safe(&mut self, value: bool) {
        self.confirm_safe_actions = value;
    }

    pub fn is_disabled(&self, tool: &str) -> bool {
        self.disabled_tools.contains(tool)
    }

    pub fn decide(&self, tool: &str, risk: RiskLevel) -> Decision {
        if self.disabled_tools.contains(tool) {
            return Decision::Deny(format!("la herramienta «{tool}» está desactivada en los ajustes"));
        }
        match risk {
            RiskLevel::Safe if self.confirm_safe_actions => Decision::Confirm { detailed: false },
            RiskLevel::Safe => Decision::Allow,
            RiskLevel::Confirm => Decision::Confirm { detailed: false },
            RiskLevel::Dangerous => Decision::Confirm { detailed: true },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn por_defecto_solo_lo_seguro_se_ejecuta_sin_preguntar() {
        let policy = PermissionPolicy::default();
        assert_eq!(policy.decide("t", RiskLevel::Safe), Decision::Allow);
        assert_eq!(policy.decide("t", RiskLevel::Confirm), Decision::Confirm { detailed: false });
        assert_eq!(policy.decide("t", RiskLevel::Dangerous), Decision::Confirm { detailed: true });
    }

    #[test]
    fn el_usuario_puede_endurecer_pero_no_relajar() {
        let mut policy = PermissionPolicy::default();
        policy.require_confirmation_for_safe(true);
        assert_eq!(policy.decide("t", RiskLevel::Safe), Decision::Confirm { detailed: false });
        policy.require_confirmation_for_safe(false);
        assert_eq!(policy.decide("t", RiskLevel::Dangerous), Decision::Confirm { detailed: true });
    }

    #[test]
    fn una_herramienta_desactivada_se_deniega() {
        let mut policy = PermissionPolicy::default();
        policy.disable_tool("t");
        assert!(matches!(policy.decide("t", RiskLevel::Safe), Decision::Deny(_)));
        assert_eq!(policy.decide("otra", RiskLevel::Safe), Decision::Allow);
    }
}