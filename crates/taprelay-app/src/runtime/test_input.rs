use super::*;
use taprelay_core::input_router::PhysicalInput;

pub(super) struct FakeInput(pub Option<&'static str>);

impl InputSource for FakeInput {
    fn is_finished(&self) -> bool {
        self.0.is_some()
    }

    fn failure(&self) -> Option<String> {
        self.0.map(str::to_owned)
    }

    fn configure(
        &self,
        _: &FunctionConfigs,
        _: &[taprelay_core::mapping::CustomMapping],
        _: &taprelay_core::foreground_app::ForegroundAppRules,
        _: bool,
        _: bool,
        _: u64,
    ) -> RouteResult {
        RouteResult::default()
    }

    fn terminate(&self, _: RouterReason) -> RouteResult {
        RouteResult::default()
    }

    fn replay(&self, _: PhysicalInput) -> Result<()> {
        Ok(())
    }
}
