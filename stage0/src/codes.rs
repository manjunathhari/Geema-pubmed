//! Fixed failure and flag codes. See `.10x/decisions/sde/stage0-writer-test.md`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Code {
    // Setup: re-run, never held against Gemma.
    PromptCut,
    PromptCountOff,
    CountsMissing,
    ContextFull,
    NumPredictLimit,
    TimeConnect,
    TimeFirstToken,
    TimeGap,
    TimeWhole,
    ConnectFailed,
    ProxyCut,
    ServerBusy,
    Access,
    OutOfMemoryOrCrash,
    SchemaRejected,
    InvalidJsonCleanFinish,
    StreamBadLine,
    StreamErrorOther,
    HttpOther,
    DoneReasonOther,
    NcbiFailed,
    NcbiNoAbstracts,
    UnloadFailed,
    // Input limit: stated with the result.
    ThinInput,
    // Against Gemma, after review.
    RunawaySuspected,
    EmptyOutcomes,
    EmptyStudies,
    EmptyNumbers,
}

impl Code {
    /// The code as logs and result files write it.
    pub fn as_str(self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default()
    }
}
