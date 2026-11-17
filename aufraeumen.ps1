# Toolless - Aufraeumen
#
# Entfernt auf Wunsch:
#   1. Ueberbleibsel der frueheren Tauri/Web-Version und den alten Klon "toolles"
#   2. den Build-Ordner "target" (wird beim naechsten "cargo run" neu erzeugt)
#   3. ALLE persoenlichen Daten der App: Einstellungen, Zwischenablage, Notizen, Verlauf,
#      Spielzeiten, Wecker - dazu den Autostart-Eintrag von Toolless
#
# Aufruf im Projektordner:   powershell -ExecutionPolicy Bypass -File .\aufraeumen.ps1
# Ohne Nachfrage:             powershell -ExecutionPolicy Bypass -File .\aufraeumen.ps1 -Alles
# Beenne Toolless vorher (auch im Tray), sonst legt es die Daten sofort neu an.

param([switch]$Alles)

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $root

function Frage($text) {
    if ($Alles) { return $true }
    $antwort = Read-Host "$text (j/N)"
    return $antwort -match '^(j|ja|y|yes)$'
}

function Entferne($pfad) {
    if (Test-Path -LiteralPath $pfad) {
        Remove-Item -LiteralPath $pfad -Recurse -Force -ErrorAction SilentlyContinue
        if (Test-Path -LiteralPath $pfad) { Write-Host "  KONNTE NICHT ENTFERNEN: $pfad" -ForegroundColor Yellow }
        else { Write-Host "  entfernt: $pfad" }
    }
}

Get-Process -Name toolless -ErrorAction SilentlyContinue | ForEach-Object {
    Write-Host "Toolless laeuft noch (PID $($_.Id))."
    if (Frage "Toolless jetzt beenden?") { Stop-Process -Id $_.Id -Force }
}

# 1. Ueberbleibsel der Tauri-Version
$alt = @(
    'src-tauri', 'toolles', 'node_modules', 'dist',
    'index.html', 'package.json', 'package-lock.json', 'tsconfig.json', 'tsconfig.node.json', 'vite.config.ts',
    'src\App.tsx', 'src\main.tsx', 'src\styles.css', 'src\types.ts', 'src\vite-env.d.ts',
    'src\components', 'src\lib', 'src\modules'
) | Where-Object { Test-Path -LiteralPath (Join-Path $root $_) }
if ($alt.Count -gt 0) {
    Write-Host "`nGefundene Ueberbleibsel der alten Version:"
    $alt | ForEach-Object { Write-Host "  $_" }
    if (Frage "Diese entfernen?") { $alt | ForEach-Object { Entferne (Join-Path $root $_) } }
}

# 2. Build-Ordner
if (Test-Path -LiteralPath (Join-Path $root 'target')) {
    $mb = [math]::Round((Get-ChildItem (Join-Path $root 'target') -Recurse -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum / 1MB)
    if (Frage "`nBuild-Ordner 'target' ($mb MB) entfernen? Der naechste Build dauert dann wieder laenger") {
        Entferne (Join-Path $root 'target')
    }
}

# 3. Persoenliche Daten
$daten = Join-Path $env:APPDATA 'Toolless'
$tauri = Join-Path $env:APPDATA 'xyz.toolless.desktop'
$vorhanden = @($daten, $tauri) | Where-Object { Test-Path -LiteralPath $_ }
if ($vorhanden.Count -gt 0) {
    Write-Host "`nPersoenliche Daten (Einstellungen, Zwischenablage, Notizen, Verlauf, Spielzeiten):"
    $vorhanden | ForEach-Object { Write-Host "  $_" }
    Write-Host "Das laesst sich nicht rueckgaengig machen. Tipp: vorher in Toolless unter Einstellungen > Daten exportieren."
    if (Frage "ALLE diese Daten loeschen?") {
        $vorhanden | ForEach-Object { Entferne $_ }
    }
}

# Autostart-Eintrag
$run = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$approved = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run'
if (Get-ItemProperty -Path $run -Name 'Toolless' -ErrorAction SilentlyContinue) {
    if (Frage "`nAutostart-Eintrag 'Toolless' entfernen?") {
        Remove-ItemProperty -Path $run -Name 'Toolless' -ErrorAction SilentlyContinue
        Remove-ItemProperty -Path $approved -Name 'Toolless' -ErrorAction SilentlyContinue
        Write-Host "  Autostart-Eintrag entfernt"
    }
}

Write-Host "`nFertig."
