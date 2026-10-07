use super::*;
use crate::{AgentLaunchConfig, WorkerActivity, WorkerEvent, WorkerSendMode, WorkerSession};

#[test]
fn temporary_helper_disables_tools_and_cleans_store_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("package.json"), r#"{"type":"module"}"#).unwrap();
    std::fs::write(
        dir.path().join("sqlite.js"),
        r#"
import { writeFile } from 'node:fs/promises';
export const SqliteLocalAgentStore = { async open({stateRoot}) {
  if (!stateRoot) throw Error('temporary store missing');
  await writeFile(stateRoot + '/index.db', 'fixture');
  return { async dispose() {} };
} };
"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("sdk.js"), r#"
export const Cursor = { configure() {}, models: { async list() { return [{id:'default'}]; } } };
export const Agent = {
  async create(options) {
    if (JSON.stringify(options.tools) !== '[]' || JSON.stringify(options.local.settingSources) !== '[]'
        || Object.keys(options.mcpServers).length) throw Error('temporary agent has tools or settings');
    return { agentId:'title-agent', close() {}, async send() { throw Error('inference failed'); } };
  },
  async get() { throw Error('temporary agent should not poll metadata'); },
};
"#).unwrap();
    let store = tempfile::tempdir().unwrap();
    let store_path = store.path().to_owned();
    let mut command = std::process::Command::new("node");
    command
        .args([
            "--input-type=module",
            "-e",
            &super::super::timing::script(include_str!("runtime.mjs")),
        ])
        .arg(dir.path().join("sdk.js"))
        .arg("--workspace")
        .arg(dir.path())
        .arg("--temporary-store")
        .arg(&store_path);
    let mut bridge = Bridge::from_command(command, None).unwrap();
    bridge.temporary_store = Some(store);
    let (mut worker, _) = super::super::worker::Worker::from_bridge(
        bridge,
        &AgentLaunchConfig::default(),
        dir.path(),
        None,
        Some("unused-caller"),
        None,
    )
    .unwrap();
    assert!(store_path.join("index.db").exists());
    worker
        .send("Generate a title".into(), WorkerSendMode::Prompt)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline, "temporary worker did not fail");
        if let Some(WorkerEvent::Failed(error)) = worker.poll() {
            assert!(error.contains("inference failed"), "{error}");
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    worker.close().unwrap();
    assert!(!store_path.exists(), "temporary SDK store leaked");
}

