import { parseSystemMetrics } from './parser.js'
import type { SystemMetricsCollector, SystemMetricsExecutor } from './types.js'

export const windowsCollector: SystemMetricsCollector = {
  platform: 'windows',
  async collect(executor) {
    const raw = await runPowerShellMetricsScript(executor)
    return parseSystemMetrics(raw, 'windows')
  }
}

async function runPowerShellMetricsScript(executor: SystemMetricsExecutor) {
  const encoded = Buffer.from(buildWindowsMetricsScript(), 'utf16le').toString('base64')
  const commands = [
    `powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand ${encoded}`,
    `pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand ${encoded}`
  ]
  let lastError: unknown

  for (const command of commands) {
    try {
      return await executor.exec(command, { allowNonZeroWithStdout: true })
    } catch (error) {
      lastError = error
    }
  }

  throw lastError instanceof Error ? lastError : new Error(String(lastError))
}

function buildWindowsMetricsScript() {
  return `
$ErrorActionPreference = "SilentlyContinue"

function Write-Metric([string] $Name, [object] $Value) {
  if ($null -eq $Value) { $Value = "" }
  Write-Output ("__" + $Name + "__" + [string] $Value)
}

function Format-Bytes([double] $Bytes) {
  if ($Bytes -ge 1TB) { return ("{0:N1} TB" -f ($Bytes / 1TB)) }
  if ($Bytes -ge 1GB) { return ("{0:N1} GB" -f ($Bytes / 1GB)) }
  if ($Bytes -ge 1MB) { return ("{0:N1} MB" -f ($Bytes / 1MB)) }
  if ($Bytes -ge 1KB) { return ("{0:N0} KB" -f ($Bytes / 1KB)) }
  return ("{0:N0} B" -f $Bytes)
}

$os = Get-CimInstance Win32_OperatingSystem
$processors = @(Get-CimInstance Win32_Processor)
$cpuPercent = [int] (($processors | Measure-Object -Property LoadPercentage -Average).Average)
if ($cpuPercent -lt 0) { $cpuPercent = 0 }
if ($cpuPercent -gt 100) { $cpuPercent = 100 }
$idlePercent = [math]::Max(0, 100 - $cpuPercent)
$uptimeSeconds = 0
if ($os.LastBootUpTime) {
  $uptimeSeconds = [int] ((Get-Date) - $os.LastBootUpTime).TotalSeconds
}

$memoryTotalBytes = [double] $os.TotalVisibleMemorySize * 1024
$memoryAvailableBytes = [double] $os.FreePhysicalMemory * 1024
$memoryUsedBytes = [math]::Max(0, $memoryTotalBytes - $memoryAvailableBytes)
$memoryPercent = if ($memoryTotalBytes -gt 0) { [int] (($memoryUsedBytes * 100) / $memoryTotalBytes) } else { 0 }

$pageFiles = @(Get-CimInstance Win32_PageFileUsage)
$swapTotalBytes = [double] (($pageFiles | Measure-Object -Property AllocatedBaseSize -Sum).Sum) * 1MB
$swapUsedBytes = [double] (($pageFiles | Measure-Object -Property CurrentUsage -Sum).Sum) * 1MB
$swapAvailableBytes = [math]::Max(0, $swapTotalBytes - $swapUsedBytes)
$swapPercent = if ($swapTotalBytes -gt 0) { [int] (($swapUsedBytes * 100) / $swapTotalBytes) } else { 0 }

$addresses = @(Get-NetIPAddress -AddressFamily IPv4 | Where-Object {
  $_.IPAddress -and $_.IPAddress -notlike "127.*" -and $_.IPAddress -notlike "169.254.*"
} | Select-Object -ExpandProperty IPAddress)
if (-not $addresses -or $addresses.Count -eq 0) {
  $addresses = @(Get-CimInstance Win32_NetworkAdapterConfiguration | Where-Object { $_.IPEnabled } | ForEach-Object { $_.IPAddress } | Where-Object { $_ -and $_ -match "^\\d+\\." -and $_ -notlike "127.*" })
}
$ip = $addresses | Select-Object -First 1

$netBefore = @{}
try {
  Get-NetAdapterStatistics | ForEach-Object {
    $netBefore[$_.Name] = [pscustomobject]@{ Rx = [double] $_.ReceivedBytes; Tx = [double] $_.SentBytes }
  }
} catch {}
Start-Sleep -Milliseconds 250
$netAfter = @()
try {
  $netAfter = @(Get-NetAdapterStatistics)
} catch {}
$sampleMs = 250
$rxRate = 0
$txRate = 0
$ifaces = @()
$ifaceRows = @()
foreach ($item in $netAfter) {
  $name = [string] $item.Name
  if ([string]::IsNullOrWhiteSpace($name)) { continue }
  $before = $netBefore[$name]
  $rxTotal = [double] $item.ReceivedBytes
  $txTotal = [double] $item.SentBytes
  $rowRxRate = 0
  $rowTxRate = 0
  if ($before) {
    $rowRxRate = [math]::Max(0, [int] (($rxTotal - $before.Rx) * 1000 / $sampleMs))
    $rowTxRate = [math]::Max(0, [int] (($txTotal - $before.Tx) * 1000 / $sampleMs))
  }
  $rxRate += $rowRxRate
  $txRate += $rowTxRate
  $ifaces += $name
  $ifaceRows += ("{0}|{1}|{2}|{3}|{4}" -f $name, [int64] $rxTotal, [int64] $txTotal, $rowRxRate, $rowTxRate)
}

$disks = @(Get-CimInstance Win32_LogicalDisk -Filter "DriveType=3")
$diskRows = @()
$fileSystemRows = @()
foreach ($disk in $disks) {
  $name = [string] $disk.DeviceID
  $size = [double] $disk.Size
  $free = [double] $disk.FreeSpace
  $used = [math]::Max(0, $size - $free)
  $percent = if ($size -gt 0) { [int] (($used * 100) / $size) } else { 0 }
  $diskRows += ("{0}|{1}/{2}" -f $name, (Format-Bytes $free), (Format-Bytes $size))
  $fileSystemRows += ("{0}|{1}|{2}|{3}%|{4}|{5}" -f $name, (Format-Bytes $size), (Format-Bytes $used), $percent, (Format-Bytes $free), $name)
}

$cpuRows = @()
foreach ($processor in $processors) {
  $cpuRows += ("{0}|{1}|{2}|-|-" -f ([string] $processor.Name).Trim(), [int] $processor.NumberOfCores, [string] $processor.MaxClockSpeed)
}

$gpuRows = @()
try {
  Get-CimInstance Win32_VideoController | ForEach-Object {
    $memory = if ($_.AdapterRAM) { Format-Bytes ([double] $_.AdapterRAM) } else { "-" }
    $gpuRows += ("{0}|{1}|{2}|{3}" -f ([string] $_.Name).Trim(), ([string] $_.AdapterCompatibility).Trim(), ([string] $_.DriverVersion).Trim(), $memory)
  }
} catch {}

$processRows = @()
try {
  Get-Process | Sort-Object -Property WorkingSet64 -Descending | Select-Object -First 80 | ForEach-Object {
    $elapsed = 0
    if ($_.StartTime) {
      $elapsed = [int] ((Get-Date) - $_.StartTime).TotalSeconds
    }
    $cpu = if ($_.CPU) { [double] $_.CPU } else { 0 }
    $processRows += ("{0:N1}M|{1:N1}|{2}|{3}" -f ($_.WorkingSet64 / 1MB), $cpu, $elapsed, $_.ProcessName)
  }
} catch {}

Write-Metric "PLATFORM" "windows"
Write-Metric "OS" $os.Caption
Write-Metric "KERNEL_NAME" "Windows"
Write-Metric "KERNEL_VERSION" $os.Version
Write-Metric "ARCH" $os.OSArchitecture
Write-Metric "HOSTNAME" $env:COMPUTERNAME
Write-Metric "IP" $ip
Write-Metric "UPTIME" ""
Write-Metric "UPTIME_SECONDS" $uptimeSeconds
Write-Metric "LOAD" "-"
Write-Metric "CPU" $cpuPercent
Write-Metric "CPU_USAGE" ("0|{0}|0|{1}|0|0|0|0" -f $cpuPercent, $idlePercent)
Write-Metric "MEM" ("{0}|{1}|{2}|0|0|0" -f [int]($memoryUsedBytes / 1MB), [int]($memoryTotalBytes / 1MB), $memoryPercent)
Write-Metric "MEM_BYTES" ("{0}|{1}|{2}|{3}|0|0|0" -f [int64]$memoryUsedBytes, [int64]$memoryTotalBytes, [int64]$memoryAvailableBytes, $memoryPercent)
Write-Metric "SWAP" ("{0}|{1}|{2}" -f [int]($swapUsedBytes / 1MB), [int]($swapTotalBytes / 1MB), $swapPercent)
Write-Metric "SWAP_BYTES" ("{0}|{1}|{2}|{3}" -f [int64]$swapUsedBytes, [int64]$swapTotalBytes, [int64]$swapAvailableBytes, $swapPercent)
Write-Output "__CPUINFO_START__"
$cpuRows | ForEach-Object { Write-Output $_ }
Write-Output "__CPUINFO_END__"
Write-Output "__GPUINFO_START__"
$gpuRows | ForEach-Object { Write-Output $_ }
Write-Output "__GPUINFO_END__"
Write-Metric "IFACES" ($ifaces -join ",")
Write-Metric "ACTIVE_IFACE" ($ifaces | Select-Object -First 1)
Write-Metric "RATES" ("{0}|{1}" -f $rxRate, $txRate)
Write-Output "__IFACE_RATES_START__"
$ifaceRows | ForEach-Object { Write-Output $_ }
Write-Output "__IFACE_RATES_END__"
Write-Output "__DISK_START__"
$diskRows | ForEach-Object { Write-Output $_ }
Write-Output "__DISK_END__"
Write-Output "__FILESYSTEMS_START__"
$fileSystemRows | ForEach-Object { Write-Output $_ }
Write-Output "__FILESYSTEMS_END__"
Write-Output "__PROCS_START__"
$processRows | ForEach-Object { Write-Output $_ }
Write-Output "__PROCS_END__"
`
}
