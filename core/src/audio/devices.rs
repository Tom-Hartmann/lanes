//! Audio device and application enumeration, and device-change notifications.

use serde::Serialize;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, DEVICE_STATE, DEVICE_STATE_ACTIVE, EDataFlow, ERole, IAudioSessionControl2,
    IAudioSessionManager2, IMMDevice, IMMDeviceEnumerator, IMMNotificationClient, IMMNotificationClient_Impl,
    MMDeviceEnumerator, eCapture, eConsole, eRender,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, STGM_READ};
use windows::core::{Interface, PCWSTR, implement};

use crate::util::ComGuard;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct AudioDevices {
    pub outputs: Vec<AudioDevice>,
    pub inputs: Vec<AudioDevice>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioApp {
    pub pid: u32,
    pub exe: String,
    pub name: String,
    pub active: bool,
}

pub(crate) fn friendly_name(dev: &IMMDevice) -> Option<String> {
    // SAFETY: property store read of a string property; the PROPVARIANT is
    // cleared by its Drop impl.
    unsafe {
        let store = dev.OpenPropertyStore(STGM_READ).ok()?;
        let value = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
        let s = value.to_string();
        (!s.is_empty()).then_some(s)
    }
}

fn device_id(dev: &IMMDevice) -> Option<String> {
    // SAFETY: GetId returns a CoTaskMem string that PWSTR::to_string copies;
    // the allocation is freed right after.
    unsafe {
        let id = dev.GetId().ok()?;
        let s = id.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(id.0 as *const _));
        s
    }
}

/// Active output and input devices.
pub fn list() -> AudioDevices {
    let _com = ComGuard::mta();
    let mut out = AudioDevices::default();
    // SAFETY: COM initialised above; enumeration with valid indices.
    unsafe {
        let Ok(enumerator) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else {
            return out;
        };
        for (flow, list) in [(eRender, &mut out.outputs), (eCapture, &mut out.inputs)] {
            let default = enumerator.GetDefaultAudioEndpoint(flow, eConsole).ok().and_then(|d| device_id(&d));
            let Ok(coll) = enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE) else { continue };
            let n = coll.GetCount().unwrap_or(0);
            for i in 0..n {
                let Ok(dev) = coll.Item(i) else { continue };
                let Some(id) = device_id(&dev) else { continue };
                list.push(AudioDevice {
                    is_default: default.as_deref() == Some(id.as_str()),
                    name: friendly_name(&dev).unwrap_or_else(|| id.clone()),
                    id,
                });
            }
        }
    }
    out
}

/// Applications that currently have an audio session on any output device.
/// The UI merges this with its own list of exe names, so stopped apps can be chosen too.
pub fn apps() -> Vec<AudioApp> {
    let _com = ComGuard::mta();
    let mut out: Vec<AudioApp> = Vec::new();
    // SAFETY: COM initialised above; enumeration with valid indices.
    unsafe {
        let Ok(enumerator) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else {
            return out;
        };
        let Ok(coll) = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else { return out };
        for i in 0..coll.GetCount().unwrap_or(0) {
            let Ok(dev) = coll.Item(i) else { continue };
            let Ok(mgr) = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { continue };
            let Ok(sessions) = mgr.GetSessionEnumerator() else { continue };
            for j in 0..sessions.GetCount().unwrap_or(0) {
                let Ok(ctl) = sessions.GetSession(j) else { continue };
                let Ok(ctl2) = ctl.cast::<IAudioSessionControl2>() else { continue };
                let Ok(pid) = ctl2.GetProcessId() else { continue };
                if pid == 0 || pid == std::process::id() {
                    continue;
                }
                let active = ctl.GetState().is_ok_and(|s| s == AudioSessionStateActive);
                let Some(exe) = crate::capture::windows_list::process_exe(pid) else { continue };
                if let Some(existing) = out.iter_mut().find(|a| a.exe.eq_ignore_ascii_case(&exe)) {
                    existing.active |= active;
                    continue;
                }
                let name = exe.trim_end_matches(".exe").trim_end_matches(".EXE").to_string();
                out.push(AudioApp { pid, exe, name, active });
            }
        }
    }
    out.sort_by(|a, b| b.active.cmp(&a.active).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

/// Device-change events, delivered on a COM thread.
#[derive(Clone, Debug)]
pub enum DeviceEvent {
    DefaultChanged { render: bool },
    Changed,
}

#[implement(IMMNotificationClient)]
struct Notifier(Box<dyn Fn(DeviceEvent) + Send + Sync>);

impl IMMNotificationClient_Impl for Notifier_Impl {
    fn OnDeviceStateChanged(&self, _id: &PCWSTR, _state: DEVICE_STATE) -> windows::core::Result<()> {
        (self.0)(DeviceEvent::Changed);
        Ok(())
    }
    fn OnDeviceAdded(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        (self.0)(DeviceEvent::Changed);
        Ok(())
    }
    fn OnDeviceRemoved(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        (self.0)(DeviceEvent::Changed);
        Ok(())
    }
    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _id: &PCWSTR) -> windows::core::Result<()> {
        if role == eConsole {
            (self.0)(DeviceEvent::DefaultChanged { render: flow == eRender });
        }
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _id: &PCWSTR, _key: &PROPERTYKEY) -> windows::core::Result<()> {
        Ok(())
    }
}

/// Registered for device notifications until dropped.
pub struct DeviceWatcher {
    enumerator: IMMDeviceEnumerator,
    client: IMMNotificationClient,
    _com: ComGuard,
}

// SAFETY: MMDevice API objects are free-threaded (MTA); the watcher is only
// moved, then dropped on the engine thread.
unsafe impl Send for DeviceWatcher {}

impl DeviceWatcher {
    pub fn new(on_event: impl Fn(DeviceEvent) + Send + Sync + 'static) -> crate::Result<Self> {
        use crate::error::WinContext;
        let com = ComGuard::mta();
        // SAFETY: COM initialised; the client stays alive until unregistered in Drop.
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ctx("MMDeviceEnumerator")?;
            let client: IMMNotificationClient = Notifier(Box::new(on_event)).into();
            enumerator.RegisterEndpointNotificationCallback(&client).ctx("RegisterEndpointNotificationCallback")?;
            Ok(Self { enumerator, client, _com: com })
        }
    }
}

impl Drop for DeviceWatcher {
    fn drop(&mut self) {
        // SAFETY: the same client that was registered.
        let _ = unsafe { self.enumerator.UnregisterEndpointNotificationCallback(&self.client) };
    }
}
