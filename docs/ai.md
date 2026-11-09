# AI analysis and MCP

## AI-assisted contextual risk

*Settings > AI*: any OpenAI-compatible chat API (Ollama, vLLM, LocalAI, OpenAI…): base URL (Ollama:
`http://host:11434/v1`), model, optional key (stored encrypted). **Test provider** makes a real round trip.

A policy with **AI mode** asks the model for one thing only: the **contextual risk** of a patch batch (how dangerous
it is to leave this machine unpatched), as strict JSON with its reasons. Everything else stays deterministic: the
stability risk, the decision table, waivers, windows, approvals, soak and rollback.

Guardrails:

- the answer must be valid JSON with a known level, or it is ignored;
- the model may **raise** the risk freely but **lower it by one level at most** below the deterministic rules (a small
  model that hallucinates "not exposed" cannot leave an exposed machine unpatched);
- any error, timeout or bad answer falls back to the deterministic engine, and the rationale says so;
- only host attributes (type, environment, criticality, exposure, tags), package names/versions and CVE metadata are
  sent; never hostnames' secrets, keys or logs.

On a task, **Ask the AI** gives a second opinion (advisory; it changes neither the decision nor the task). It is
stored on the task and audited.

### Test setup in the homelab

LXC 107 `ollama` on the Proxmox host (Debian 12, 8 cores, 12 GB, VLAN 10, `10.0.10.40:11434`), model
`croustypatch-risk` = `qwen2.5:3b` with `num_thread 4` and `temperature 0` (the host CPU is shared with other VMs; with
8 threads llama.cpp slowed to 0.2 token/s, with 4 it runs at about 8 token/s, ~30 s per assessment). A 3B model
answers in the right format but its reasoning is weak (it contradicted its input in testing): use a 7B+ model, ideally
on a GPU, for real decisions. Ollama has no authentication: keep it on the management VLAN.

## MCP server

`POST /api/v1/mcp`, JSON-RPC 2.0 ([Model Context Protocol](https://modelcontextprotocol.io), protocol `2025-06-18`,
streamable HTTP answered with plain JSON). Authenticate with a **personal API token** (*My account > API tokens*,
`Authorization: Bearer cpat_…`). The token acts as its user: the same role, the same permissions, the same audit
trail. Tokens are stored hashed, shown once, expire (1-365 days) and can be revoked.

Tools: `fleet_summary`, `list_hosts`, `get_host`, `list_tasks`, `get_task` (read-only) and `request_patch`, which
always creates a task **awaiting human approval** (`requested_by = mcp:<user>`); an assistant never installs anything
by itself.

Client configuration (Claude Desktop / Claude Code style):

```json
{ "mcpServers": { "croustypatch": { "type": "http", "url": "https://patch.example.org/api/v1/mcp",
  "headers": { "Authorization": "Bearer cpat_…" } } } }
```
