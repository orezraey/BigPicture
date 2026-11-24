# Benchmarks — Grade Virtualizada de 1.000 Capas (600×900)

Medições reais coletadas pelo `gpui_kit::profiler::FrameTimingCollector` (mesma fonte do HUD `gpui-fps`) varrendo todas as linhas da grade virtualizada de **1.000 cards** com capas **600×900** carregadas de forma assíncrona fora da thread de UI (`bigpicture_media::CoverLoader` + `CoverCache` em disco QOI + LRU de 192 texturas na VRAM com `window.drop_image`).

## Ambiente de Teste

- **Sistema Operacional**: Windows (x86_64 MSVC)
- **Toolchain**: `rustc 1.98.1` (`edition = "2024"`, `resolver = "3"`, `--release`)
- **GPU**: NVIDIA GeForce RTX 5070 Ti (Direct3D 11.1 / DirectComposition borderless fullscreen)
- **Resolução / Refresh do Monitor**: `3440×1440 @ 240 Hz` (14 colunas visíveis por linha, ~4,17 ms de orçamento por frame a 240 Hz; 8,33 ms a 120 Hz; 16,67 ms a 60 Hz)
- **Metodologia**:
  - Descarte dos 8 primeiros frames frios de inicialização de janela/atlas (idêntico ao `WARMUP_FRAMES` do `gpui-fps`).
  - Rolagem contínua de ponta a ponta pelas 72 linhas (1.000 cards) via `--benchmark-scroll`.
  - Comparação entre **cache frio** (decodificando 1.000 arquivos locais + redimensionando para `600×900` + persistindo QOI em disco em paralelo) e **cache quente** (servindo os `600×900` diretamente do cache em disco).

## Comandos Reproduzíveis

```powershell
# Com gpui-fast (feature padrão)
cargo run --release -- --benchmark-scroll

# Sem gpui-fast (backend upstream gpui-pre)
cargo run --release --no-default-features -- --benchmark-scroll
```

## Resultados (`Window::draw` em milissegundos)

| Backend | Estado do Cache em Disco | Frames Medidos | Média (ms) | Mín (ms) | Mediana P50 (ms) | P95 (ms) | P99 (ms) | Máx (ms) | Disk Hits / Misses |
| :--- | :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| **`gpui-fast`** (`default`) | **Quente** | 347 | **2,925** | 1,238 | **2,168** | **5,086** | **6,535** | 8,643 | 1.104 / 0 |
| **`gpui-fast`** (`default`) | **Frio** (decode + resize) | 407 | **4,215** | 1,323 | **4,245** | **6,982** | **8,333** | 10,432 | 259 / 1.000 |
| **`gpui-pre`** (`--no-default-features`) | **Quente** | 316 | **8,614** | 3,527 | **7,042** | **15,888** | **40,104** | 48,119 | 1.319 / 0 |
| **`gpui-pre`** (`--no-default-features`) | **Frio** (decode + resize) | 454 | **8,912** | 3,966 | **7,565** | **18,470** | **36,115** | 56,868 | 387 / 1.000 |

## Análise

> **Superada pela remedição de 24/11/2026 (abaixo).** Remedindo de forma intercalada, a vantagem de ~3× do `gpui-fast` sobre o `gpui-pre` não se repetiu. As tabelas acima são execuções únicas e ficam só como histórico.

1. **Ganho do `gpui-fast` vs `gpui-pre` (cache quente)**:
   - **Mediana (P50) 3,25× menor**: cai de `7,042 ms` para **`2,168 ms`** — mantendo folga confortável até para painéis de **240 Hz** (`4,17 ms`).
   - **Média 2,94× menor**: cai de `8,614 ms` para **`2,925 ms`**.
   - **Cauda lenta (P99) 6,14× menor**: cai de `40,104 ms` para **`6,535 ms`**, eliminando engasgos perceptíveis durante rolagem rápida e upload de novas capas `600×900` no atlas da GPU.
2. **Isolamento da Thread de UI (`bigpicture-media`)**:
   - Mesmo com **1.000 cache misses simultâneos** (decodificação + `resize_to_fill(600, 900)` + escrita atômica em disco), o `gpui-fast` sustentou P99 de **`8,333 ms`** (acima de 120 FPS contínuos), comprovando que o trabalho de I/O e redimensionamento ocorre integralmente nas worker threads do `CoverLoader`.

---

## Remedição (24/11/2026) e comparação com Rune

