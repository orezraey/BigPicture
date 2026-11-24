# Lê o log do run-bench.ps1 e imprime, por fase (warm/cold) e backend, a mediana entre rodadas
# de cada estatística, mais o "CPU total" (soma de draw+present ou update+encode na varredura).
param([string]$Log = (Join-Path $PSScriptRoot "bench-results.log"))

$recs = @(); $cur = $null
foreach ($l in Get-Content $Log) {
    if ($l -match '^== (\S+) (\S+)') {
        $cur = @{ phase = ($matches[1] -replace '#.*', ''); cfg = $matches[2] }
        $recs += $cur
        continue
    }
    if (-not $cur) { continue }
    if ($l -match 'draw over 4.17 ms: (\d+)/') { $cur['over'] = [double]$matches[1]; continue }
    $prefix = switch -Regex ($l) {
        '^\s+update \(' { 'upd_'; break }
        '^\s+update sem' { 'upx_'; break }
        '^\s+encode \(' { 'enc_'; break }
        '^\s+update\+encode' { 'tot_'; break }
        '^\s+present' { 'pres_'; break }
        '^\s+ingest' { 'ing_'; break }
        '^\s+frames >' { 'slow_'; break }
        'BENCHMARK_RESULT' { ''; break }
        default { $null }
    }
    if ($null -eq $prefix) { continue }
    foreach ($m in [regex]::Matches($l, '(\w+)=([\d.]+)')) {
        $cur[$prefix + $m.Groups[1].Value] = [double]$m.Groups[2].Value
    }
    if ($l -match 'over_4_17ms=(\d+)') { $cur['over'] = [double]$matches[1] }
}

function Get-Median($xs) {
    $s = @($xs | Where-Object { $null -ne $_ } | Sort-Object)
    if ($s.Count -eq 0) { return $null }
    $s[[int][math]::Floor(($s.Count - 1) / 2)]
}

foreach ($g in ($recs | Where-Object { $_.phase -ne 'prewarm' } | Group-Object { "$($_.phase) $($_.cfg)" })) {
    $r = $g.Group
    $keys = $r[0].Keys | Where-Object { $_ -notin 'phase', 'cfg' } | Sort-Object
    $med = ($keys | ForEach-Object { $k = $_; "$k=$(Get-Median ($r | ForEach-Object { $_[$k] }))" }) -join ' '
    $cpu = if ($r[0].ContainsKey('tot_mean_ms')) {
        Get-Median ($r | ForEach-Object { $_.samples * $_.tot_mean_ms / 1000 })
    } else {
        Get-Median ($r | ForEach-Object { $_.samples * ($_.mean_ms + $_.pres_mean_ms) / 1000 })
    }
    "$($g.Name) n=$($r.Count) cpu_total_s=$([math]::Round($cpu, 3)) $med"
    ""
}
