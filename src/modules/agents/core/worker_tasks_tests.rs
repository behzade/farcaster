use super::*;

#[test]
fn routing_is_independent_of_parent_and_task_kind() {
    let tasks = WorkerTasks::default();
    for task in ["read", "implement", "review"] {
        assert_eq!(
            tasks
                .resolve(task, WorkerJudgment::Specified)
                .unwrap()
                .execution
                .model,
            "gpt-5.6-luna"
        );
        assert_eq!(
            tasks
                .resolve(task, WorkerJudgment::Guided)
                .unwrap()
                .execution
                .effort
                .as_deref(),
            Some("medium")
        );
        assert_eq!(
            tasks
                .resolve(task, WorkerJudgment::Independent)
                .unwrap()
                .execution
                .model,
            "gpt-6-astra"
        );
        assert_eq!(
            tasks
                .resolve(task, WorkerJudgment::Independent)
                .unwrap()
                .execution
                .effort
                .as_deref(),
            Some("medium")
        );
    }
}

#[test]
fn empty_is_deliberate_and_invalid_definitions_fail_closed() {
    let mut tasks = WorkerTasks { tasks: Vec::new() };
    assert!(tasks.validate().is_ok());
    assert!(tasks.resolve("implement", WorkerJudgment::Guided).is_err());
    tasks = WorkerTasks::default();
    tasks.tasks[1].name = "READ".into();
    assert!(tasks.validate().is_err());
    tasks = WorkerTasks::default();
    tasks.tasks[0].specified.model.clear();
    assert!(tasks.validate().is_err());
}
