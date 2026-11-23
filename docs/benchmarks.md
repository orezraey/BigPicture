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

1. **Ganho do `gpui-fast` vs `gpui-pre` (cache quente)**:
   - **Mediana (P50) 3,25× menor**: cai de `7,042 ms` para **`2,168 ms`** — mantendo folga confortável até para painéis de **240 Hz** (`4,17 ms`).
   - **Média 2,94× menor**: cai de `8,614 ms` para **`2,925 ms`**.
   - **Cauda lenta (P99) 6,14× menor**: cai de `40,104 ms` para **`6,535 ms`**, eliminando engasgos perceptíveis durante rolagem rápida e upload de novas capas `600×900` no atlas da GPU.
2. **Isolamento da Thread de UI (`bigpicture-media`)**:
   - Mesmo com **1.000 cache misses simultâneos** (decodificação + `resize_to_fill(600, 900)` + escrita atômica em disco), o `gpui-fast` sustentou P99 de **`8,333 ms`** (acima de 120 FPS contínuos), comprovando que o trabalho de I/O e redimensionamento ocorre integralmente nas worker threads do `CoverLoader`.
