# Skill Switch context-warning hook (UserPromptSubmit). Windows PowerShell 5.1
# compatible: no ternary, no ??. Always exits 0; on any error, prints nothing.
# JSON on stdout with {"systemMessage": "..."} shows a message to the user without
# touching the model's context and without blocking the prompt.
try {
    $stdin = [Console]::In.ReadToEnd()
    $payload = $stdin | ConvertFrom-Json
    $transcriptPath = $payload.transcript_path
    $sessionId = $payload.session_id
    if (-not $transcriptPath -or -not (Test-Path -LiteralPath $transcriptPath)) { exit 0 }

    # Only the tail: transcripts can be several MB, and the current context only
    # depends on the last assistant line's usage numbers.
    $fs = [System.IO.File]::Open($transcriptPath, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
    $len = $fs.Length
    $start = [Math]::Max(0, $len - 524288)
    $fs.Seek($start, [System.IO.SeekOrigin]::Begin) | Out-Null
    $buf = New-Object byte[] ($len - $start)
    $fs.Read($buf, 0, $buf.Length) | Out-Null
    $fs.Close()
    $tail = [System.Text.Encoding]::UTF8.GetString($buf)

    # Find the last assistant line in the tail (transcripts are JSONL, one JSON object
    # per line) instead of regex-searching for "usage":{ anywhere in the tail: a
    # tool_result can echo transcript text containing that literal (this app reads
    # transcripts itself) and would otherwise match the wrong line.
    $lines = $tail -split "`n"
    $assistantLine = $null
    for ($i = $lines.Length - 1; $i -ge 0; $i--) {
        if ($lines[$i].Contains('"type":"assistant"')) { $assistantLine = $lines[$i]; break }
    }
    if (-not $assistantLine) { exit 0 }

    # Pull the three fields directly by name instead of parsing "usage":{...} as JSON:
    # real transcripts nest a "cache_creation" breakdown object inside usage, so a
    # naive "up to the first }" match grabs an incomplete, unparseable fragment.
    function Get-UsageField($text, $name) {
        $m = [regex]::Match($text, '"' + $name + '":(\d+)')
        if ($m.Success) { return [int64]$m.Groups[1].Value }
        return 0
    }

    $inputTok = Get-UsageField $assistantLine 'input_tokens'
    $cacheRead = Get-UsageField $assistantLine 'cache_read_input_tokens'
    $cacheCreate = Get-UsageField $assistantLine 'cache_creation_input_tokens'
    $context = $inputTok + $cacheRead + $cacheCreate

    # 850k instead of 400k: context is kept on purpose; auto-compact only starts at ~970k.
    if ($context -lt 850000) { exit 0 }

    # Don't nag every prompt: only warn again once context grew >= 100k since the
    # last warning for this session.
    $appDataDir = Join-Path $env:LOCALAPPDATA 'com.skillswitch.app'
    New-Item -ItemType Directory -Path $appDataDir -Force | Out-Null
    $statePath = Join-Path $appDataDir 'context-hook-state.json'

    # Several Claude Code sessions can hit this hook at once; a named mutex serializes
    # the read-modify-write so a concurrent run can't clobber another session's entry.
    # On timeout, skip the state update (no dedupe bookkeeping) but still warn below.
    $mutex = New-Object System.Threading.Mutex($false, 'Global\SkillSwitchContextHook')
    $gotLock = $mutex.WaitOne(2000)
    try {
        if ($gotLock) {
            $state = @{}
            if (Test-Path -LiteralPath $statePath) {
                $raw = Get-Content -LiteralPath $statePath -Raw -ErrorAction SilentlyContinue | ConvertFrom-Json -ErrorAction SilentlyContinue
                if ($raw) { $raw.PSObject.Properties | ForEach-Object { $state[$_.Name] = [int64]$_.Value } }
            }

            $lastWarned = 0
            if ($state.ContainsKey($sessionId)) { $lastWarned = $state[$sessionId] }
            if ($lastWarned -gt 0 -and ($context - $lastWarned) -lt 100000) { exit 0 }

            # Bounded but crude: once the file accumulates entries from many old
            # sessions, just keep the current one instead of growing forever.
            if ($state.Count -gt 200) { $state = @{} }
            $state[$sessionId] = $context
            ($state | ConvertTo-Json -Compress) | Set-Content -LiteralPath $statePath -Encoding UTF8
        }
    } finally {
        if ($gotLock) { $mutex.ReleaseMutex() }
        $mutex.Dispose()
    }

    $k = [Math]::Round($context / 1000)
    $msg = "Skill Switch: Kontext ≈ ${k}k Tokens — Auto-Compact ab ~970k; bei Bedarf /compact mit Fokus."
    @{ systemMessage = $msg } | ConvertTo-Json -Compress
} catch {
}
exit 0