Medido na mesma sessão, intercalando os quatro binários: 5 rodadas com cache quente e 3 com cache frio (cada rodada fria com um `--cache-dir` novo). As tabelas mostram a **mediana entre rodadas** de cada estatística. Scripts: [`spikes/rune-grid/run-bench.ps1`](../spikes/rune-grid/run-bench.ps1) roda a bateria e [`spikes/rune-grid/summarize-bench.ps1`](../spikes/rune-grid/summarize-bench.ps1) calcula as medianas. Ambiente igual ao da seção acima: RTX 5070 Ti, 3440×1440 @ 240 Hz, escala 1,0, 14 colunas, borderless fullscreen no único monitor conectado.

Os números do GPUI ficaram abaixo da tabela original, sobretudo no `gpui-pre` (P50 de 7,04 para 2,40 ms). Código e lockfile são os do commit `00c0420`. A causa da diferença não foi investigada; a tabela original era de execuções únicas. **Use esta seção como referência atual.**

### O que foi comparado

- **GPUI**: `bigpicture-app` (`crates/app`), com e sem `gpui-fast`.
- **Rune**: [`spikes/rune-grid`](../spikes/rune-grid), crate isolado com workspace e `Cargo.lock` próprios. Não entra no workspace, no lockfile raiz nem no CI. Rune fixado na rev `a1cfa9c` da branch `dev`: o repositório `anomalyco/rune` não tem `main` nem release, e o README pede para fixar uma rev de `dev`.
- Os dois usam **os mesmos crates puros**: `bigpicture-core` (`GridMetrics`), `bigpicture-providers` (catálogo local) e `bigpicture-media` (`CoverLoader` com o mesmo número de workers e o mesmo cache QOI em disco). A chave do cache depende do caminho da capa, então os dois rodam da raiz do repositório com os mesmos caminhos.
- Mesmo card (capa 200×300, rodapé com título e subtítulo truncados, borda, raio 8, hover), mesma linha (14 cards, padding 56, gap 24, altura 374), mesmo LRU de 192 capas na GPU (soltar o handle libera a textura nos dois) e capas sem mipmaps nos dois (`RenderImage` no GPUI, `Pixels::set` no Rune).
- Mesmo driver de rolagem: 250 ms de espera, 221 saltos instantâneos para a linha alvo (no topo) a cada 16 ms, 200 ms de assentamento, 8 frames de aquecimento descartados. O cursor fica parado sobre o cabeçalho em todas as rodadas (ver "hover" abaixo).
- Diferenças que ficaram: o GPUI desenha o HUD `gpui-fps` e a scrollbar e roda com o profiler ligado; o Rune não tem HUD equivalente. O Rune usa DX12 (wgpu); o GPUI no Windows usa Direct3D 11.

### Mapeamento das métricas

| GPUI | Rune | Cobre |
| :--- | :--- | :--- |
| `FrameTiming::draw_duration` (`Window::draw`) | `FrameEncoded::update_ms` | view, layout, prepaint/paint, montagem da cena |
| `PresentTiming::present_duration` | `FrameEncoded::encode_ms` | upload de texturas, preparo e submissão (tempo de CPU, não de GPU) |
| `ingest_loaded_covers`, numa task **fora** do draw | parte do `poll`, **dentro** do `update_ms` | receber a capa decodificada, converter, criar a imagem e pôr no LRU |

Por causa da última linha, o Rune também é reportado com "update sem ingestão" (o tempo do `poll` gasto com capas é medido em cada frame e subtraído), e a ingestão aparece separada nos dois lados. "Frames > 4,17 ms" conta frames cuja fase de draw/update passou do orçamento de 240 Hz; no Rune, a conta inclui a ingestão. "CPU total" soma draw+present+ingestão (GPUI) ou update+encode (Rune, que já inclui a ingestão) em toda a varredura.

Para registrar present e ingestão, o `ScrollBenchmarkRecorder` do app ganhou essas duas medições. É a única mudança no app principal, e ela não altera o que é desenhado.

### Cache quente (mediana de 5 rodadas)

| Backend | Frames | Draw/update P50 | P95 | P99 | Máx | Present/encode P50 | P95 | P99 | Frames > 4,17 ms | Ingestão (total, por capa) | CPU total |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `gpui-fast` | 345 | **1,93** | **4,30** | **5,39** | 6,87 | 0,82 | 1,23 | 1,77 | **25** (7%) | 119 ms, 0,12 ms | 1,27 s |
| `gpui-pre` | 355 | 2,40 | 4,74 | 5,73 | 6,78 | **0,19** | **0,29** | **0,40** | 69 (19%) | 120 ms, 0,12 ms | **1,20 s** |
| Rune `stepped` | 186 | 5,32 | 7,77 | 8,15 | 8,33 | 0,43 | 5,76 | 6,31 | 128 (69%) | 312 ms, 0,34 ms | 1,26 s |
| Rune `stepped`, update sem ingestão | | 2,65 | 7,13 | 7,51 | 7,84 | | | | | | |
| Rune `stepped-commit` | 182 | 6,20 | 8,01 | 8,40 | 9,19 | 0,38 | 5,80 | 6,26 | 131 (72%) | 312 ms, 0,34 ms | 1,35 s |

