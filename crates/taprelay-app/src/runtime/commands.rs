#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureTarget {
    pub id: taprelay_core::function::FunctionId,
    pub slot: usize,
}

#[derive(Clone, PartialEq, Eq)]
pub enum BindingCommand {
    BeginCustomCapture {
        output: bool,
        slot: usize,
        draft: taprelay_core::mapping::CustomMapping,
    },
    BeginCapture(CaptureTarget),
    CancelCapture,
    DeleteShortcut(CaptureTarget),
    SetFunctionEnabled {
        id: taprelay_core::function::FunctionId,
        enabled: bool,
    },
}
