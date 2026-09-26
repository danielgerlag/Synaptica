# Isolated Synaptica verification harness.
# Usage: powershell -NoProfile -File .grok/skills/verify-synaptica/scripts/verify.ps1 <launch|doctor|drive-query|cleanup>
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("launch", "doctor", "drive-query", "cleanup")]
    [string]$Command
)

$ErrorActionPreference = "Stop"
$Repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..\..")).Path
$RunDir = Join-Path $env:TEMP "synaptica-verify"
$StatePath = Join-Path $RunDir "state.json"
$Evidence = Join-Path $Repo ".grok\verify-evidence\query-graph"
$Listen = "127.0.0.1:19090"
$Metrics = "127.0.0.1:19091"
$HostUrl = "http://127.0.0.1:19090"

function Read-State {
    if (-not (Test-Path $StatePath)) {
        throw "No verification instance. Run launch first."
    }
    return Get-Content $StatePath -Raw | ConvertFrom-Json
}

function Strip-Ansi([string]$Text) {
    if (-not $Text) { return "" }
    return [regex]::Replace($Text, "\x1b\[[0-9;]*m", "")
}

function Wait-Ready([string]$LogPath, [int]$TimeoutSec) {
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    $needle = "starting gRPC server addr=$Listen"
    while ((Get-Date) -lt $deadline) {
        if (Test-Path $LogPath) {
            $text = Strip-Ansi (Get-Content $LogPath -Raw -ErrorAction SilentlyContinue)
            if ($text.Contains($needle)) {
                return $text
            }
        }
        Start-Sleep -Milliseconds 400
    }
    throw "Server did not log '$needle' within ${TimeoutSec}s. See $LogPath"
}

