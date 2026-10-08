param(
    [Parameter(Mandatory = $true)]
    [string]$Source
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Write-Serial([string]$Line) {
    $port = New-Object System.IO.Ports.SerialPort 'COM1', 115200
    $port.Open()
    try {
        $port.WriteLine($Line)
    } finally {
        $port.Close()
    }
}

function Find-VirtioDriver([string]$Name, [string]$Inf) {
    foreach ($volume in Get-PSDrive -PSProvider FileSystem) {
        $candidate = Join-Path $volume.Root "$Name\w11\amd64\$Inf"
        if (Test-Path -LiteralPath $candidate) {
            return $candidate
        }
    }
    throw "virtio driver $Name was not found on any attached volume"
}

function Set-RegistryValue([string]$Path, [string]$Name, $Value, [string]$Type = 'DWord') {
    if (-not (Test-Path -LiteralPath $Path)) {
        New-Item -Path $Path -Force | Out-Null
    }
    New-ItemProperty -Path $Path -Name $Name -Value $Value -PropertyType $Type -Force | Out-Null
}

try {
    Write-Serial 'QOL_IMAGE_BUILD_PROVISIONING'

    foreach ($driver in @(@('viostor', 'viostor.inf'), @('vioserial', 'vioser.inf'))) {
        $inf = Find-VirtioDriver $driver[0] $driver[1]
        & pnputil.exe /add-driver $inf /install | Out-Null
        if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne 259 -and $LASTEXITCODE -ne 3010) {
            throw "pnputil failed for $inf with exit code $LASTEXITCODE"
        }
    }

    $programRoot = Join-Path $env:ProgramFiles 'qol'
    $dataRoot = Join-Path $env:ProgramData 'qol'
    New-Item -ItemType Directory -Force -Path $programRoot, $dataRoot | Out-Null
    $runner = Join-Path $programRoot 'qol-guest-runner.exe'
    Copy-Item -LiteralPath (Join-Path $Source 'qol-guest-runner.exe') -Destination $runner -Force
    Copy-Item -LiteralPath (Join-Path $Source 'image-identity.json') -Destination (Join-Path $dataRoot 'qol-dev-image.json') -Force

    $action = New-ScheduledTaskAction -Execute $runner -Argument 'run'
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User "$env:COMPUTERNAME\qol"
    $principal = New-ScheduledTaskPrincipal -UserId "$env:COMPUTERNAME\qol" -LogonType Interactive -RunLevel Highest
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) -MultipleInstances IgnoreNew
    Register-ScheduledTask -TaskName 'qol-guest-runner' -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null

    $winlogon = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'
    Set-RegistryValue $winlogon 'AutoAdminLogon' '1' 'String'
    Set-RegistryValue $winlogon 'DefaultUserName' 'qol' 'String'
    Set-RegistryValue $winlogon 'DefaultPassword' 'qol' 'String'
    Set-RegistryValue $winlogon 'DefaultDomainName' $env:COMPUTERNAME 'String'
    Remove-ItemProperty -Path $winlogon -Name 'AutoLogonCount' -ErrorAction SilentlyContinue

    Set-RegistryValue 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU' 'NoAutoUpdate' 1
    Set-RegistryValue 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\Personalization' 'NoLockScreen' 1
    Set-RegistryValue 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\CloudContent' 'DisableWindowsConsumerFeatures' 1
    Set-RegistryValue 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\OOBE' 'DisablePrivacyExperience' 1
    Set-RegistryValue 'HKCU:\Software\Microsoft\Windows\CurrentVersion\UserProfileEngagement' 'ScoobeSystemSettingEnabled' 0
    Set-RegistryValue 'HKCU:\Control Panel\Desktop' 'ScreenSaveActive' '0' 'String'

    & powercfg.exe /hibernate off
    & powercfg.exe /change standby-timeout-ac 0
    & powercfg.exe /change monitor-timeout-ac 0
    & powercfg.exe /change disk-timeout-ac 0

    Write-Serial 'QOL_IMAGE_BUILD_COMPLETE'
} catch {
    Write-Serial "QOL_IMAGE_BUILD_FAILED $($_.Exception.Message)"
} finally {
    & shutdown.exe /s /t 5 /f
}