No Rune `stepped`, update+encode no mesmo frame ficou em P95 12,8 ms, P99 13,6 ms e máximo 14,4 ms. No `gpui-fast`, mesmo somando os P99 de draw e present (5,4 + 1,8 ms), o frame fica abaixo de 8,33 ms (120 Hz).

### Cache frio (decode + resize + escrita QOI, mediana de 3 rodadas)

| Backend | Frames | Draw/update P50 | P95 | P99 | Máx | Present/encode P50 | P95 | P99 | Frames > 4,17 ms | Ingestão | CPU total |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `gpui-fast` | 422 | **3,51** | **6,50** | **7,20** | **8,76** | 1,04 | 1,80 | 2,18 | 173 (41%) | 217 ms | 2,23 s |
| `gpui-pre` | 439 | 3,91 | 7,74 | 8,38 | 9,14 | **0,26** | **0,45** | **0,61** | 197 (45%) | 226 ms | 2,19 s |
| Rune `stepped` | 250 | 6,00 | 10,03 | 10,41 | 11,31 | 0,55 | 9,32 | 10,28 | 172 (69%) | 400 ms | **1,97 s** |
| Rune, update sem ingestão | | 3,11 | 9,21 | 10,14 | 10,62 | | | | | | |

No Rune, update+encode no mesmo frame ficou em P95 18,8 ms e P99 20,2 ms.

### Leitura

1. **Por frame, o GPUI é mais rápido e mais previsível.** Com cache quente, o draw do `gpui-fast` tem P50 de 1,9 ms e P95 de 4,3 ms. O update do Rune tem 5,3 e 7,8 ms; tirando a ingestão de capas, 2,7 e 7,1 ms. O encode do Rune também tem cauda maior que o present do GPUI (P95 de 5,8 contra 1,2 ms). A 240 Hz, 7% dos frames do `gpui-fast` passam do orçamento, contra 69% no Rune. A 120 Hz o GPUI cabe com folga; o Rune passa de 8,33 ms em mais de 5% dos frames.
2. **O trabalho total é parecido.** Somando tudo na varredura, deu 1,27 s no `gpui-fast` e 1,26 s no Rune com cache quente; com cache frio, 2,23 s contra 1,97 s. O Rune desenha cerca de metade dos frames (186 contra 345), cada um mais pesado. Ele também desenhou menos frames que os 221 passos: parte dos passos não gerou um frame desenhado próprio (a causa não foi investigada).
3. **Nos frames lentos do Rune, sempre houve ingestão de capas ou um passo de rolagem** (nunca nenhum dos dois). Com `RUNE_COMMIT_BUDGET_MS=5`, nenhum commit da view passou de 5 ms, então o custo não está em reexecutar a view.
4. **A ingestão custa ~3× mais por capa no Rune** (0,34 contra 0,12 ms), mas quase todo esse custo é nosso: o spike copia os 2,16 MB de cada capa (`pixels().to_vec()`) antes de `Pixels::set`. Um `RgbaFrame::into_pixels()` no `bigpicture-media` tiraria essa cópia. Não foi testado.
5. **`stepped-commit`**, que força a view a rodar a cada passo como o `cx.notify()` do GPUI, deixa o update sem ingestão igual (P50 de 2,66 ms) e aumenta o CPU total em ~7%. Reexecutar a view com `virtual_rows` é barato.
6. **Hover distorce a medição.** Na primeira bateria, o cursor ficou sobre a grade. O Rune desenhou ~730 frames por varredura, a maioria de hover (o card sob um ponteiro parado muda a cada salto), e a mediana dele caiu para 0,4 ms. Esses números foram descartados, e o script agora fixa o cursor sobre o cabeçalho.
7. **`gpui-fast` contra `gpui-pre`**: o `gpui-fast` tem draw ~20% mais rápido na mediana e um terço dos frames acima de 4,17 ms (25 contra 69). Em troca, o present é ~4× mais caro (0,82 contra 0,19 ms), e o CPU total é ~6% maior (1,27 contra 1,20 s). A vantagem de ~3× da tabela original não se repetiu.
8. **Ressalva**: é um único cenário (saltos de rolagem numa grade de imagens), numa única máquina Windows com um único monitor. Não mede latência de input, animações, texto longo nem Linux. O spike do Rune tem uma tarde de trabalho contra um app GPUI mais maduro.

### Limitações encontradas no Rune

