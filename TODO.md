# TODO

- Make system prompt attribution/provider branding configurable based on the active provider (avoid Claude-specific phrasing when provider != anthropic).
- Consider propagating provider/model metadata into prompts so sub-agents don't misidentify the model.
- Identify ways to unify the provider/model/api/base_url setup logic and make it more straightforward, readable and easy to extend. Recurring logic should be well decomposed, non-repeating, but also allow for preparing hardcoded provider classes that match the existing, non-varying provider setups. These hardcoded providers shall serve as default setups, but overriding things such as model string/ID, provider base URL, api key, etc. should be implemented in a generic way, which allows the code to reuse this for every provider and the user should be able to edit these things dynamically from the CLI.
- Identify all the variables which are named with "anthropic", "openai" or the name of any other specific provider AND SIMULTANEOUSLY do not carry information exclusive to that provider, such are variable named "anthropic_key", "anthropic_base" which carry the generic key or base url for any provider. Rename these variables to a more appropriate name. If the variable can hold any API key for any provider based on the config, rename it to just "provider_api_key". If the variable can hold any provider base URL, rename it to just "provider_base_url", if there is anything else, that carries the anthropic name, but it makes sense to reuse it for any provider, change the name and update the code for maximum reuse of logic (as described in the previous bulletpoint)

- **Refactor TUI transcript rendering to interleave tool calls/results with assistant messages** (currently tool blocks are grouped at the bottom of each turn, breaking the natural LLM flow).  
  **Key structural facts:**
  - `Message.content` already contains `ContentBlock::ToolUse` and `ContentBlock::ToolResult` blocks in their natural sequence within the message.
  - `TranscriptTurn` stores both `assistant_messages` (full messages with inline tool blocks) AND a separate `tool_blocks: Vec<&ToolUseBlock>` list for expanded UI.
  - `app.tool_use_blocks` is a global list indexed by `turn_index`, not by sequence position within a turn.
  - Current rendering order: all assistant messages → all grouped tool blocks (causes visual separation).
  - Desired rendering order: text → tool call → tool result → text → tool call → ... (interleaved as they occurred).
  - Fix requires either: (a) storing sequence index in `ToolUseBlock` to interleave during render, or (b) relying solely on inline `ContentBlock::ToolUse/Result` in messages and removing grouped `tool_blocks` from transcript view (or showing them as collapsible inline elements).