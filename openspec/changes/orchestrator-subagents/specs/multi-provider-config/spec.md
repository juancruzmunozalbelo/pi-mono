## ADDED Requirements

### Requirement: Config TOML Sub-Agent Section
The config TOML SHALL support a `[sub_agent]` section containing the fields `provider`, `model`, `api_key`, and `system_prompt`. All fields in this section SHALL be optional; omitting the entire section means no sub-agent provider is configured.

#### Scenario: Full sub_agent section present
- **WHEN** `config.toml` contains a `[sub_agent]` section with `provider`, `model`, `api_key`, and `system_prompt` values
- **THEN** the application SHALL parse all four fields and use them to construct the sub-agent provider instance

#### Scenario: Partial sub_agent section present
- **WHEN** `config.toml` contains a `[sub_agent]` section with only some fields (e.g. `provider` and `model` but no `api_key`)
- **THEN** the application SHALL parse the present fields and fall back to CLI flags or environment variables for the missing ones

#### Scenario: No sub_agent section
- **WHEN** `config.toml` does not contain a `[sub_agent]` section
- **THEN** the application SHALL treat sub-agent configuration as absent and SHALL NOT register the `spawn_agent` tool

---

### Requirement: CLI Flag for Sub-Agent API Key
The CLI SHALL accept a `--sub-agent-key <key>` flag. When provided, it SHALL override the `api_key` value from the `[sub_agent]` config section.

#### Scenario: Flag provided with config present
- **WHEN** `--sub-agent-key sk-abc` is passed and `config.toml` has `sub_agent.api_key = "sk-old"`
- **THEN** the sub-agent provider SHALL be initialised with `sk-abc`, ignoring `sk-old`

#### Scenario: Flag provided without config section
- **WHEN** `--sub-agent-key sk-abc` is passed and `config.toml` has no `[sub_agent]` section
- **THEN** the application SHALL use the flag value as the sub-agent API key alongside any other sub-agent fields resolvable from environment variables or defaults

---

### Requirement: Environment Variable Fallback for Sub-Agent API Key
The CLI SHALL check the `MINIMAX_API_KEY` environment variable as a fallback source for the sub-agent API key when neither the `--sub-agent-key` flag nor `config.toml`'s `sub_agent.api_key` provides a value.

#### Scenario: Only env var set
- **WHEN** `MINIMAX_API_KEY=sk-env` is set in the environment and no flag or config key is provided
- **THEN** the sub-agent provider SHALL be initialised with `sk-env`

#### Scenario: Env var and flag both set
- **WHEN** both `MINIMAX_API_KEY=sk-env` and `--sub-agent-key sk-flag` are provided
- **THEN** the CLI flag SHALL take precedence and the sub-agent provider SHALL use `sk-flag`

#### Scenario: Env var and config both set
- **WHEN** both `MINIMAX_API_KEY=sk-env` and `sub_agent.api_key = "sk-config"` are present and no CLI flag is given
- **THEN** the config file value SHALL take precedence over the env var and the sub-agent provider SHALL use `sk-config`

---

### Requirement: Graceful Absence of Sub-Agent Provider
If no sub-agent provider can be constructed (no provider config, no key from any source), the application SHALL start normally without the `spawn_agent` tool. No error or warning SHALL prevent the orchestrator from running.

#### Scenario: Orchestrator starts without sub-agent config
- **WHEN** neither `[sub_agent]` config, `--sub-agent-key`, nor `MINIMAX_API_KEY` is provided
- **THEN** the orchestrator SHALL start successfully and run without the `spawn_agent` tool in its tool list

#### Scenario: Incomplete sub-agent config (provider set but no key)
- **WHEN** `[sub_agent]` section specifies `provider` and `model` but no API key is available from any source
- **THEN** the application SHALL log a warning that sub-agent capability is unavailable and SHALL NOT register `spawn_agent`

---

### Requirement: Provider Independence
The orchestrator and the sub-agent SHALL use completely independent provider instances. Their API keys, base URLs, models, and rate-limit state SHALL not be shared.

#### Scenario: Different API keys for orchestrator and sub-agent
- **WHEN** `config.toml` specifies `[provider] api_key = "sk-orch"` and `[sub_agent] api_key = "sk-sub"`
- **THEN** requests made by the orchestrator SHALL use `sk-orch` and requests made by sub-agents SHALL use `sk-sub`; neither key SHALL appear in the other's HTTP calls

#### Scenario: Same provider type, independent instances
- **WHEN** both orchestrator and sub-agent use the same provider type (e.g. OpenAI-compatible) with different models
- **THEN** each SHALL hold its own provider instance and changing one's configuration at runtime SHALL not affect the other
