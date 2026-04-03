# Context Engineering Plan — Sub-Agent Strategy

## Estructura del prompt para cada sub-agent

```
┌─────────────────────────────────┐
│ 1. SNAPSHOT del estado actual   │  ← qué archivos existen, qué compila
│ 2. TIPOS y APIs relevantes     │  ← contenido literal de types.rs, traits, etc.
│ 3. TAREA concreta              │  ← qué archivo crear/modificar
│ 4. PROTOCOLO de referencia     │  ← datos del deep dive (SSE format, headers, etc.)
│ 5. REGLAS de verificación      │  ← cargo check, cargo test, clippy
└─────────────────────────────────┘
```

## Contexto por grupo

| Grupo | Contexto mínimo (código compilado) | Contexto de referencia (deep dives) |
|-------|-----------------------------------|-------------------------------------|
| **3. Providers** | types.rs + sse.rs | Deep dive OpenAI + Deep dive Anthropic (protocolo exacto, headers, message conversion) |
| **4. Copilot Auth** | types.rs (Model, ApiType) | Deep dive OAuth flow (device code, polling, token exchange) |
| **5. Tools** | Tool trait definition | Deep dive Tools (schema, bash process mgmt, edit fuzzy match) |
| **6. Agent Runtime** | types.rs + LlmProvider trait + Tool trait | Deep dive Agent Core (state machine, parallel execution, hooks, events) |
| **7. TUI** | AgentEvent enum + types.rs | Deep dive TUI (no portar 1:1 — usar ratatui idioms) |
| **8. CLI** | Todo lo anterior (traits, no impl) | design.md (config format, session format) |

## Técnicas

### 1. Inyectar código compilado, no specs
En vez de describir "el trait LlmProvider tiene un método chat que...", copiar el `.rs` literal. El sub-agent lee código real, no prosa.

### 2. Separar read-only context de write target
Decir explícitamente: "Lee estos archivos para contexto, NO los modifiques. Escribí SOLO en estos otros archivos."

### 3. Golden files para tests
Para los providers, preparar fixtures con SSE responses reales grabadas del TS. Así testea contra datos reales, no inventados.

### 4. Verificación obligatoria al final
Cada sub-agent DEBE correr:
```bash
cargo check --workspace
cargo test -p <crate>
cargo clippy -p <crate> -- -D warnings
```
Si falla, arregla antes de terminar.

### 5. Paralelismo por independencia de crates

```
Grupo 3 (providers) ─┐
Grupo 4 (auth)       ├─ EN PARALELO (no comparten archivos)
Grupo 5 (tools)      ┘
                      │
                      ▼
Grupo 6 (agent)      ← espera a 3 + 5 (depende de ambos traits)
                      │
                      ▼
Grupo 7 (TUI)  ──┐
Grupo 8 (CLI)  ──┘   ← parcialmente en paralelo
                      │
                      ▼
Grupo 9 (integration) ← al final
```

### 6. Context budget por complejidad
- Tareas mecánicas (scaffold, config) → **Haiku**, prompt corto
- Tareas con lógica de protocolo (providers, agent loop) → **Sonnet**, prompt largo con deep dive data
- Debugging/fix después de fallo → **Sonnet** con error output literal

## Dependencia entre crates (DAG)

```
pi-ai ────────┐
              ├──→ pi-agent ──→ pi-tui ──→ pi-cli
pi-tools ─────┘
```

pi-ai y pi-tools son hojas — se pueden implementar en paralelo sin conflictos.
