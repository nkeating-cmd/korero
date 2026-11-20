param([Parameter(Mandatory)][string]$Uat, [switch]$Proxy)
# Kōrero local AI UAT: the test build, portable (its own Data folder), with
# LOCALAPPDATA pointed at an empty folder and Ollama off PATH, so it sees a
# computer with no Ollama installed and shares no webview data with the
# installed app. The webview listens for a test driver on 127.0.0.1:9333.
New-Item -ItemType Directory -Force "$Uat\lad" | Out-Null
$env:LOCALAPPDATA = "$Uat\lad"
$env:PATH = (($env:PATH -split ';') | Where-Object { $_ -and $_ -notmatch 'Ollama' }) -join ';'
Get-ChildItem Env: | Where-Object { $_.Name -like 'OLLAMA_*' } | ForEach-Object { Remove-Item "Env:$($_.Name)" }
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9333 --remote-debugging-address=127.0.0.1'
if ($Proxy) { $env:HTTP_PROXY = 'http://127.0.0.1:9'; $env:HTTPS_PROXY = 'http://127.0.0.1:9' }
$p = Start-Process "$Uat\app\korero.exe" -PassThru
"LAUNCHED pid=$($p.Id) proxy=$Proxy $(Get-Date -Format o)" | Out-File "$Uat\launch.log" -Append -Encoding utf8
