$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$speechDir = Join-Path $env:LOCALAPPDATA 'Lanes\speech'
$stageDir = Join-Path $speechDir ('setup-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $stageDir -Force | Out-Null
function Get-VerifiedFile($url, $path, $hash) {
    Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $path
    if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $hash) { throw 'Speech download checksum mismatch' }
}
try {
    $archive = Join-Path $stageDir 'speech.zip'
    Get-VerifiedFile 'https://github.com/ggml-org/whisper.cpp/releases/download/v1.9.1/whisper-bin-x64.zip' $archive '7d8be46ecd31828e1eb7a2ecdd0d6b314feafd82163038ab6092594b0a063539'
    Get-VerifiedFile 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin' (Join-Path $stageDir 'ggml-tiny.bin') 'be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21'
    Invoke-WebRequest -UseBasicParsing 'https://raw.githubusercontent.com/ggml-org/whisper.cpp/v1.9.1/LICENSE' -OutFile (Join-Path $speechDir 'whisper-cpp-LICENSE.txt')
    Invoke-WebRequest -UseBasicParsing 'https://raw.githubusercontent.com/openai/whisper/main/LICENSE' -OutFile (Join-Path $speechDir 'whisper-model-LICENSE.txt')
    Expand-Archive -LiteralPath $archive -DestinationPath (Join-Path $stageDir 'bin')
    $engine = Get-ChildItem -LiteralPath (Join-Path $stageDir 'bin') -Recurse -Filter whisper-cli.exe | Select-Object -First 1
    if (!$engine) { throw 'Speech archive has no whisper-cli.exe' }
    Get-ChildItem -LiteralPath $engine.Directory.FullName -File | Where-Object { $_.Extension -in '.exe', '.dll' } | Copy-Item -Destination $speechDir -Force
    Move-Item -LiteralPath (Join-Path $stageDir 'ggml-tiny.bin') -Destination (Join-Path $speechDir 'ggml-tiny.bin') -Force
    Write-Output 'Local caption engine installed.'
} finally {
    $resolvedStage = [IO.Path]::GetFullPath($stageDir)
    $resolvedRoot = [IO.Path]::GetFullPath($speechDir) + [IO.Path]::DirectorySeparatorChar
    if ($resolvedStage.StartsWith($resolvedRoot, [StringComparison]::OrdinalIgnoreCase) -and (Test-Path -LiteralPath $resolvedStage)) {
        Remove-Item -LiteralPath $resolvedStage -Recurse -Force
    }
}
