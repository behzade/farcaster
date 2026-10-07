use super::*;

#[test]
fn unfinished_turn_keeps_the_prompt_without_an_empty_reply() {
    let message = json!({"type":"user","message":{"turn":{"case":"agentConversationTurn","value":{
        "userMessage":{"text":"pending prompt"},"steps":[]
    }}}});
    assert_eq!(
        expand_message(&message, "agent:1").unwrap(),
        vec![
            json!({"id":"agent:1:user","role":"user","content":[{"type":"text","text":"pending prompt"}]})
        ]
    );
}

#[test]
fn flat_message_content_is_preserved() {
    let content = json!([{"type":"text","text":"Earlier reply"}]);
    for payload in [
        json!({"content":content}),
        json!({"message":{"content":content}}),
    ] {
        assert_eq!(
            expand_message(&json!({"type":"assistant","message":payload}), "a1").unwrap(),
            vec![json!({"id":"a1","role":"assistant","content":content})]
        );
    }
}

#[test]
#[ignore = "reads an existing local Cursor SDK session selected by FARCASTER_CURSOR_HISTORY_AGENT_ID"]
fn live_cursor_sdk_history() -> Result<(), String> {
    let id =
        std::env::var("FARCASTER_CURSOR_HISTORY_AGENT_ID").map_err(|error| error.to_string())?;
    let project =
        std::env::var("FARCASTER_CURSOR_HISTORY_PROJECT").map_err(|error| error.to_string())?;
    let project = Path::new(&project);
    let config = crate::AgentLaunchConfig {
        program: super::super::program(),
        ..Default::default()
    };
    let bridge = Bridge::start(&config, project)?;
    let history = load(&bridge, &id, project)?;
    let live = Bridge::start_live(&config, project, false)?;
    assert_eq!(load(&live, &id, project)?.messages, history.messages);
    assert!(history.messages.iter().any(|message| {
        message["role"] == "user"
            && message["content"]
                .as_array()
                .is_some_and(|content| !content.is_empty())
    }));
    assert!(
        history
            .messages
            .iter()
            .any(|message| message["role"] == "assistant"
                && message["content"]
                    .as_array()
                    .is_some_and(|content| content.iter().any(|part| part["type"] == "text"
                        && part["text"].as_str().is_some_and(|text| !text.is_empty()))))
    );
    eprintln!(
        "Loaded {} transcript messages from the saved Cursor session",
        history.messages.len()
    );
    Ok(())
}

#[test]
fn persisted_tools_keep_order_identity_results_and_later_text() {
    let message = json!({"type":"user","message":{"turn":{"case":"agentConversationTurn","value":{
        "userMessage":{"text":"run checks"},"steps":[
            {"message":{"case":"thinkingMessage","value":{"text":"check first"}}},
            {"message":{"case":"assistantMessage","value":{"text":"Running checks"}}},
            {"message":{"case":"toolCall","value":{"toolCallId":"shell-1","tool":{"case":"shellToolCall","value":{
                "args":{"command":"cargo test"},"result":{"result":{"case":"success","value":{"stdout":"ok","stderr":"","exitCode":0}}}
            }}}}},
            {"message":{"case":"assistantMessage","value":{"text":"Checks passed"}}},
            {"message":{"case":"toolCall","value":{"toolCallId":"read-1","tool":{"case":"readToolCall","value":{
                "args":{"path":"missing"},"result":{"result":{"case":"error","value":{"message":"not found"}}}
            }}}}},
            {"message":{"case":"assistantMessage","value":{"text":"That file is missing"}}}
        ]
    }}}});
    let rows = expand_message(&message, "turn-1").unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| r["role"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "user",
            "assistant",
            "toolResult",
            "assistant",
            "toolResult",
            "assistant"
        ]
    );
    assert_eq!(rows[1]["content"][0]["type"], "thinking");
    assert_eq!(rows[1]["content"][2]["id"], "shell-1");
    assert_eq!(rows[2]["toolCallId"], "shell-1");
    assert_eq!(rows[2]["content"][0]["text"], "ok");
    assert_eq!(rows[2]["isError"], false);
    assert_eq!(rows[3]["content"][0]["text"], "Checks passed");
    assert_eq!(rows[4]["toolCallId"], "read-1");
    assert_eq!(rows[4]["isError"], true);
    assert_eq!(rows[5]["content"][0]["text"], "That file is missing");
    let ids: HashSet<_> = rows.iter().map(|row| row["id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), rows.len());
    assert_eq!(expand_message(&message, "turn-1").unwrap(), rows);
}

#[test]
fn stored_shell_turn_and_unfinished_tool_are_not_dropped() {
    let shell = json!({"type":"user","message":{"turn":{"case":"shellConversationTurn","value":{
        "shellCommand":{"command":"false"},"shellOutput":{"stdout":"","stderr":"failed","exitCode":1}
    }}}});
    let rows = expand_message(&shell, "shell").unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["content"][0]["arguments"]["command"], "false");
    assert_eq!(rows[1]["isError"], true);
    let unfinished = json!({"type":"user","message":{"turn":{"case":"agentConversationTurn","value":{
        "steps":[{"message":{"case":"toolCall","value":{"tool":{"case":"shellToolCall","value":{"args":{"command":"sleep 10"}}}}}}]
    }}}});
    let rows = expand_message(&unfinished, "pending").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["content"][0]["type"], "toolCall");
}

#[test]
fn js_sdk_canonical_protobuf_json_replays_tools_and_errors() {
    let rows = expand_message(&json!({"type":"user","message":{"agentConversationTurn":{
        "userMessage":{"text":"check"},"steps":[
            {"thinkingMessage":{"text":"inspect"}},
            {"toolCall":{"toolCallId":"run-check","shellToolCall":{
                "args":{"command":"check"},"result":{"success":{"stdout":"out","stderr":"err","exitCode":1}}
            }}},
            {"assistantMessage":{"text":"check failed"}},
            {"toolCall":{"toolCallId":"read","readToolCall":{
                "args":{"path":"missing"},"result":{"error":{"message":"missing file"}}
            }}}
        ]
    }}}), "canonical").unwrap();
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0]["content"][0]["text"], "check");
    assert_eq!(rows[1]["content"][0]["thinking"], "inspect");
    assert_eq!(rows[1]["content"][1]["arguments"]["command"], "check");
    assert_eq!(rows[2]["toolCallId"], "run-check");
    assert_eq!(rows[2]["content"][0]["text"], "outerr");
    assert_eq!(rows[2]["isError"], true);
    assert_eq!(rows[3]["content"][0]["text"], "check failed");
    assert_eq!(rows[4]["isError"], true);
}
