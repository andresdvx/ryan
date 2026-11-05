//! Único camino para ejecutar una herramienta:
//! registro -> validación -> política de permisos -> (confirmación) -> ejecución.
//! La IA solo puede llamar a `submit`; no existe forma de ejecutar saltándose la política.

use std::sync::{Arc, RwLock};

use crate::events::{Event, EventBus};
use crate::permissions::{Decision, PermissionPolicy};
use crate::tools::{RiskLevel, Tool, ToolCall, ToolError, ToolOutput, ToolRegistry, ToolSpec};

/// Acción validada que espera la respuesta del usuario. Solo el ejecutor puede crearla.
pub struct PendingAction {
    call: ToolCall,
    preview: String,
    risk: RiskLevel,
    detailed: bool,
}

impl PendingAction {
    pub fn tool(&self) -> &str {
        &self.call.name
    }

    pub fn preview(&self) -> &str {
        &self.preview
    }

    pub fn risk(&self) -> RiskLevel {
        self.risk
    }

    /// `true` si la interfaz debe mostrar una confirmación explícita y detallada.
    pub fn detailed(&self) -> bool {
        self.detailed
    }
}

pub enum Submission {
    Completed(ToolOutput),
    NeedsConfirmation(PendingAction),
    Rejected(ToolError),
}

pub struct ToolExecutor {
    registry: ToolRegistry,
    policy: RwLock<PermissionPolicy>,
    bus: Arc<EventBus>,
}

impl ToolExecutor {
    pub fn new(registry: ToolRegistry, policy: PermissionPolicy, bus: Arc<EventBus>) -> Self {
        Self { registry, policy: RwLock::new(policy), bus }
    }

    /// Herramientas que se describen al modelo de IA: las que el usuario no desactivó.
    pub fn specs(&self) -> Vec<ToolSpec> {
        let policy = self.policy.read().expect("política envenenada");
        self.registry.specs().into_iter().filter(|spec| !policy.is_disabled(&spec.name)).collect()
    }

    /// Todas las herramientas registradas, también las desactivadas (para la pantalla de ajustes).
    pub fn all_specs(&self) -> Vec<ToolSpec> {
        self.registry.specs()
    }

    /// Cada herramienta con su nivel de riesgo habitual (el que tiene sin argumentos), para enseñarlo en Ajustes.
    pub fn all_with_risk(&self) -> Vec<(ToolSpec, RiskLevel)> {
        let empty = serde_json::Value::Object(Default::default());
        self.registry
            .specs()
            .into_iter()
            .map(|spec| {
                let risk = self.registry.get(&spec.name).map(|tool| tool.risk(&empty)).unwrap_or(RiskLevel::Confirm);
                (spec, risk)
            })
            .collect()
    }

    /// Aplica nuevos ajustes de permisos sin reiniciar; afecta a las siguientes peticiones.
    pub fn set_policy(&self, policy: PermissionPolicy) {
        *self.policy.write().expect("política envenenada") = policy;
    }

    fn decide(&self, tool: &str, risk: RiskLevel) -> Decision {
        self.policy.read().expect("política envenenada").decide(tool, risk)
    }

    pub fn submit(&self, call: ToolCall) -> Submission {
        self.bus.publish(Event::ToolRequested { tool: call.name.clone() });

        let Some(tool) = self.registry.get(&call.name) else {
            return self.reject(&call.name, ToolError::UnknownTool(call.name.clone()));
        };
        if let Err(error) = tool.validate(&call.args) {
            return self.reject(&call.name, error);
        }

        let risk = tool.risk(&call.args);
        match self.decide(&call.name, risk) {
            Decision::Deny(reason) => self.reject(&call.name, ToolError::NotAllowed(reason)),
            Decision::Allow => match self.run(tool, &call) {
                Ok(output) => Submission::Completed(output),
                Err(error) => Submission::Rejected(error),
            },
            Decision::Confirm { detailed } => {
                self.bus.publish(Event::ToolConfirmationRequired { tool: call.name.clone() });
                let preview = tool.preview(&call.args);
                Submission::NeedsConfirmation(PendingAction { call, preview, risk, detailed })
            }
        }
    }

