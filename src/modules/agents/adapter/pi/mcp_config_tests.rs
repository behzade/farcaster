use super::*;

#[test]
fn exposes_farcaster_through_a_reopenable_transient_file() -> Result<(), Box<dyn std::error::Error>>
{
    let config = TransientMcpConfig::create("caller-1")?;
    let path = config.path().to_owned();
    for _ in 0..2 {
        let value = serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path)?)?;
        assert_eq!(value["mcpServers"]["farcaster"]["url"], URL);
        assert_eq!(value["mcpServers"]["farcaster"]["lifecycle"], "keep-alive");
        assert_eq!(value["mcpServers"]["farcaster"]["directTools"], true);
        assert_eq!(
            value["mcpServers"]["farcaster"]["headers"][CALLER_HEADER],
            "caller-1"
        );
        assert_eq!(
            value["mcpServers"]["farcaster"]["protocolVersion"],
            "2026-07-28"
        );
    }
    drop(config);
    assert!(!path.exists());
    Ok(())
}
