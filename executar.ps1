# Compila o BigPicture com o código atual e abre o launcher. Use executar.bat (dois cliques).
#
# Se o launcher desta pasta estiver aberto, pergunta antes de fechá-lo: o Windows não deixa
# substituir um .exe em uso. Só o que mudou é compilado de novo. Argumentos passados ao .bat vão
# para o launcher (por exemplo: executar.bat --monitor 1).

param([Parameter(ValueFromRemainingArguments = $true)] [string[]] $AppArgs)

$ErrorActionPreference = 'Stop'

function Sair($codigo) {
    if ($codigo -ne 0) { Read-Host 'Pressione Enter para fechar' | Out-Null }
    exit $codigo
}

# Qualquer erro inesperado é mostrado antes de a janela fechar.
trap {
    Write-Host ''
    Write-Host "Erro: $_" -ForegroundColor Red
    Sair 1
}

# A pasta do projeto: o launcher lê assets\ (capas, trailers, cache) relativo a ela.
Set-Location $PSScriptRoot
$exe = Join-Path $PSScriptRoot 'target\release\bigpicture-app.exe'

# O cargo do rustup, que respeita o rust-toolchain.toml do projeto.
$cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
if (-not (Test-Path $cargo)) {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Write-Host 'O Rust não foi encontrado: instale o rustup (https://rustup.rs) e tente de novo.' -ForegroundColor Red
        Sair 1
    }
    $cargo = 'cargo'
}

# GStreamer (trailers): a compilação precisa do pacote de desenvolvimento e o launcher precisa das
# DLLs da pasta bin no PATH. Uma janela aberta antes da instalação não tem o PATH novo, então a
# pasta é posta no PATH deste processo (o launcher herda).
$gstRoot = [Environment]::GetEnvironmentVariable('GSTREAMER_1_0_ROOT_MSVC_X86_64', 'Machine')
if (-not $gstRoot) { $gstRoot = 'C:\Program Files\gstreamer\1.0\msvc_x86_64\' }
$gstBin = Join-Path $gstRoot 'bin'
if (-not (Test-Path (Join-Path $gstBin 'gobject-2.0-0.dll'))) {
    Write-Host 'O GStreamer não foi encontrado. Instale o runtime e o pacote de desenvolvimento' -ForegroundColor Red
    Write-Host '(MSVC x86_64, 1.26) de https://gstreamer.freedesktop.org/download/ e tente de novo.' -ForegroundColor Red
    Sair 1
}
$env:GSTREAMER_1_0_ROOT_MSVC_X86_64 = $gstRoot
if (($env:PATH -split ';') -notcontains $gstBin) { $env:PATH = "$gstBin;$env:PATH" }

$aberto = @(Get-Process bigpicture-app -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe })
if ($aberto.Count -gt 0) {
    Write-Host 'O BigPicture está aberto e precisa ser fechado para ser atualizado.'
    Write-Host 'Jogos abertos por ele continuam rodando.'
    $resposta = Read-Host 'Fechar agora? (S/n)'
    if ($resposta -match '^[nN]') { Sair 1 }
    # Pede para a janela fechar; se não fechar em 5 segundos, encerra.
    $aberto | ForEach-Object { [void]$_.CloseMainWindow() }
    $aberto | Wait-Process -Timeout 5 -ErrorAction SilentlyContinue
    $aberto | Where-Object { -not $_.HasExited } | Stop-Process -Force -ErrorAction SilentlyContinue
}

Write-Host 'Compilando o BigPicture...'
& $cargo build --release -p bigpicture-app
if ($LASTEXITCODE -ne 0) {
    Write-Host ''
    Write-Host 'A compilação falhou: veja o erro acima.' -ForegroundColor Red
    Sair 1
}

$inicio = @{ FilePath = $exe; WorkingDirectory = $PSScriptRoot }
if ($AppArgs) { $inicio.ArgumentList = $AppArgs }
Start-Process @inicio
Sair 0
