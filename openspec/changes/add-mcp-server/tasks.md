# Tasks: add-mcp-server

## 1. Server
- [ ] 1.1 `estream-api::mcp`: Streamable HTTP endpoint on 127.0.0.1 with bearer token, on the API I/O thread
- [ ] 1.2 Queue tool calls to `Api::tick` and answer through `Studio` / the command bus
- [ ] 1.3 Settings: enable, port, token (regenerate), allow going live and stopping outputs
- [ ] 1.4 `clutch --mcp-stdio` bridge

## 2. Tools
- [ ] 2.1 Read tools: status, setups, canvases, scenes, sources, stats, health
- [ ] 2.2 Scene/source tools: switch scene, visibility, text, volume, mute
- [ ] 2.3 Clip and output tools, with destructive annotations and the allow-outputs gate
- [ ] 2.4 No tool returns stream keys, tokens or passwords (test)

## 3. UI
- [ ] 3.1 Settings → Remote control → AI assistants (MCP), Copy connection
- [ ] 3.2 Status bar indicator while a client is connected; action log lines

## 4. Check
- [ ] 4.1 Tests per tool; connect from an MCP client (HTTP and stdio) and run each tool
