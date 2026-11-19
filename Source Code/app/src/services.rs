//! Windows services: list (one SCM call), start / stop / restart, start type and description on demand.
use crate::sys::*;
use windows::Win32::System::Services::*;
use windows::core::PCWSTR;

#[derive(Clone, Debug)]
pub struct Service {
    pub name: String,
    pub display: String,
    pub state: &'static str,
    pub pid: u32,
}

fn state_str(s: SERVICE_STATUS_CURRENT_STATE) -> &'static str {
    match s {
        SERVICE_RUNNING => "running",
        SERVICE_STOPPED => "stopped",
        SERVICE_START_PENDING => "starting",
        SERVICE_STOP_PENDING => "stopping",
        SERVICE_PAUSED => "paused",
        SERVICE_PAUSE_PENDING => "pausing",
        SERVICE_CONTINUE_PENDING => "resuming",
        _ => "unknown",
    }
}

pub fn list() -> Vec<Service> {
    let mut out = vec![];
    unsafe {
        let Ok(scm) = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ENUMERATE_SERVICE) else { return out };
        let mut needed = 0u32;
        let mut count = 0u32;
        let mut resume = 0u32;
        let mut buf: Vec<u8> = vec![0; 256 * 1024];
        loop {
            let r = EnumServicesStatusExW(scm, SC_ENUM_PROCESS_INFO, SERVICE_WIN32, SERVICE_STATE_ALL, Some(&mut buf), &mut needed, &mut count, Some(&mut resume), PCWSTR::null());
            let items = std::slice::from_raw_parts(buf.as_ptr() as *const ENUM_SERVICE_STATUS_PROCESSW, count as usize);
            for it in items {
                out.push(Service {
                    name: it.lpServiceName.to_string().unwrap_or_default(),
                    display: it.lpDisplayName.to_string().unwrap_or_default(),
                    state: state_str(it.ServiceStatusProcess.dwCurrentState),
                    pid: it.ServiceStatusProcess.dwProcessId,
                });
            }
            if r.is_ok() {
                break;
            }
            if needed as usize > buf.len() {
                buf.resize(needed as usize, 0);
            } else if count == 0 {
                break;
            }
        }
        let _ = CloseServiceHandle(scm);
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// start type ("auto", "auto-delayed" not separated, "manual", "disabled", "boot", "system"), binary path, description
pub fn details(name: &str) -> Option<(String, String, String)> {
    unsafe {
        let scm = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT).ok()?;
        let n = wide(name);
        let svc = OpenServiceW(scm, PCWSTR(n.as_ptr()), SERVICE_QUERY_CONFIG);
        let res = svc.ok().map(|svc| {
            let mut needed = 0u32;
            let _ = QueryServiceConfigW(svc, None, 0, &mut needed);
            let mut buf = vec![0u8; needed.max(64) as usize];
            let mut start = String::new();
            let mut bin = String::new();
            if QueryServiceConfigW(svc, Some(buf.as_mut_ptr() as *mut QUERY_SERVICE_CONFIGW), buf.len() as u32, &mut needed).is_ok() {
                let c = &*(buf.as_ptr() as *const QUERY_SERVICE_CONFIGW);
                start = match c.dwStartType {
                    SERVICE_AUTO_START => "auto",
                    SERVICE_DEMAND_START => "manual",
                    SERVICE_DISABLED => "disabled",
                    SERVICE_BOOT_START => "boot",
                    SERVICE_SYSTEM_START => "system",
                    _ => "unknown",
                }
                .into();
                bin = c.lpBinaryPathName.to_string().unwrap_or_default();
            }
            let mut desc = String::new();
            let _ = QueryServiceConfig2W(svc, SERVICE_CONFIG_DESCRIPTION, None, &mut needed);
            let mut b2 = vec![0u8; needed.max(16) as usize];
            if QueryServiceConfig2W(svc, SERVICE_CONFIG_DESCRIPTION, Some(&mut b2), &mut needed).is_ok() {
                let d = &*(b2.as_ptr() as *const SERVICE_DESCRIPTIONW);
                if !d.lpDescription.is_null() {
                    desc = d.lpDescription.to_string().unwrap_or_default();
                }
            }
            let _ = CloseServiceHandle(svc);
            (start, bin, desc)
        });
        let _ = CloseServiceHandle(scm);
        res
    }
}

pub fn control(name: &str, action: &str) -> Result<(), String> {
    unsafe {
        let scm = OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT).map_err(|e| e.message())?;
        let n = wide(name);
        let access = SERVICE_START | SERVICE_STOP | SERVICE_QUERY_STATUS;
        let svc = OpenServiceW(scm, PCWSTR(n.as_ptr()), access).map_err(|e| {
            let _ = CloseServiceHandle(scm);
            if e.code().0 as u32 == 0x80070005 { "access denied (run as administrator)".to_string() } else { e.message() }
        })?;
        let stop = |svc| -> Result<(), String> {
            let mut st = SERVICE_STATUS::default();
            ControlService(svc, SERVICE_CONTROL_STOP, &mut st).map_err(|e| e.message())?;
            for _ in 0..100 {
                if QueryServiceStatus(svc, &mut st).is_ok() && st.dwCurrentState == SERVICE_STOPPED {
                    return Ok(());
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err("timed out waiting for stop".into())
        };
        let r = match action {
            "start" => StartServiceW(svc, None).map_err(|e| e.message()),
            "stop" => stop(svc),
            "restart" => stop(svc).and_then(|_| StartServiceW(svc, None).map_err(|e| e.message())),
            _ => Err(format!("unknown action '{action}'")),
        };
        let _ = CloseServiceHandle(svc);
        let _ = CloseServiceHandle(scm);
        r
    }
}
