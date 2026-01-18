use rustpython_vm::vm::ContinuationMode;

#[derive(Clone, Debug)]
pub struct ContinuationOptions {
    pub mode: ContinuationMode,
    pub resume_bytes: Option<Vec<u8>>,
    pub resume_key: Option<Vec<u8>>,
    pub checkpoint_key: Option<Vec<u8>>,
}

impl Default for ContinuationOptions {
    fn default() -> Self {
        Self {
            mode: ContinuationMode::Fsm,
            resume_bytes: None,
            resume_key: None,
            checkpoint_key: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RuntimeConfig {
    pub continuation_mode: ContinuationMode,
}

impl RuntimeConfig {
    pub fn from_options(options: Option<&ContinuationOptions>) -> Self {
        let continuation_mode = options.map(|o| o.mode).unwrap_or(ContinuationMode::Fsm);
        Self { continuation_mode }
    }
}
