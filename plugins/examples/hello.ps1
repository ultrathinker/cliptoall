# @plugin: Hello PowerShell
# @description: Minimal example plugin that prints a greeting
# @version: 1.0.0
# @mode: oneshot
# @key: H
# @label: Say Hello

<#
.SYNOPSIS
ClipToAll example plugin (PowerShell).

Mirrors the oneshot template in src/lib/plugin-templates.ts and also speaks
the line-delimited JSON stdin/stdout protocol from plugins/PLUGIN-PROTOCOL.md
so it can be enabled as a plugin.

Usage:
  powershell -NoProfile -File hello.ps1                               Print help
  powershell -NoProfile -File hello.ps1 --call <json>                 Oneshot
  powershell -NoProfile -File hello.ps1 --call '@file.json'           Oneshot (file)
  powershell -NoProfile -File hello.ps1 --daemon                      Plugin mode
#>

param(
    [string]$call,
    [switch]$daemon
)

$Name = "Hello PowerShell"
$Version = "1.0.0"
$Description = "Minimal example plugin that prints a greeting"

function Emit-Line {
    param([Parameter(Mandatory)][hashtable]$Object, [int]$Depth = 3)
    # -Compress is mandatory: multi-line output breaks the line-delimited protocol.
    $Object | ConvertTo-Json -Compress -Depth $Depth | Write-Output
}

function Result {
    param([string]$Message, [string]$Status = "ok")
    Emit-Line @{ type = "result"; status = $Status; message = $Message } -Depth 2
}

function Run-Daemon {
    # Exactly one hello line first, then one result line per stdin command.
    Emit-Line @{
        type = "hello"
        name = $Name
        version = $Version
        description = $Description
        instruction = "Press the assigned shortcut key to print a hello message."
        settings_description = ""
        settings_format = ""
        functions = @(
            @{ id = "hello"; label = "Say Hello"; default_key = "H" }
        )
    } -Depth 4

    while ($line = [Console]::In.ReadLine()) {
        if (-not $line.Trim()) { continue }
        try {
            $cmd = $line | ConvertFrom-Json
        } catch { continue }
        if ($cmd.type -eq "shutdown") { break }
        if ($cmd.type -eq "call") {
            Result "Hello from $Name! (function=$($cmd.function))"
        }
    }
}

function Print-Help {
    Write-Output "$Name v$Version"
    Write-Output $Description
    Write-Output ""
    Write-Output "Usage:"
    Write-Output "  powershell -NoProfile -File hello.ps1 --daemon       Run as ClipToAll plugin"
    Write-Output "  powershell -NoProfile -File hello.ps1 --call <json>  Oneshot: print a result line, exit"
    Write-Output "  powershell -NoProfile -File hello.ps1 --call @file   Oneshot: read call JSON from a file"
    Write-Output "  powershell -NoProfile -File hello.ps1 -Help          Show this help"
}

if ($daemon) {
    Run-Daemon
}
elseif ($call) {
    # $call is inline JSON or @<file> (ClipToAll passes @<file>).
    $callJson = if ($call.StartsWith("@")) { Get-Content -Raw -LiteralPath $call.Substring(1) } else { $call }
    $data = $callJson | ConvertFrom-Json
    $func = $data.function
    Result "Hello from $Name! (function=$func)"
}
else {
    Print-Help
}
