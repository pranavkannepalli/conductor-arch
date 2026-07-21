$ErrorActionPreference = "Stop"

$repo_root = Split-Path -Parent $PSScriptRoot
$bash = "C:\msys64\usr\bin\bash.exe"
$dev_env = "scripts/dev-instance-env.sh"
$child_launcher = "scripts/dev-windows-child.sh"

function Stop-DevProcessTree {
    param([System.Diagnostics.Process]$RootProcess)

    if ($null -eq $RootProcess) {
        return
    }

    $processes = @(Get-CimInstance Win32_Process)
    $children_by_parent = @{}
    foreach ($process in $processes) {
        $parent_id = [int]$process.ParentProcessId
        if (-not $children_by_parent.ContainsKey($parent_id)) {
            $children_by_parent[$parent_id] = [System.Collections.Generic.List[int]]::new()
        }
        $children_by_parent[$parent_id].Add([int]$process.ProcessId)
    }

    $owned_ids = [System.Collections.Generic.List[int]]::new()
    $pending = [System.Collections.Generic.Stack[int]]::new()
    $pending.Push($RootProcess.Id)
    while ($pending.Count -gt 0) {
        $process_id = $pending.Pop()
        $owned_ids.Add($process_id)
        if ($children_by_parent.ContainsKey($process_id)) {
            foreach ($child_id in $children_by_parent[$process_id]) {
                $pending.Push($child_id)
            }
        }
    }

    for ($index = $owned_ids.Count - 1; $index -ge 0; $index--) {
        Stop-Process -Id $owned_ids[$index] -Force -ErrorAction SilentlyContinue
    }
}

Push-Location $repo_root
$archcar = $null
$gtk_watch = $null
$exit_code = 0
try {
    & $bash $dev_env cargo build --workspace
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }

    $archcar = Start-Process -FilePath $bash -ArgumentList @(
        $child_launcher, "archcar"
    ) -WorkingDirectory $repo_root -NoNewWindow -PassThru
    $gtk_watch = Start-Process -FilePath $bash -ArgumentList @(
        $child_launcher, "gtk-watch"
    ) -WorkingDirectory $repo_root -NoNewWindow -PassThru

    while ($true) {
        $archcar.Refresh()
        $gtk_watch.Refresh()
        if ($archcar.HasExited) {
            $exit_code = $archcar.ExitCode
            break
        }
        if ($gtk_watch.HasExited) {
            $exit_code = $gtk_watch.ExitCode
            break
        }
        Start-Sleep -Milliseconds 200
    }
}
finally {
    Stop-DevProcessTree -RootProcess $archcar
    Stop-DevProcessTree -RootProcess $gtk_watch
    Pop-Location
}

exit $exit_code
