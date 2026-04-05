## Why

Pi Rust se queda sin contexto después de ~20 mensajes porque no tiene compaction. Cada mensaje (user + assistant + tool results) se acumula sin límite hasta que excede la context window del modelo, momento en que el provider devuelve un error. Esto hace que Pi sea inusable para sesiones de trabajo reales.

Además, el TUI renderiza mensajes como texto plano — sin bold, sin code blocks, sin syntax highlighting. Esto hace que la experiencia sea pobre comparada con cualquier otro coding agent.

## What Changes

**Compaction (context window management):**
- Estimación de tokens por mensaje (heurística chars/4 o tokenizer)
- Detección de cuándo el contexto se acerca al límite del modelo
- Compaction automática: resumir mensajes antiguos vía el LLM, mantener los recientes
- Hook `transform_context` ya existe — la compaction se implementa como un uso concreto de ese hook
- Mensajes compactados se reemplazan por un summary message

**Markdown rendering en TUI:**
- Parsing de markdown en mensajes del assistant (bold, italic, inline code, code blocks, headings, lists, blockquotes, links)
- Syntax highlighting para code blocks (al menos los lenguajes comunes: rust, python, javascript, bash, json)
- Wrapping de texto respetando anchos de terminal
- Rendering de tool output con formato

## Capabilities

### New Capabilities

- `context-compaction`: Sistema de compaction automática que estima tokens, detecta overflow inminente, y resume mensajes antiguos para mantener el contexto dentro del límite del modelo. Se integra via el hook `transform_context` existente.
- `markdown-tui`: Rendering completo de markdown en el TUI con ratatui: headings, bold/italic, inline code, fenced code blocks con syntax highlighting, listas, blockquotes, y word wrapping ANSI-aware.

### Modified Capabilities

- `rust-agent-core`: El hook transform_context se usa para inyectar compaction automática.
- `rust-tui`: El widget de mensajes pasa de texto plano a markdown renderizado.

## Impact

**Código nuevo:**
- `crates/pi-agent/src/compaction.rs` — lógica de estimación de tokens y compaction
- `crates/pi-tui/src/widgets/markdown.rs` — parser + renderer de markdown para ratatui

**Código modificado:**
- `crates/pi-cli/src/main.rs` — registrar compaction hook en AgentConfig
- `crates/pi-tui/src/widgets/messages.rs` — usar markdown renderer en vez de texto plano
- `crates/pi-tui/src/app.rs` — pasar theme al markdown renderer

**Dependencias nuevas:**
- `pulldown-cmark` — parser de markdown (estándar en Rust)
- `syntect` — syntax highlighting (usado por bat, delta, etc.)