#[test]
fn sdk_helper_owns_live_runs_and_exposes_correlated_steering() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("package.json"), r#"{"type":"module"}"#).unwrap();
    std::fs::write(
        dir.path().join("sqlite.js"),
        "export const SqliteLocalAgentStore = { async open() { return { async dispose() {} }; } };",
    )
    .unwrap();
    std::fs::write(dir.path().join("sdk.js"), r#"
let created = false;
export const Cursor = { configure() {}, models: { async list() { return [{id:'default'}]; } } };
export const Agent = {
  async create(options) {
    if (created) throw Error('agent created twice');
    created = true;
    if (options.local.sandboxOptions.enabled !== true || options.apiKey !== 'fixture-key') throw Error('create options lost');
    return makeAgent();
  },
  async resume() { throw Error('live agent must not be resumed again'); },
  async get() {
    // Metadata must never delay a turn or its queued follow-up.
    await new Promise(resolve => setTimeout(resolve, 1500));
    return {name:'fixture', lastModified:Date.now()};
  },
};
function makeAgent() {
  return { agentId:'sdk-agent', close() {}, async send(message, options) {
    if (options.mode !== 'agent') throw Error('send options lost');
    let finish;
    const done = new Promise(resolve => { finish = resolve; });
    const inputs = [];
    const tool = {type:'shell',args:{command:'printf progress'}};
    await options.onDelta({update:{type:'tool-call-started', callId:'shell-1', toolCall:tool}});
    await options.onDelta({update:{type:'partial-tool-call', callId:'shell-1', toolCall:tool}});
    await options.onDelta({update:{type:'shell-output-delta',event:{case:'stdout',value:{data:'progress'}}}});
    if (message.text === 'queued') finish();
    return {
      id: 'run-'+message.text,
      async *stream() {
        await options.onDelta({update:{type:'text-delta',text:'started'}});
        yield { type:'assistant', message:{ content:[{type:'text',text:'started'}] } };
        await done;
        await options.onDelta({update:{type:'tool-call-completed',callId:'shell-1',toolCall:{...tool,result:{status:'success',value:{stdout:'progress',stderr:'',exitCode:0}}}}});
        await options.onDelta({update:{type:'text-delta',text:inputs.join('|')}});
        yield { type:'assistant', message:{ content:[{type:'text',text:inputs.join('|')}] } };
      },
      async steer(text) {
        if (text === 'fallback') return 'revert_to_followup';
        inputs.push(text);
        if (inputs.length === 2) finish();
        // Let the Send stream finish before the steering receipt.
        await new Promise(resolve => setTimeout(resolve, 30));
        return 'complete_delivered';
      },
      async cancel() { finish(); },
      async wait() { await done; return {status:'finished', result:'started'+inputs.join('|')}; },
    };
  } };
}
"#).unwrap();
    let mut command = std::process::Command::new("node");
    command
        .args([
            "--input-type=module",
            "-e",
            &super::super::timing::script(include_str!("runtime.mjs")),
        ])
        .arg(dir.path().join("sdk.js"))
        .arg("--workspace")
        .arg(dir.path());
    let bridge = Bridge::from_command(command, Some("fixture-key".into())).unwrap();
    let config = AgentLaunchConfig {
        access_mode: crate::HarnessAccessMode::Auto,
        ..Default::default()
    };
    let startup = Instant::now();
    let (mut worker, _) =
        super::super::worker::Worker::from_bridge(bridge, &config, dir.path(), None, None, None)
            .unwrap();
    assert!(startup.elapsed() < Duration::from_secs(1));
    let mut tool_starts = 0;
    let mut tool_output = String::new();
    worker
        .submit_prompt(
            "prompt".into(),
            "work".into(),
            WorkerSendMode::Prompt,
            vec![],
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut started = false;
    while !started && Instant::now() < deadline {
        match worker.poll() {
            Some(WorkerEvent::Activity(WorkerActivity::TextDelta { .. })) => started = true,
            Some(WorkerEvent::Activity(WorkerActivity::ToolStarted { .. })) => tool_starts += 1,
            Some(WorkerEvent::Activity(WorkerActivity::ToolUpdated { content, .. })) => {
                tool_output.push_str(content[0]["text"].as_str().unwrap())
            }
            Some(WorkerEvent::Failed(error)) => panic!("{error}"),
            _ => {}
        }
        thread::sleep(Duration::from_millis(2));
    }
    assert!(started);
    assert_eq!(tool_starts, 1);
    assert_eq!(tool_output, "progress");
    let finish_started = Instant::now();
    for (id, text) in [
        ("fallback", "fallback"),
        ("first", "same"),
        ("second", "same"),
    ] {
        assert!(
            !worker
                .submit_prompt(id.into(), text.into(), WorkerSendMode::Steer, vec![])
                .unwrap()
        );
    }
    let mut acks = Vec::new();
    let mut delivered = Vec::new();
    let mut settled = false;
    while !settled && Instant::now() < deadline {
        while let Some(ack) = worker.poll_prompt_ack() {
            acks.push(ack);
        }
        while let Some(event) = worker.poll() {
            match event {
                WorkerEvent::Activity(WorkerActivity::InputDelivered {
                    submission_id: Some(id),
                    ..
                }) => delivered.push(id),
                WorkerEvent::Settled { output } => {
                    assert_eq!(output, "startedsame|same");
                    settled = true;
                }
                WorkerEvent::Failed(error) => panic!("{error}"),
                _ => {}
            }
        }
        thread::sleep(Duration::from_millis(2));
    }
    while let Some(ack) = worker.poll_prompt_ack() {
        acks.push(ack);
    }
    assert!(settled);
    assert!(
        finish_started.elapsed() < Duration::from_secs(1),
        "title lookup blocked completion"
    );
    delivered.sort();
    assert_eq!(delivered, ["first", "second"]);
    assert_eq!(acks.len(), 3);
    assert_eq!(acks.iter().filter(|(_, result)| result.is_ok()).count(), 2);
    assert!(
        acks.iter()
            .any(|(id, result)| id == "fallback" && result.is_err())
    );
    // Once the live handle has gone, the same API explicitly returns to the shared queue.
    assert!(
        !worker
            .submit_prompt(
                "after".into(),
                "later".into(),
                WorkerSendMode::Steer,
                vec![]
            )
            .unwrap()
    );
    assert!(worker.poll_prompt_ack().unwrap().1.is_err());
    worker
        .submit_prompt(
            "queued".into(),
            "queued".into(),
            WorkerSendMode::Queue,
            vec![],
        )
        .unwrap();
    let mut follow_up = false;
    while !follow_up && Instant::now() < deadline {
        match worker.poll() {
            Some(WorkerEvent::Settled { output }) => {
                assert_eq!(output, "started");
                follow_up = true;
            }
            Some(WorkerEvent::Failed(error)) => panic!("{error}"),
            _ => {}
        }
        thread::sleep(Duration::from_millis(2));
    }
    assert!(follow_up);
    worker.close().unwrap();
}

#[test]
#[ignore = "uses the installed SDK locally without a model request or account"]
fn installed_sdk_helper_creates_resumes_and_reads_its_own_store() -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|error| error.to_string())?;
    let start = |temporary_store: Option<&Path>| {
        let mut environment = std::process::Command::new("node");
        environment
            .env("HOME", dir.path())
            .env_remove("CURSOR_API_KEY");
        let mut command =
            super::super::auth::sdk_command(&environment, dir.path(), include_str!("runtime.mjs"))?;
        command.arg("--workspace").arg(dir.path());
        if let Some(store) = temporary_store {
            command.arg("--temporary-store").arg(store);
        }
        Bridge::from_command(command, None)
    };
    let options = json!({"model":{"id":"auto"},"local":{"cwd":[dir.path()],"sandboxOptions":{"enabled":true}}});
    let mut first = start(None)?;
    let created = first.agent("CreateAgent", json!({"options":options}))?;
    let id = created["agentId"].as_str().ok_or("missing agent id")?;
    let info = first.agent("GetAgent", json!({"agentId":id}))?;
    assert_eq!(info["agent"]["agentId"], id);
    let runs = first.agent("ListRuns", json!({"agentId":id}))?["items"].clone();
    assert_eq!(runs[0]["model"]["id"], "auto");
    first.agent("RenameAgent", json!({"agentId":id,"name":"Saved title"}))?;
    first.close()?;
    let mut second = start(None)?;
    assert_eq!(
        second.agent("ResumeAgent", json!({"agentId":id,"options":options}))?["agentId"],
        id
    );
    assert_eq!(
        second.agent("ListAgentMessages", json!({"agentId":id}))?["messages"],
        json!([])
    );
    assert_eq!(
        second.agent("GetAgent", json!({"agentId":id}))?["agent"]["name"],
        "Saved title"
    );
    assert_eq!(
        second.agent("ListRuns", json!({"agentId":id}))?["items"],
        runs
    );
    second.close()?;
    let store = tempfile::tempdir().map_err(|error| error.to_string())?;
    let path = store.path().to_owned();
    let mut temporary = start(Some(&path))?;
    temporary.temporary_store = Some(store);
    temporary.agent("CreateAgent", json!({"options":options}))?;
    assert!(path.join("index.db").exists());
    drop(temporary);
    assert!(!path.exists(), "temporary store must be removed on drop");
    Ok(())
}

#[test]
#[ignore = "uses the signed-in Cursor account for one temporary title request"]
fn installed_sdk_generates_title_through_shared_worker() -> Result<(), String> {
    let project = tempfile::tempdir().map_err(|error| error.to_string())?;
    let config = AgentLaunchConfig {
        program: super::super::program(),
        ..Default::default()
    };
    let title = crate::adapter::auxiliary::generate_session_title(
        &config,
        crate::Backend::Cursor,
        project.path(),
        "Fix duplicate queued messages in the chat window",
        None,
    )?;
    assert!(!title.is_empty());
    assert_ne!(title, "New Agent");
    Ok(())
}