**Distribuição e build**
- Sem release e sem branch `main`: só `dev`, fixada por rev. O projeto não aceita pull requests, só issues.
- O Rune corrige sete crates publicados (`wgpu-hal`, `naga`, `winit`, `sctk-adwaita`, `parley_engine`, `accesskit_windows`, `accesskit_macos`). `[patch]` de dependência não se propaga, então o app precisa repetir os sete `[patch.crates-io]` apontando para o git do Rune (ver `spikes/rune-grid/Cargo.toml`). Isso prende o app aos forks de `winit` e `wgpu-hal` do Rune. Um conflito com outra dependência que use `winit` ou `wgpu` publicados é possível, mas não foi testado.
- O `rust-toolchain.toml` do Rune pede 1.99.1. Nesta rev ele compilou em 1.98.1, mas isso pode mudar a qualquer commit.
- A correção local que existia em `C:\projetos\rune-fix` (espera de frame-latency do swap chain no resize, no Windows) já está upstream nesta rev (#1545).

**Janela nativa (Windows)**
- `full_screen(true)` é **ignorado em silêncio** quando `WindowOptions::resizable` é `false`: `window/host/create.rs` só aplica `open_as` a janelas redimensionáveis. Foi preciso usar `resizable: true`.
- Com `borderless: true` e sem fullscreen efetivo, a janela abriu **com a barra de título nativa** (testado com e sem `full_screen`, ambos com `resizable: false`). Com o fullscreen funcionando, a barra some. A causa não foi investigada.
- `full_screen` usa `Fullscreen::Borderless(None)`, ou seja, o monitor atual. Para escolher o monitor há dois caminhos: posicionar a janela com `position`/`physical_size` do display escolhido na lista de `run_windows` (o que o spike faz; a lista é tirada na inicialização), ou `AppHandle::place`/`WindowHandle::place(DisplayTarget, OverlayPlacement::FullScreen)`, que move e redimensiona a janela para o display e não faz nada no Wayland. O `--monitor N` do spike **não foi testado com dois monitores**, porque só há um conectado.

**Virtualização e imagens**
- O overscan de `virtual_rows` é fixo em 2 viewports por lado (constante interna `OVERSCAN`). A 3440×1440 isso monta cerca de 260 cards, mais que o LRU de 192. Pedir capas no builder da linha causou um ciclo de expulsão e recarga (12.688 leituras de cache numa varredura). A correção foi pedir decodificação só em `on_visible_rows` (linhas visíveis ±1). Linhas do overscan mostram placeholder até ficarem visíveis.
- Não há grade virtualizada pronta: as linhas de 14 cards são montadas à mão (no GPUI também).
- `AppSpec` usa ponteiros de função (`init`, `view`, `poll`, `setup`), sem captura. O estado precisa carregar tudo; `App::init` e `WindowSpec::state` aceitam closure só para o estado inicial.
- `El::hover` é uma cor sobreposta. Não encontrei como trocar a cor da borda no hover, como o card do GPUI faz; o spike usa a sobreposição.

**Input**
- Não encontrei suporte a gamepad (o GPUI também não tem; o BigPicture usa `gilrs` numa thread própria). No Rune isso encaixa bem: a thread chama o `Waker` de `AppSpec::setup` e o `poll` drena o canal, como o spike faz com as capas.
- Não encontrei navegação espacial por direcional. Há foco por Tab, `focusable`, anéis de foco e `scroll_into_view`; mover o foco em grade fica com o app, como no GPUI.

**Pontos a favor do Rune observados**
- Rolar não reexecuta a view, e forçar a reexecução a cada passo custa pouco (item 5).
- Trabalho total de render igual ou menor que o do GPUI, com metade dos frames.
- `on_frame_encoded` separa build e encode por frame sem profiler ligado. `RUNE_COMMIT_BUDGET_MS` e os avisos de performance em debug apontam a origem de commits caros.
- Ferramentas headless (`--snap`, `--tree`, input por script) pensadas para um agente de IA verificar a UI sem abrir janela.
- `Pixels::set` pode ser chamado de qualquer thread e acorda só as janelas que mostram a imagem, o que serve para frames de vídeo (GStreamer `appsink`) sem reexecutar a view.

### Como reproduzir

```powershell
# Na raiz do BigPicture
cargo build --release
cargo build --release --no-default-features --target-dir target\gpui-pre
cargo build --release --manifest-path spikes\rune-grid\Cargo.toml
powershell -NoProfile -ExecutionPolicy Bypass -File spikes\rune-grid\run-bench.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File spikes\rune-grid\summarize-bench.ps1
```

Uma execução avulsa do Rune, sempre a partir da raiz do repositório: `spikes\rune-grid\target\release\rune-grid.exe --benchmark-scroll [--mode stepped|stepped-commit] [--monitor N] [--cache-dir DIR]`.
