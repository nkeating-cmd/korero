param([Parameter(Mandatory)][string]$Uat, [switch]$Undo)
# Stand in for a software-restriction policy: deny this user "execute" on
# Kōrero's runtime folder in the TEST copy only. -Undo removes the denial.
$d = "$Uat\app\Data\local-ai\runtime"
New-Item -ItemType Directory -Force $d | Out-Null
$who = "$env:USERDOMAIN\$env:USERNAME"
if ($Undo) { icacls $d /remove:d $who /T /C | Select-Object -Last 1 }
else { icacls $d /deny "${who}:(OI)(CI)(X)" | Select-Object -Last 1 }