    /// El usuario aceptó. Se vuelve a validar porque el sistema pudo cambiar mientras esperaba.
    pub fn confirm(&self, pending: PendingAction) -> Result<ToolOutput, ToolError> {
        let call = &pending.call;
        let Some(tool) = self.registry.get(&call.name) else {
            return Err(self.fail(&call.name, ToolError::UnknownTool(call.name.clone())));
        };
        tool.validate(&call.args).map_err(|error| self.fail(&call.name, error))?;
        if let Decision::Deny(reason) = self.decide(&call.name, tool.risk(&call.args)) {
            return Err(self.fail(&call.name, ToolError::NotAllowed(reason)));
        }
        self.run(tool, call)
    }

    /// Imágenes que dejó la última ejecución de esa herramienta (se entregan una sola vez).
    pub fn take_images(&self, tool: &str) -> Vec<crate::ai::Image> {
        self.registry.get(tool).map(|tool| tool.take_images()).unwrap_or_default()
    }

    pub fn cancel(&self, pending: PendingAction) {
        self.bus.publish(Event::ToolCancelled { tool: pending.call.name });
    }

    fn run(&self, tool: &dyn Tool, call: &ToolCall) -> Result<ToolOutput, ToolError> {
        self.bus.publish(Event::ToolStarted { tool: call.name.clone() });
        match tool.execute(&call.args) {
            Ok(output) => {
                self.bus.publish(Event::ToolCompleted { tool: call.name.clone() });
                Ok(output)
            }
            Err(error) => Err(self.fail(&call.name, error)),
        }
    }

    fn fail(&self, tool: &str, error: ToolError) -> ToolError {
        self.bus.publish(Event::ToolFailed { tool: tool.to_string(), error: error.to_string() });
        error
    }

