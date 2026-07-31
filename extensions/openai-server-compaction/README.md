# OpenAI server compaction

This repo owns this Pi extension. It uses OpenAI's native Responses compaction output for direct OpenAI and OpenAI Codex models.

For Codex, a host with the Pi AI output-item hook runs compaction through Pi AI's cached WebSocket stream. That lets the provider send `previous_response_id` plus only a `compaction_trigger` when the active request chain matches. Older hosts use the full-input HTTP path.

The extension saves the native compaction item and its usage in the Pi compaction entry. Hosts can add that saved usage to session totals.

See [NOTICE.md](./NOTICE.md) for the source history.
