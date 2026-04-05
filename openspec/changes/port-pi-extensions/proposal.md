## Why

Pi Rust tiene el core funcional (providers, tools, agent loop, spawn_agent) pero carece de funcionalidades de producción que el ecosistema TS de Pi ya resolvió via extensiones. En vez de reinventar, portamos las 4 extensiones más valiosas de [tmustier/pi-extensions](https://github.com/tmustier/pi-extensions) adaptándolas a Rust nativo — sin necesidad de un sistema de extensiones dinámico.

Esto convierte a Pi Rust de un prototipo a una herramienta usable para sesiones largas de desarrollo real.

## What Changes

**4 extensiones portadas como funcionalidad nativa:**

- **Ralph-Wiggum (long-running task loop)** — Comando `/ralph start <task>` que ejecuta un loop iterativo: sub-agent trabaja → reflexiona → ajusta → sigue. Persiste estado en `.ralph/` para pause/resume. Usa `spawn_agent` internamente para cada iteración. Incluye reflection checkpoints y detección de completitud (`COMPLETE` marker).

- **Usage dashboard** — Subcomando `pi usage` que lee sesiones JSON de `~/.pi/sessions/`, calcula tokens (input/output/cache) y costos por modelo/provider, agrupa por periodo (hoy, esta semana, total). Output tabular en terminal.

- **Tab status** — Hook sobre AgentEvents que actualiza el título del tab del terminal con estado: `:new`, `:running...`, `:✅` (done+commit), `:🚧` (done sin commit), `:🛑` (timeout 180s). Usa escape sequence `\x1b]0;title\x07`.

- **Agent guidance** — Carga system prompts específicos por provider/modelo desde archivos `.pi/guidance/COPILOT.md`, `.pi/guidance/MINIMAX.md`. El prompt se inyecta automáticamente según el provider activo del agente. Permite optimizar instrucciones para el orquestador vs sub-agents.

## Capabilities

### New Capabilities

- `ralph-loop`: Loop iterativo de tareas largas con state persistence, reflection, pause/resume, y detección de completitud. Integrado con spawn_agent para ejecución de cada iteración.
- `usage-dashboard`: Subcomando CLI que analiza sesiones guardadas y presenta métricas de consumo (tokens, costo) por modelo, provider, y periodo temporal.
- `tab-status`: Actualización automática del título del tab terminal basada en eventos del agente (start, running, done, timeout).
- `agent-guidance`: Carga dinámica de system prompts desde archivos de guidance por provider, inyectados al agente según el modelo activo.

### Modified Capabilities

- `rust-cli`: Se agregan subcomandos (`pi usage`, `/ralph` commands) y hooks de eventos (tab-status, guidance loading).

## Impact

**Código nuevo:**
- `crates/pi-cli/src/ralph.rs` — Ralph loop manager (~300 LOC)
- `crates/pi-cli/src/usage.rs` — Usage dashboard (~200 LOC)
- `crates/pi-cli/src/tab_status.rs` — Tab title updater (~60 LOC)
- `crates/pi-cli/src/guidance.rs` — Guidance loader (~80 LOC)
- Modificaciones a `main.rs`, `cli.rs` para registrar commands y hooks

**Código modificado:**
- `crates/pi-cli/src/cli.rs` — nuevos subcomandos y slash commands
- `crates/pi-cli/src/main.rs` — wiring de hooks y commands
- `crates/pi-cli/src/session.rs` — expose session parsing para usage dashboard

**Dependencias nuevas:** Ninguna — todo se implementa con las crates existentes (serde_json, chrono, tokio, crossterm).

**Archivos de configuración nuevos:**
- `.ralph/` — directorio de estado por proyecto para ralph loops
- `~/.pi/guidance/` — directorio de system prompts por provider
