## Why

Pi Rust no tiene skills — prompts expandibles on-demand que guían al modelo en workflows complejos. Sin skills, OpenSpec no funciona correctamente (el modelo escribe archivos directamente en vez de seguir el flow de `openspec new change` → artifacts). El Pi TS original carga SKILL.md desde `.pi/skills/` y los expone como slash commands invocables.

## What Changes

- **Skill loader** — parsea archivos SKILL.md (YAML frontmatter + markdown body) desde `.pi/skills/` (proyecto) y `~/.pi/skills/` (global)
- **SkillTool** — cada skill se registra como un tool que el orquestador puede invocar. Al ejecutarse, crea un sub-agent con las instrucciones del skill como system prompt (Opción B: aislado)
- **Slash commands en TUI** — el usuario puede escribir `/opsx:propose` directamente y se ejecuta el skill correspondiente
- **Auto-discovery** — OpenSpec instala skills en `.pi/skills/`, Pi Rust las detecta automáticamente al startup

## Capabilities

### New Capabilities

- `skill-loader`: Parser y discovery de SKILL.md archivos con YAML frontmatter (name, description, disable-model-invocation) desde project-local y global paths.
- `skill-tool`: Cada skill se convierte en un tool invocable. La ejecución crea un sub-agent (como spawn_agent) con el SKILL.md como system prompt, usando el sub-agent provider (MiniMax).
- `skill-slash-commands`: El TUI reconoce `/nombre:skill` como invocación directa de skills desde el input del usuario.

### Modified Capabilities

- `rust-cli`: Se cargan skills al startup y se registran como tools adicionales.

## Impact

**Código nuevo:**
- `crates/pi-cli/src/skills.rs` — loader + SkillTool implementation (~200 LOC)

**Código modificado:**
- `crates/pi-cli/src/main.rs` — cargar skills, registrar como tools
- `crates/pi-tui/src/app.rs` — detectar /slash commands en input

**Dependencias:** Ninguna nueva (YAML frontmatter se parsea con string splitting, no necesita un crate YAML completo).
