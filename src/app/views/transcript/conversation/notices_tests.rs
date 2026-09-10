use super::*;

#[test]
fn adjacent_extension_notices_share_a_transcript_item() {
    let mut conversation = ConversationState::default();
    assert_eq!(
        conversation.push_extension_notice("MCP connected".into()),
        0
    );
    let previous = conversation.clone();
    assert_eq!(conversation.push_extension_notice("Tools ready".into()), 0);
    assert_eq!(conversation.items.len(), 1);
    let item = &conversation.items[0];
    assert_eq!(item.kind, TranscriptKind::Notice);
    assert_eq!(item.complete_text(), "MCP connected\nTools ready");
    assert_eq!(previous.items[0].complete_text(), "MCP connected");
}

#[test]
fn extension_notices_do_not_group_across_messages_or_run_boundaries() {
    let mut conversation = ConversationState::default();
    conversation.push_extension_notice("Connected".into());
    conversation.push_local_user("Hello".into(), 0, false);
    assert_eq!(conversation.push_extension_notice("Ready".into()), 2);
    conversation.begin_run();
    assert_eq!(conversation.push_extension_notice("Running".into()), 3);
    assert_eq!(conversation.items[2].complete_text(), "Ready");
}
