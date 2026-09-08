use super::*;

#[test]
fn peer_prompt_round_trips_structured_origin() {
    let peer = PeerMessage {
        from: "diff-review".into(),
        message: "review complete\nwith details".into(),
    };
    assert_eq!(PeerMessage::from_prompt(&peer.prompt()), Some(peer));
    assert!(PeerMessage::from_prompt("Message from Farcaster worker bad id:\n\nno").is_none());
    assert!(PeerMessage::from_prompt("ordinary user message").is_none());
    assert_eq!(
        PeerMessage::from_prompt("Message from Farcaster peer worker-7:\n\nlegacy")
            .map(|message| message.from),
        Some("worker-7".into())
    );
}