    fn reject(&self, tool: &str, error: ToolError) -> Submission {
        Submission::Rejected(self.fail(tool, error))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::{json, Value};

    use super::*;

    struct FakeTool {
        risk: RiskLevel,
        runs: Arc<AtomicUsize>,
    }

    impl Tool for FakeTool {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn description(&self) -> &'static str {
            "herramienta de prueba"
        }
        fn parameters(&self) -> Value {
            json!({ "type": "object" })
        }
        fn risk(&self, _args: &Value) -> RiskLevel {
            self.risk
        }
        fn validate(&self, args: &Value) -> Result<(), ToolError> {
            args.get("ok")
                .map(|_| ())
                .ok_or_else(|| ToolError::InvalidArguments("falta «ok»".into()))
        }
        fn preview(&self, _args: &Value) -> String {
            "hace algo".into()
        }
        fn execute(&self, _args: &Value) -> Result<ToolOutput, ToolError> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            Ok(ToolOutput { summary: "hecho".into(), data: Value::Null })
        }
    }

    #[test]
    fn la_pantalla_de_ajustes_ve_el_riesgo_de_cada_herramienta_aunque_este_desactivada() {
        for risk in [RiskLevel::Safe, RiskLevel::Confirm, RiskLevel::Dangerous] {
            let mut policy = PermissionPolicy::default();
            policy.disable_tool("fake");
            let (executor, ..) = setup(risk, policy);
            let all = executor.all_with_risk();
            assert_eq!(all.len(), 1);
            assert_eq!((all[0].0.name.as_str(), all[0].1), ("fake", risk));
            assert!(executor.specs().is_empty(), "desactivada: la IA no la ve, pero Ajustes sí");
        }
    }

    fn setup(risk: RiskLevel, policy: PermissionPolicy) -> (ToolExecutor, Arc<AtomicUsize>, Arc<EventBus>) {
        let runs = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry.register(FakeTool { risk, runs: runs.clone() });
        let bus = Arc::new(EventBus::new());
        (ToolExecutor::new(registry, policy, bus.clone()), runs, bus)
    }

    fn call(args: Value) -> ToolCall {
        ToolCall { name: "fake".into(), args }
    }

    #[test]
    fn lo_seguro_se_ejecuta_directamente_y_publica_los_eventos() {
        let (executor, runs, bus) = setup(RiskLevel::Safe, PermissionPolicy::default());
        let events = bus.subscribe();
        assert!(matches!(executor.submit(call(json!({ "ok": 1 }))), Submission::Completed(_)));
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        let seen: Vec<Event> = events.try_iter().collect();
        assert_eq!(
            seen,
            vec![
                Event::ToolRequested { tool: "fake".into() },
                Event::ToolStarted { tool: "fake".into() },
                Event::ToolCompleted { tool: "fake".into() },
            ]
        );
    }

    #[test]
    fn lo_que_requiere_confirmacion_no_se_ejecuta_hasta_confirmar() {
        let (executor, runs, _) = setup(RiskLevel::Confirm, PermissionPolicy::default());
        let Submission::NeedsConfirmation(pending) = executor.submit(call(json!({ "ok": 1 }))) else {
            panic!("debía pedir confirmación");
        };
        assert_eq!(runs.load(Ordering::SeqCst), 0);
        assert_eq!(pending.preview(), "hace algo");
        assert!(!pending.detailed());
        assert!(executor.confirm(pending).is_ok());
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn lo_peligroso_pide_confirmacion_detallada_aunque_se_relaje_la_politica() {
        let mut policy = PermissionPolicy::default();
        policy.require_confirmation_for_safe(false);
        let (executor, runs, _) = setup(RiskLevel::Dangerous, policy);
        let Submission::NeedsConfirmation(pending) = executor.submit(call(json!({ "ok": 1 }))) else {
            panic!("debía pedir confirmación");
        };
        assert!(pending.detailed());
        assert_eq!(runs.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn cancelar_no_ejecuta_y_lo_publica() {
        let (executor, runs, bus) = setup(RiskLevel::Confirm, PermissionPolicy::default());
        let events = bus.subscribe();
        let Submission::NeedsConfirmation(pending) = executor.submit(call(json!({ "ok": 1 }))) else {
            panic!("debía pedir confirmación");
        };
        executor.cancel(pending);
        assert_eq!(runs.load(Ordering::SeqCst), 0);
        assert_eq!(events.try_iter().last(), Some(Event::ToolCancelled { tool: "fake".into() }));
    }

    #[test]
    fn argumentos_invalidos_y_herramientas_desconocidas_se_rechazan_sin_ejecutar() {
        let (executor, runs, _) = setup(RiskLevel::Safe, PermissionPolicy::default());
        assert!(matches!(
            executor.submit(call(json!({}))),
            Submission::Rejected(ToolError::InvalidArguments(_))
        ));
        let unknown = ToolCall { name: "no_existe".into(), args: json!({}) };
        assert!(matches!(executor.submit(unknown), Submission::Rejected(ToolError::UnknownTool(_))));
        assert_eq!(runs.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn los_cambios_de_politica_se_aplican_en_caliente() {
        let (executor, runs, _) = setup(RiskLevel::Safe, PermissionPolicy::default());
        assert_eq!(executor.specs().len(), 1);

        let mut strict = PermissionPolicy::default();
        strict.require_confirmation_for_safe(true);
        executor.set_policy(strict);
        assert!(matches!(executor.submit(call(json!({ "ok": 1 }))), Submission::NeedsConfirmation(_)));

        let mut off = PermissionPolicy::default();
        off.disable_tool("fake");
        executor.set_policy(off);
        assert!(executor.specs().is_empty(), "una herramienta desactivada no se le describe a la IA");
        assert_eq!(executor.all_specs().len(), 1, "pero sigue disponible para la pantalla de ajustes");
        assert!(matches!(executor.submit(call(json!({ "ok": 1 }))), Submission::Rejected(ToolError::NotAllowed(_))));
        assert_eq!(runs.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn una_herramienta_desactivada_no_se_ejecuta() {
        let mut policy = PermissionPolicy::default();
        policy.disable_tool("fake");
        let (executor, runs, _) = setup(RiskLevel::Safe, policy);
        assert!(matches!(
            executor.submit(call(json!({ "ok": 1 }))),
            Submission::Rejected(ToolError::NotAllowed(_))
        ));
        assert_eq!(runs.load(Ordering::SeqCst), 0);
    }
}