# Roda GPUI (gpui-fast e gpui-pre) e Rune intercalados, a partir da raiz do BigPicture, e grava
# as linhas BENCHMARK_RESULT em um log. Pré-requisitos (na raiz do repositório):
#   cargo build --release
#   cargo build --release --no-default-features --target-dir target\gpui-pre
#   cargo build --release --manifest-path spikes\rune-grid\Cargo.toml
param(
    [int]$WarmRounds = 5,
    [int]$ColdRounds = 3,
    [string]$Log = "spikes\rune-grid\bench-results.log"
)
# stderr de executável nativo não deve abortar o script (Windows PowerShell 5).
$ErrorActionPreference = "Continue"
Set-Location (Join-Path $PSScriptRoot "..\..")

$runs = @(
    @{ Name = "gpui-fast"; Exe = "target\release\bigpicture-app.exe"; Args = @() },
    @{ Name = "gpui-pre"; Exe = "target\gpui-pre\release\bigpicture-app.exe"; Args = @() },
    @{ Name = "rune-stepped"; Exe = "spikes\rune-grid\target\release\rune-grid.exe"; Args = @("--mode", "stepped") },
    @{ Name = "rune-stepped-commit"; Exe = "spikes\rune-grid\target\release\rune-grid.exe"; Args = @("--mode", "stepped-commit") }
)

"# $(Get-Date -Format o)" | Set-Content $Log -Encoding ascii

Add-Type -AssemblyName System.Windows.Forms
function Invoke-Run($run, $cache, $label) {
    # Cursor parado no canto superior esquerdo (sobre o cabeçalho): hover sob um ponteiro
    # parado gera frames durante a rolagem, então a posição precisa ser a mesma em toda rodada.
    [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point(4, 4)
    $argList = @("--benchmark-scroll") + $run.Args
    if ($cache) { $argList += @("--cache-dir", $cache) }
    $out = & $run.Exe @argList 2>&1 | Where-Object { "$_" -match "BENCHMARK_RESULT|^\s+(update|encode|present|draw|ingest|frames)" }
    $lines = @("== $label $($run.Name)") + ($out | ForEach-Object { "$_" })
    $lines | Add-Content $Log -Encoding ascii
    $lines
    Start-Sleep -Seconds 2
}

# Aquece o cache em disco padrão antes das rodadas quentes.
Invoke-Run $runs[0] $null "prewarm"

for ($i = 1; $i -le $WarmRounds; $i++) {
    foreach ($run in $runs) { Invoke-Run $run $null "warm#$i" }
}

for ($i = 1; $i -le $ColdRounds; $i++) {
    foreach ($run in $runs | Select-Object -First 3) {
        $cache = Join-Path $env:TEMP ("bp-cold-" + [guid]::NewGuid())
        Invoke-Run $run $cache "cold#$i"
        Remove-Item -Recurse -Force $cache -ErrorAction SilentlyContinue
    }
}
