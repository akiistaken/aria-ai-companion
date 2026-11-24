# Samples already-running Aria processes; run with the app idle, talking, or hidden to compare.
# CPU is normalized across logical processors. GPU is the busiest engine per process, averaged
# over the sample; GPU counters may be unavailable on older Windows/driver versions.
param(
    [ValidateRange(1, 600)][int]$Seconds = 30,
    [ValidateRange(0, 60)][int]$WarmupSeconds = 20,
    [string[]]$ProcessName = @('aria', 'aria-avatar'),
    [int[]]$ProcessId = @(),
    [string]$JsonOut = ''
)

$ErrorActionPreference = 'Stop'
if ($WarmupSeconds) { Start-Sleep -Seconds $WarmupSeconds }
# Discover counters before the CPU baseline: counter initialization can take several seconds.
$gpuAvailable = $null -ne (Get-Counter -ListSet 'GPU Engine' -ErrorAction SilentlyContinue)
$processes = if ($ProcessId.Count) {
    @(Get-Process -Id $ProcessId -ErrorAction SilentlyContinue)
} else {
    @(Get-Process -Name $ProcessName -ErrorAction SilentlyContinue)
}
if (-not $processes.Count) { throw 'No matching running processes.' }

$samples = @{}
foreach ($process in $processes) {
    $samples[$process.Id] = @{
        Name = $process.ProcessName
        CpuStart = $process.TotalProcessorTime.TotalSeconds
        CpuEnd = $process.TotalProcessorTime.TotalSeconds
        Ram = [double]$process.WorkingSet64
        Private = [double]$process.PrivateMemorySize64
        Gpu = 0.0
        GpuTotal = 0.0
        Count = 0
        GpuCount = 0
        LastAt = 0.0
    }
}
$watch = [Diagnostics.Stopwatch]::StartNew()
for ($index = 0; $index -lt $Seconds; $index++) {
    $counters = $null
    if ($gpuAvailable) {
        $counters = Get-Counter '\GPU Engine(*)\Utilization Percentage' -SampleInterval 1 -MaxSamples 1 -ErrorAction SilentlyContinue
    } else {
        Start-Sleep -Seconds 1
    }
    foreach ($processIdValue in @($samples.Keys)) {
        $process = Get-Process -Id $processIdValue -ErrorAction SilentlyContinue
        if (-not $process) { continue }
        $sample = $samples[$processIdValue]
        $sample.CpuEnd = $process.TotalProcessorTime.TotalSeconds
        $sample.Ram += $process.WorkingSet64
        $sample.Private += $process.PrivateMemorySize64
        $sample.Count++
        $sample.LastAt = $watch.Elapsed.TotalSeconds
        if ($counters) {
            $busy = $counters.CounterSamples | Where-Object { $_.InstanceName -match "^pid_${processIdValue}_" } |
                Measure-Object -Property CookedValue -Maximum -Sum
            if ($null -ne $busy.Maximum) {
                $sample.Gpu += [Math]::Max(0, $busy.Maximum)
                $sample.GpuTotal += [Math]::Max(0, $busy.Sum)
                $sample.GpuCount++
            }
        }
    }
}
$rows = @(foreach ($processIdValue in $samples.Keys) {
    $sample = $samples[$processIdValue]
    [pscustomobject]@{
        Process = $sample.Name
        Id = $processIdValue
        Seconds = [Math]::Round($sample.LastAt, 1)
        CpuPercent = [Math]::Round(100 * ($sample.CpuEnd - $sample.CpuStart) / [Math]::Max(0.001, $sample.LastAt) / [Environment]::ProcessorCount, 2)
        WorkingSetMB = [Math]::Round($sample.Ram / ($sample.Count + 1) / 1MB, 1)
        PrivateMB = [Math]::Round($sample.Private / ($sample.Count + 1) / 1MB, 1)
        GpuPercent = if ($sample.GpuCount) { [Math]::Round($sample.Gpu / $sample.GpuCount, 2) } else { $null }
        GpuEngineTotalPercent = if ($sample.GpuCount) { [Math]::Round($sample.GpuTotal / $sample.GpuCount, 2) } else { $null }
    }
})
if ($JsonOut) { ConvertTo-Json -InputObject $rows | Set-Content -LiteralPath $JsonOut -Encoding UTF8 }
$rows | Sort-Object Process | Format-Table -AutoSize
