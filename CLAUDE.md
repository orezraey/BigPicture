# CLAUDE.md — BigPicture

Launcher de jogos em tela cheia estilo Steam Big Picture para **Windows** e **Linux (X11 e Wayland)**, escrito em Rust.

## Decisões Arquiteturais Fixas (Não Reabrir)

- **UI**: GPUI via **GPUI Kit** pelo git (`branch = "main"`):
  ```toml
  gpui-kit = { git = "https://github.com/longbridge/gpui-kit", branch = "main" }
  ```
  - No crate `crates/app`, a feature padrão (`default`) habilita `"gpui-kit/gpui-fast"` (via feature `gpui-fast`).
  - Use **somente** o que `gpui_kit` reexporta (`use gpui_kit::*;`, `gpui_kit::component`, `gpui_kit::base`, `gpui_kit::assets`, `gpui_kit::platform`).
  - **Nunca** adicione `gpui`, `gpui-pre` ou `gpui-fast` diretamente em nenhum `Cargo.toml`.
  - Confirme assinaturas diretamente no código-fonte da versão resolvida no `Cargo.lock` (a API do GPUI muda com frequência; nunca assuma de memória).
- **Toolchain**:
  - `rust-toolchain.toml` com `channel = "1.98.1"`.
  - `edition = "2024"` em todos os crates do workspace.
  - `resolver = "3"` no workspace.
- **Controle (Gamepad)**:
  - `gilrs` pelo git (`https://gitlab.com/gilrs-project/gilrs`), com `default-features = false` e `features = ["xinput"]`.
  - Thread dedicada no crate `input` publicando `NavCommand` (`Up`, `Down`, `Left`, `Right`, `Confirm`, `Back`, `Menu`, `Guide`) em canal; `app` converte em actions do GPUI.
- **Vídeo / Mídia**:
  - `gstreamer-rs` pelo git (`https://gitlab.freedesktop.org/gstreamer/gstreamer-rs`, `branch = "main"`), entregando frames RGBA via `appsink` para a UI.
  - Decodificação/redimensionamento de capas (600x900) fora da thread de UI com cache em disco no crate `media`.
- **Janela / Fullscreen**:
  - Fullscreen **sempre sem borda** (borderless fullscreen), **nunca exclusivo**.
  - Seleção de monitor via argumento de linha de comando `--monitor N`.
- **Estrutura do Workspace (`crates/`)**:
  - `crates/core` (`bigpicture-core`): modelos de domínio puros, catálogo, geometria de navegação espacial.
  - `crates/providers` (`bigpicture-providers`): descoberta/provedores locais de jogos e aplicativos (lojas Steam/Epic e conquistas ficam para o Marco 2).
  - `crates/input` (`bigpicture-input`): polling de controle via `gilrs` e emissão de `NavCommand`.
  - `crates/media` (`bigpicture-media`): pipeline assíncrono de capas 600x900 com cache em disco e player de vídeo WebM via GStreamer (`appsink` RGBA).
  - `crates/system` (`bigpicture-system`): trait `SystemControl` (`list_displays`, `set_display_mode`, `launch_app`, `minimize_window`, `focus_window`, `bring_launcher_to_front`) com backends Windows (`windows`), X11 (`x11rb` + RandR + EWMH) e Wayland KDE (`plasma` / `kscreen`), retornando não suportado nos demais compositores Wayland.
  - `crates/app` (`bigpicture-app`): único crate que depende de `gpui-kit` (e `gpui-fps`). Todos os outros crates são Rust puro e testáveis sem janela.
- **Lockfile**:
  - `Cargo.lock` é versionado no Git.
  - Se o `main` do GPUI Kit quebrar a compilação, reverter no `Cargo.lock` para o último commit funcional e documentar em `docs/upstream-issues.md`.

## Fluxo de Verificação e Benchmarks

- Ao final de cada etapa, rodar:
  - `cargo check --workspace --all-targets`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test` nos crates alterados / workspace
- Benchmarks de tempo de frame da grade virtualizada com e sem `gpui-fast`:
  - Com `gpui-fast` (padrão): `cargo run --release`
  - Sem `gpui-fast`: `cargo run --release --no-default-features`
  - Registrar resultados em `docs/benchmarks.md`.
- Escopo do Marco 1: spike técnico completo (sem integração Steam/Epic nem conquistas, reservados ao Marco 2).

## Commits e PRs

- Nunca incluir coautoria ou atribuição de IA: sem `Co-Authored-By: Claude`, sem `Claude-Session:`, sem rodapé `Generated with Claude Code`.
- Autor dos commits: `orezraey <49327055+orezraey@users.noreply.github.com>` (configurado no `.git/config` local; o repositório é público).
- Depois de uma mudança grande, rodar o code review no nível `high` e aplicar as correções **antes** do commit.
