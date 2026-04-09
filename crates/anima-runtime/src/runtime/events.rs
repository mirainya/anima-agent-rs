use crate::tasks::model::{
    RequirementRecord, RunRecord, SuspensionRecord, TaskRecord, ToolInvocationRuntimeRecord,
    TurnRecord,
};
use crate::transcript::model::MessageRecord;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeDomainEvent {
    RunUpserted {
        run: RunRecord,
    },
    TurnUpserted {
        turn: TurnRecord,
    },
    TaskUpserted {
        task: TaskRecord,
    },
    SuspensionUpserted {
        suspension: SuspensionRecord,
    },
    ToolInvocationUpserted {
        invocation: ToolInvocationRuntimeRecord,
    },
    RequirementUpserted {
        requirement: RequirementRecord,
    },
    MessageAppended {
        message: MessageRecord,
    },
    ProjectionHintRecorded {
        run_id: String,
        scope: String,
        key: String,
        value: Value,
    },
}
