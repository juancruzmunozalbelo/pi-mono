## Why

Pi actualmente corre un solo agente con un solo provider LLM. Para tareas complejas, un modelo potente (orquestador) debería poder delegar sub-tareas de implementación a un modelo rápido y barato (sub-agente), igual que Claude Code delega a sub-agents Sonnet/Haiku.

Esto permite: orquestador en GitHub Copilot (GPT-4o, bueno para planificación) delegando a MiniMax M2.7-highspeed (rápido, barato, bueno para ejecución mecánica). El orquestador mantiene el contexto global mientras los sub-agentes ejecutan en contextos aislados.

## What Changes

- **Nuevo tool `spawn_agent`** — permite al orquestador crear sub-agentes efímeros que ejecutan tareas con su propio provider/modelo y herramientas
- **Sub-agent runner** — sistema de ejecución de agentes efímeros dentro de un tool call, con timeout y cancellation
- **Configuración multi-provider** — el CLI soporta configurar un provider para el orquestador y otro para sub-agentes
- **Config TOML extendido** — nueva sección `[sub_agent]` con provider, model, api_key, system_prompt
- **CLI flags** — `--sub-agent-key` para pasar la API key del sub-agente por línea de comandos

## Capabilities

### New Capabilities

- `spawn-agent-tool`: Tool que el orquestador invoca para delegar tareas. Crea un Agent efímero con provider/modelo diferente, le pasa un prompt + contexto, lo ejecuta hasta completarse, y devuelve el resultado como texto. Incluye timeout (5 min), cancellation, y manejo de errores.
- `multi-provider-config`: Configuración que permite definir un provider principal (orquestador) y un provider secundario (sub-agentes) en `config.toml` y via CLI flags. Soporta API keys separadas por provider.

### Modified Capabilities

<!-- ninguna — es funcionalidad nueva sobre la base existente -->

## Impact

**Código afectado:**
- `crates/pi-cli/` — nuevo archivo `spawn_agent.rs`, cambios en `main.rs` (registro de tool), `config.rs` (sección sub_agent), `cli.rs` (nuevo flag)
- `crates/pi-tools/` — NO se modifica (evitar dependencia circular pi-tools → pi-agent)

**Dependencias:** Ninguna nueva — usa `pi-agent`, `pi-ai`, y `pi-tools` existentes.

**Arquitectura:** El `SpawnAgentTool` vive en `pi-cli` (no en `pi-tools`) para evitar la dependencia circular `pi-tools → pi-agent → pi-tools`. Implementa el trait `pi_tools::Tool` pero se registra solo desde el CLI.
