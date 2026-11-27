$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath($env:UNICA_TEST_HBK_ROOT)
$directories = '8.3.27.2074', 'workspace', 'state', 'cache', 'temp'
$watchers = @()
Add-Type -TypeDefinition @"
using System;
using System.Collections.Concurrent;
using System.IO;

public sealed class DiskEvent {
    public FileSystemEventArgs Change;
    public Exception Error;
}

public sealed class DiskObservation {
    public readonly BlockingCollection<DiskEvent> Events = new BlockingCollection<DiskEvent>();
    public void Attach(FileSystemWatcher watcher) {
        watcher.Created += OnChange;
        watcher.Changed += OnChange;
        watcher.Deleted += OnChange;
        watcher.Renamed += OnChange;
        watcher.Error += OnError;
    }
    private void OnChange(object sender, FileSystemEventArgs change) {
        Events.Add(new DiskEvent { Change = change });
    }
    private void OnError(object sender, ErrorEventArgs error) {
        Events.Add(new DiskEvent { Error = error.GetException() });
    }
}
"@
$observation = [DiskObservation]::new()

$controls = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
$existing = @{}
foreach ($phase in 'before', 'after') {
    $existing[$phase] = [Guid]::NewGuid().ToString('N')
    foreach ($name in $directories) {
        $file = [IO.Path]::Combine($root, $name, $existing[$phase])
        [void]$controls.Add($file)
        [IO.File]::WriteAllText($file, 'initial')
    }
}

# Each root has its own ordered event stream. The child starts after every
# start control and exits before the end controls. Seeing the last events in
# every stream establishes the end of the observation window for all five.
function Wait-ControlEvents($phase) {
    $pending = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $transient = [Guid]::NewGuid().ToString('N')
    foreach ($name in $directories) {
        $ephemeral = [IO.Path]::Combine($root, $name, $transient)
        $persistent = [IO.Path]::Combine($root, $name, $existing[$phase])
        [void]$controls.Add($ephemeral)
        [void]$pending.Add("Created/$ephemeral")
        [void]$pending.Add("Deleted/$ephemeral")
        [void]$pending.Add("Changed/$persistent")
        [IO.File]::AppendAllText($persistent, ' changed')
        [IO.File]::WriteAllText($ephemeral, 'transient')
        [IO.File]::Delete($ephemeral)
    }
    $deadline = [Diagnostics.Stopwatch]::StartNew()
    while ($pending.Count -gt 0) {
        if ($deadline.Elapsed.TotalSeconds -gt 20) {
            throw "missing $phase filesystem control events: $($pending -join ', ')"
        }
        $event = $null
        if (!$observation.Events.TryTake([ref]$event, 1000)) { continue }
        if ($null -ne $event.Error) { throw $event.Error }
        $change = $event.Change
        $kind = $change.ChangeType
        if ($pending.Remove("$kind/$($change.FullPath)")) { continue }
        # An instrumented write may notify Changed more than once.
        if ($kind -eq 'Changed' -and $controls.Contains($change.FullPath)) { continue }
        throw "unexpected filesystem effect: $kind $($change.FullPath)"
    }
}

try {
    foreach ($name in $directories) {
        $watcher = [IO.FileSystemWatcher]::new([IO.Path]::Combine($root, $name))
        $watchers += $watcher
        $watcher.IncludeSubdirectories = $true
        $watcher.NotifyFilter = [IO.NotifyFilters]'FileName, DirectoryName, LastWrite, Size'
        $observation.Attach($watcher)
        $watcher.EnableRaisingEvents = $true
    }
    Wait-ControlEvents 'before'
    $start = [Diagnostics.ProcessStartInfo]::new($env:UNICA_TEST_HBK_BINARY)
    $start.Arguments = "--exact $($env:UNICA_TEST_HBK_CASE) --nocapture"
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.WorkingDirectory = [IO.Path]::Combine($root, 'workspace')
    $start.EnvironmentVariables['UNICA_TEST_HBK_OBSERVED_ROOT'] = $root
    foreach ($key in 'TEMP', 'TMP', 'TMPDIR') {
        $start.EnvironmentVariables[$key] = [IO.Path]::Combine($root, 'temp')
    }
    $start.EnvironmentVariables['UNICA_PROVIDER_STATE_DIR'] = [IO.Path]::Combine($root, 'state')
    foreach ($key in 'UNICA_CACHE_DIR', 'UNICA_RUNTIME_CACHE_DIR', 'UNICA_ARTIFACT_CACHE') {
        $start.EnvironmentVariables[$key] = [IO.Path]::Combine($root, 'cache')
    }
    $child = [Diagnostics.Process]::Start($start)
    try {
        $read = $child.StandardOutput.ReadToEndAsync()
        if (!$child.WaitForExit(20000)) {
            $child.Kill()
            $child.WaitForExit()
            throw 'provider child did not finish'
        }
        $output = $read.GetAwaiter().GetResult()
        if ($child.ExitCode -ne 0 -or $output -notmatch '1 passed; 0 failed') {
            throw "provider child did not execute the selected test successfully: $output"
        }
    } finally {
        $child.Dispose()
    }
    Wait-ControlEvents 'after'
} finally {
    foreach ($watcher in $watchers) { $watcher.Dispose() }
}