switch ($Command) {
    "launch" {
        if (Test-Path $StatePath) {
            $existing = Get-Content $StatePath -Raw | ConvertFrom-Json
            $alive = Get-Process -Id $existing.pid -ErrorAction SilentlyContinue
            if ($alive) {
                throw "Verification server already running as pid $($existing.pid). Run cleanup before launching another."
            }
        }
        if (Test-Path $RunDir) { Remove-Item -Recurse -Force $RunDir }
        New-Item -ItemType Directory -Force -Path $RunDir | Out-Null
        New-Item -ItemType Directory -Force -Path $Evidence | Out-Null
        $dataDir = (Join-Path $RunDir "data").Replace("\", "/")
        $configPath = Join-Path $RunDir "synaptica.toml"
        $configText = @"
listen_addr = "$Listen"
data_dir = "$dataDir"
default_graph = "default"
log_level = "info"
metrics_enabled = true
metrics_addr = "$Metrics"
"@
        [System.IO.File]::WriteAllText($configPath, $configText.Replace("`r`n", "`n"))

        $server = Join-Path $Repo "target\debug\synaptica-server.exe"
        if (-not (Test-Path $server)) {
            Push-Location $Repo
            try { cargo build -p synaptica-server --offline }
            finally { Pop-Location }
        }
        if (-not (Test-Path $server)) {
            throw "Missing $server after cargo build."
        }

        $log = Join-Path $RunDir "server.log"
        $stderr = Join-Path $RunDir "server.err"
        $launcher = Join-Path $RunDir "run-server.cmd"
        # The wrapper redirects stdout, which is where tracing writes the ready line.
        @"
@echo off
"$server" --config "$configPath" > "$log" 2> "$stderr"
"@ | Set-Content -Path $launcher -Encoding ascii
        # A scheduled task starts outside this shell's process tree, so the
        # server stays up after launch returns.
        $task = "synaptica-verify"
        schtasks /Create /F /TN $task /SC ONCE /ST 00:00 /TR "`"$launcher`"" | Out-Null
        schtasks /Run /TN $task | Out-Null
        schtasks /Delete /F /TN $task | Out-Null
        try {
            $ready = Wait-Ready $log 40
        } catch {
            throw
        }
        $owner = Get-Process -Name synaptica-server -ErrorAction SilentlyContinue |
            Where-Object { $_.StartTime -gt (Get-Date).AddMinutes(-2) } |
            Sort-Object StartTime -Descending |
            Select-Object -First 1
        if (-not $owner) { throw "Server logged ready but synaptica-server.exe is not running." }
        $serverPid = [int]$owner.Id
        @{
            pid = $serverPid
            data_dir = $dataDir
            config = $configPath
            log = $log
            listen = $Listen
            metrics = $Metrics
            host = $HostUrl
        } | ConvertTo-Json | Set-Content $StatePath -Encoding utf8
        $readyLines = ($ready -split "`n" | Where-Object { $_ -match "node started|starting gRPC server|metrics endpoint started" }) -join "`n"
        Set-Content -Path (Join-Path $Evidence "server-ready.log") -Value $readyLines -Encoding utf8
        Write-Output "ready pid=$serverPid host=$HostUrl metrics=http://$Metrics data=$dataDir"
    }
    "doctor" {
        $state = Read-State
        $proc = Get-Process -Id $state.pid -ErrorAction SilentlyContinue
        if (-not $proc) { throw "pid $($state.pid) is not running." }
        if ($proc.ProcessName -ne "synaptica-server") {
            throw "pid $($state.pid) is $($proc.ProcessName), not synaptica-server."
        }
        $log = Strip-Ansi (Get-Content $state.log -Raw)
        if ($log -notmatch [regex]::Escape("data_dir=$($state.data_dir)")) {
            throw "Log data_dir does not match this run ($($state.data_dir))."
        }
        if ($log -notmatch [regex]::Escape("starting gRPC server addr=$($state.listen)")) {
            throw "Log is not the verification listener $($state.listen)."
        }
        $tcp = New-Object System.Net.Sockets.TcpClient
        try {
            $tcp.Connect("127.0.0.1", 19090)
            if (-not $tcp.Connected) { throw "127.0.0.1:19090 is not accepting connections." }
        } catch {
            throw "127.0.0.1:19090 is not accepting connections."
        } finally {
            $tcp.Close()
        }
        $metrics = (Invoke-WebRequest -UseBasicParsing "http://$($state.metrics)/").Content
        if ($metrics -notmatch "synaptica_nodes_total") { throw "Metrics endpoint did not report synaptica_nodes_total." }
        Write-Output "doctor ok pid=$($state.pid) listen=$($state.listen) data=$($state.data_dir)"
    }
    "drive-query" {
        $state = Read-State
        & $PSCommandPath doctor | Out-Host
        $cli = Join-Path $Repo "target\debug\synaptica-cli.exe"
        if (-not (Test-Path $cli)) {
            Push-Location $Repo
            try { cargo build -p synaptica-cli --offline }
            finally { Pop-Location }
        }
        $queries = @"
INSERT (:Person {name: 'Alice', age: 30})
INSERT (:Person {name: 'Bob', age: 25})
MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Bob'}) \
INSERT (a)-[:KNOWS {since: 2020}]->(b)
MATCH (a:Person)-[r:KNOWS]->(b:Person) RETURN a.name, b.name, r.since
MATCH (n:Person) WHERE n.age > 20 RETURN n.name, n.age ORDER BY n.age DESC
"@
        $queryFile = Join-Path $RunDir "query-graph.gql"
        Set-Content -Path $queryFile -Value $queries -Encoding utf8
        $transcript = Join-Path $Evidence "transcript.txt"
        $prevProfile = $env:USERPROFILE
        try {
            $env:USERPROFILE = $RunDir
            $output = Get-Content $queryFile -Raw | & $cli --host $state.host 2>&1 | Out-String
        } finally {
            $env:USERPROFILE = $prevProfile
        }
        Set-Content -Path $transcript -Value $output -Encoding utf8
        $metrics = (Invoke-WebRequest -UseBasicParsing "http://$($state.metrics)/").Content
        $metricLines = ($metrics -split "`n" | Where-Object { $_ -match "^synaptica_nodes_total |^synaptica_edges_total " }) -join "`n"
        Set-Content -Path (Join-Path $Evidence "metrics.txt") -Value $metricLines -Encoding utf8
        $needles = @("Nodes created: 1", "Edges created: 1", "Alice", "Bob", "2020", "1 row(s) returned", "2 row(s) returned")
        foreach ($needle in $needles) {
            if ($output -notlike "*$needle*") { throw "Transcript missing '$needle'." }
        }
        if ($metricLines -notmatch "synaptica_nodes_total 2") { throw "Expected synaptica_nodes_total 2." }
        if ($metricLines -notmatch "synaptica_edges_total 1") { throw "Expected synaptica_edges_total 1." }
        Write-Output "drive-query ok transcript=$transcript"
        Write-Output $output
    }
    "cleanup" {
        if (Test-Path $StatePath) {
            $state = Get-Content $StatePath -Raw | ConvertFrom-Json
            $proc = Get-Process -Id $state.pid -ErrorAction SilentlyContinue
            if ($proc -and $proc.ProcessName -eq "synaptica-server") {
                Stop-Process -Id $state.pid -Force
            }
        }
        if (Test-Path $RunDir) {
            $removed = $false
            for ($try = 0; $try -lt 6; $try++) {
                try {
                    Remove-Item -Recurse -Force $RunDir -ErrorAction Stop
                    $removed = $true
                    break
                } catch {
                    Start-Sleep -Milliseconds 400
                }
            }
            if (-not $removed) { throw "Could not remove $RunDir. A process still has it open." }
        }
        Write-Output "cleanup ok evidence=$Evidence"
    }
}
