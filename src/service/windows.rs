use super::{ServiceStatus, log_path};
use anyhow::{Result, anyhow};
use std::process::Command;

const TASK_NAME: &str = "isq";

fn generate_task_xml() -> Result<String> {
    let exe = std::env::current_exe()?;
    let exe_path = exe.to_string_lossy();
    let log = log_path()?;
    let log_path_str = log.to_string_lossy();

    // Get current username for the UserId field
    let username = std::env::var("USERNAME").unwrap_or_default();

    // Use cmd.exe wrapper to redirect stdout/stderr to log file
    // This matches macOS/Linux behavior where the service manager handles logging
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{username}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>3</Count>
    </RestartOnFailure>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>cmd.exe</Command>
      <Arguments>/c "{exe_path}" daemon run &gt;&gt; "{log_path}" 2&gt;&amp;1</Arguments>
    </Exec>
  </Actions>
</Task>
"#,
        username = username,
        exe_path = exe_path,
        log_path = log_path_str
    ))
}

fn run_powershell(script: &str) -> Result<std::process::Output> {
    Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .map_err(|e| anyhow!("Failed to run PowerShell: {}", e))
}

/// Parse PowerShell error output and return an actionable error message
fn parse_task_scheduler_error(stderr: &str, operation: &str) -> anyhow::Error {
    let stderr_lower = stderr.to_lowercase();

    if stderr_lower.contains("access is denied")
        || stderr_lower.contains("accessdenied")
        || stderr_lower.contains("0x80070005")
    {
        return anyhow!(
            "Access denied while trying to {} the scheduled task.\n\
             Try running as Administrator, or check Task Scheduler permissions.",
            operation
        );
    }

    if stderr_lower.contains("cannot create a file when that file already exists") {
        return anyhow!(
            "Task already exists with conflicting settings.\n\
             Run `isq daemon stop` then `isq daemon start` to reinstall."
        );
    }

    if stderr_lower.contains("the system cannot find the file specified") {
        return anyhow!(
            "Task Scheduler could not find the isq executable.\n\
             Ensure isq is installed in a permanent location."
        );
    }

    // Default: return the raw error with context
    anyhow!("Failed to {} scheduled task: {}", operation, stderr.trim())
}

fn is_installed() -> Result<bool> {
    let script = format!(
        "Get-ScheduledTask -TaskName '{}' -ErrorAction SilentlyContinue",
        TASK_NAME
    );
    let output = run_powershell(&script)?;
    Ok(output.status.success() && !output.stdout.is_empty())
}

fn is_running() -> Result<bool> {
    let script = format!(
        "(Get-ScheduledTask -TaskName '{}' -ErrorAction SilentlyContinue).State -eq 'Running'",
        TASK_NAME
    );
    let output = run_powershell(&script)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.trim().eq_ignore_ascii_case("true"))
}

pub fn install() -> Result<()> {
    let xml = generate_task_xml()?;

    // Write XML to temp file and register task
    let script = format!(
        r#"
$xml = @'
{xml}
'@
$tempFile = [System.IO.Path]::GetTempFileName()
$xml | Out-File -FilePath $tempFile -Encoding Unicode
try {{
    Register-ScheduledTask -TaskName '{task}' -Xml (Get-Content $tempFile -Raw) -Force
}} finally {{
    Remove-Item $tempFile -Force
}}
"#,
        xml = xml,
        task = TASK_NAME
    );

    let output = run_powershell(&script)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(parse_task_scheduler_error(&stderr, "register"));
    }

    // Start the task immediately after install
    start()?;

    Ok(())
}

pub fn uninstall() -> Result<()> {
    if !is_installed()? {
        return Ok(());
    }

    let script = format!(
        "Unregister-ScheduledTask -TaskName '{}' -Confirm:$false",
        TASK_NAME
    );
    let output = run_powershell(&script)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(parse_task_scheduler_error(&stderr, "unregister"));
    }

    Ok(())
}

pub fn start() -> Result<()> {
    if !is_installed()? {
        install()?;
        return Ok(());
    }

    if is_running()? {
        return Ok(());
    }

    let script = format!("Start-ScheduledTask -TaskName '{}'", TASK_NAME);
    let output = run_powershell(&script)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(parse_task_scheduler_error(&stderr, "start"));
    }

    Ok(())
}

pub fn stop() -> Result<()> {
    if !is_running()? {
        return Ok(());
    }

    let script = format!("Stop-ScheduledTask -TaskName '{}'", TASK_NAME);
    let output = run_powershell(&script)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(parse_task_scheduler_error(&stderr, "stop"));
    }

    Ok(())
}

pub fn status() -> Result<ServiceStatus> {
    let installed = is_installed()?;

    if !installed {
        return Ok(ServiceStatus {
            installed: false,
            running: false,
            pid: None,
        });
    }

    let running = is_running()?;

    // Get PID if running - find isq.exe process started by Task Scheduler
    let pid = if running {
        let script = r#"
Get-CimInstance Win32_Process | Where-Object {
    $_.Name -eq 'isq.exe' -and $_.CommandLine -like '*daemon run*'
} | Select-Object -First 1 -ExpandProperty ProcessId
"#;
        let output = run_powershell(script)?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.trim().parse::<u32>().ok()
    } else {
        None
    };

    Ok(ServiceStatus {
        installed: true,
        running,
        pid,
    })
}
